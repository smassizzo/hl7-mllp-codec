//! # A tokio codec implementation for the HL7 MLLP network protocol.
//!
//! HL7's MLLP is a simple, single-byte-text based protocol for framing HL7 messages over a TCP (or similar) transport.
//! The full specification is available at [the HL7 site](https://www.hl7.org/documentcenter/private/standards/v3/V3_TRMLLP_R2_R2019.zip) (Note that they place the standards behind a free membership/login form).
//!
//! This crate provides a [Codec](https://docs.rs/tokio/0.2.0-alpha.6/tokio/codec/index.html) implementation
//! that encodes/decodes MLLP frames from a Tokio stream, allowing simple programmatic access to the messages (both
//! primary and ack/nack).
//!
//! Tokio (and the rust async ecosystem) is currently in a state of flux, however there are two simple (not production ready!) examples in the source, of
//! both a publisher and a listener. NB. These examples just write output to the console, and this can seriously limit throughput.  If you want to run
//! some simple perf tests with the samples, ensure you minimise the amount of data written out.
//!
//! ## Example
//! This is a highly simplified example, lifted from the Examples included in source control.
//!
//! ### Publisher
//! ```no_run
//!use bytes::*;
//!use tokio_util::codec::Framed;
//!use tokio::net::TcpStream;
//!use futures::{SinkExt, StreamExt};
//!
//!use hl7_mllp_codec::MllpCodec;
//!
//!#[tokio::main]
//!async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!    // Open a TCP stream to the socket address.
//!    let stream = TcpStream::connect("127.0.0.1:8080").await?;
//!
//!    // Construct a MLLP transport using our codec
//!    let mut transport = Framed::new(stream, MllpCodec::new());
//!
//!    // Send some bytes wrapped in MLLP (Note: not a valid HL7 message)
//!    transport.send(BytesMut::from("Hello World")).await?; //because this is through the codec it gets wrapped in MLLP header/footer for us
//!
//!    if let Some(response) = transport.next().await {
//!        match response{
//!            Ok(msg) => println!("  Received response: {:?}", msg),
//!            Err(e) => println!("  ERROR: {:?}", e)
//!        }
//!    }
//!
//!    Ok(())
//!}

use bytes::buf::{Buf, BufMut};
use bytes::BytesMut;
use log::{debug, trace};
use tokio_util::codec::*;

/// See the [crate] documentation for better details.
#[derive(Default)]
pub struct MllpCodec {}

impl MllpCodec {
    const BLOCK_HEADER: u8 = 0x0B; //Vertical-Tab char, the marker for the start of a message
    const BLOCK_FOOTER: [u8; 2] = [0x1C, 0x0D]; //File-Separator char + CR, the marker for the end of a message

    /// Creates a new Codec instance, generally for use within a [Tokio Framed](https://docs.rs/tokio/0.2.0-alpha.6/tokio/codec/struct.Framed.html),
    /// but can be instantiated standalone for testing purposes etc.
    /// Example:
    /// ```
    /// use hl7_mllp_codec::MllpCodec;
    /// let mllp = MllpCodec::new();
    /// ```
    pub fn new() -> Self {
        MllpCodec {}
    }

    fn get_footer_position(src: &BytesMut) -> Option<usize> {
        let mut iter = src.iter().enumerate().peekable(); //search from start because we may have multiple messages on socket
        loop {
            let cur = iter.next();
            let next = iter.peek();

            match (cur, next) {
                (Some((i, cur_ele)), Some((_, next_ele))) => {
                    //both current and next ele are avail
                    if cur_ele == &MllpCodec::BLOCK_FOOTER[0]
                        && *next_ele == &MllpCodec::BLOCK_FOOTER[1]
                    {
                        trace!("MLLP: Found footer at index {}", i);
                        return Some(i);
                    }
                }
                (_, None) => {
                    trace!("MLLP: Unable to find footer...");
                    return None;
                }
                _ => {} //keep looping
            }
        }
    }
}

// Support encoding data as an MLLP Frame.
// This is used for both the primary HL7 message sent from a publisher, and also any ACK/NACK messages sent from a Listener.
impl Encoder<BytesMut> for MllpCodec {
    // For the moment all we do is return the underlying byte array, I'm not getting into message parsing here.
    type Error = std::io::Error; // Just to get rolling, custom error type later when needed.

    fn encode(&mut self, event: BytesMut, dst: &mut BytesMut) -> Result<(), Self::Error> {
        dst.reserve(event.len() + 3); //we need an extra 3 bytes of space on top of the message proper
        dst.put_u8(MllpCodec::BLOCK_HEADER); //header

        dst.put_slice(&event); //data

        dst.put_slice(&MllpCodec::BLOCK_FOOTER); //footer

        debug!("MLLP: Encoded value for send: '{:?}'", dst);
        Ok(())
    }
}

// Support decoding data from an MLLP Frame.
// This is used for receiving the primary HL7 message in a listener, and also decoding any ACK/NACK responses in a publisher.
impl Decoder for MllpCodec {
    type Item = BytesMut; // For the moment all we do is return the underlying byte array, I'm not getting into message parsing here.
    type Error = std::io::Error; // Just to get rolling, custom error type later when needed.

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        // We're lucky the MLLP is specced as synchronous, and requires an ACK before sending the
        // next message, so we don't have to worry about multiple messages in the buffer.

        // we DO have to ignore any bytes prior to the BLOCK_HEADER

        //do we have a BLOCK_HEADER?
        if let Some(start_offset) = src.iter().position(|b| *b == MllpCodec::BLOCK_HEADER) {
            //yes we do, do we have a footer?

            //trace!("MLLP: Found message header at index {}", start_offset);

            if let Some(end_offset) = MllpCodec::get_footer_position(src) {
                //TODO: Is it worth passing a slice of src so we don't search the header chars?
                //Most of the time the start_offset == 0, so not sure it's worth it.

                let mut result = src
                    .split_to(end_offset + 2) //get the footer bytes
                    .split_to(end_offset); // grab our data from the buffer, consuming (and losing) the footer
                result.advance(start_offset + 1); //move to start of data
                                                  //debug!("MLLP: Received message: {:?}", result);
                return Ok(Some(result));
            }
        }

        //trace!("MLLP: No clear header/footer available, waiting for more data.");
        Ok(None) // no message lurking in here yet
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////////////////////////////////////////
#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    fn wrap_for_mllp(s: &str) -> Bytes {
        Bytes::from(format!("\x0B{}\x1C\x0D", s))
    }

    fn wrap_for_mllp_mut(s: &str) -> BytesMut {
        BytesMut::from(format!("\x0B{}\x1C\x0D", s).as_str())
    }

    #[test]
    fn can_construct_without_error() {
        let _m = MllpCodec::new();
    }

    #[test]
    fn implements_default() {
        let _m = MllpCodec::default();
    }

    #[test]
    fn wraps_simple_data() {
        let data = BytesMut::from("abcd");
        let mut m = MllpCodec::new();

        let mut output_buf = BytesMut::with_capacity(64);

        match m.encode(data, &mut output_buf) {
            Ok(()) => {}
            _ => panic!("Non OK value returned from encode"),
        }
        let encoded_msg = output_buf.freeze();
        println!("Encoded: {:?}", encoded_msg);
        assert_eq!(encoded_msg, wrap_for_mllp("abcd"));
    }

    #[test]
    fn find_footer_location() {
        let data = wrap_for_mllp_mut("abcd"); //this gets the footer at position 5, as there's a leading byte added
        let result = MllpCodec::get_footer_position(&data);

        assert_eq!(result, Some(5));
    }

    #[test]
    fn missing_footer_detected() {
        let data = BytesMut::from("no footer");
        let result = MllpCodec::get_footer_position(&data);

        assert_eq!(result, None);
    }

    #[test]
    fn ensure_decoder_finds_simple_message() {
        let mut data = wrap_for_mllp_mut("abcd");
        let mut m = MllpCodec::new();

        let result = m.decode(&mut data);
        println!("simple message result: {:?}", result);
        match result {
            Ok(None) => panic!("Failed to find a simple message!"),
            Ok(Some(message)) => {
                assert_eq!(&message[..], b"abcd");
            }
            Err(err) => panic!("Error looking for simple message: {:?}", err),
        }
    }

    #[test]
    fn ensure_data_after_end_is_ignored() {
        // The MLLP spec states:
        // "the Source system shall not send new HL7 content until an acknowledgement for the previous HL7 Content has been received."
        // so we don't actually have to worry about this sort of data (ie more than 1 message per `receive`)

        let mut data = BytesMut::from("\x0BTest Data\x1C\x0DMore Data");
        let mut m = MllpCodec::new();

        let result = m.decode(&mut data);

        match result {
            Ok(Some(message)) => {
                assert_eq!(&message[..], b"Test Data");
            }
            _ => panic!("Failure for message with illegal trailing data"),
        }
    }

    #[test]
    fn ensure_no_data_is_left_on_the_stream() {
        // we get errors from the tokio stuff if we close a connection with data still sitting unread on the stream.
        // Ensure we remove it all as part of the decoder
        let mut data = BytesMut::from("\x0BTest Data\x1C\x0D");
        let mut m = MllpCodec::new();

        let _result = m.decode(&mut data);

        assert_eq!(
            data.len(),
            0,
            "Decoder left data sitting in the buffer after read!"
        );
    }

    #[test]
    fn ensure_buffer_is_reset_per_message() {
        let mut mllp = MllpCodec::new();

        let mut data1 = wrap_for_mllp_mut("Test Data");
        let mut data2 = wrap_for_mllp_mut("This is different");

        let result = mllp.decode(&mut data1);
        match result {
            Ok(Some(message)) => {
                assert_eq!(&message[..], b"Test Data");
            }
            _ => panic!("Error decoding second message"),
        }

        let result = mllp.decode(&mut data2);
        match result {
            Ok(Some(message)) => {
                assert_eq!(&message[..], b"This is different");
            }
            _ => panic!("Error decoding second message"),
        }
    }

    #[test]
    fn test_real_message() {
        let mut mllp = MllpCodec::new();
        let mut data = wrap_for_mllp_mut("MSH|^~\\&|ZIS|1^AHospital|||200405141144||¶ADT^A01|20041104082400|P|2.3|||AL|NE|||8859/15|¶EVN|A01|20041104082400.0000+0100|20041104082400¶PID||\"\"|10||Vries^Danny^D.^^de||19951202|M|||Rembrandlaan^7^Leiden^^7301TH^\"\"^^P||\"\"|\"\"||\"\"|||||||\"\"|\"\"¶PV1||I|3w^301^\"\"^01|S|||100^van den Berg^^A.S.^^\"\"^dr|\"\"||9||||H||||20041104082400.0000+0100");

        let result = mllp.decode(&mut data);
        match result {
            Ok(Some(message)) => {
                assert_eq!(message.len(), 338);
            }
            _ => panic!("Error decoding second message"),
        }
    }
    #[test]
    fn test_parsing_multiple_messages() {
        let mut mllp = MllpCodec::new();
        let mut data = wrap_for_mllp_mut("MSH|^~\\&|ZIS|1^AHospital|||200405141144||¶ADT^A01|20041104082400|P|2.3|||AL|NE|||8859/15|¶EVN|A01|20041104082400.0000+0100|20041104082400¶PID||\"\"|10||Vries^Danny^D.^^de||19951202|M|||Rembrandlaan^7^Leiden^^7301TH^\"\"^^P||\"\"|\"\"||\"\"|||||||\"\"|\"\"¶PV1||I|3w^301^\"\"^01|S|||100^van den Berg^^A.S.^^\"\"^dr|\"\"||9||||H||||20041104082400.0000+0100");
        let bytes = data.clone().iter().map(|s| s.to_owned()).collect::<Vec<u8>>();
        data.extend_from_slice(&bytes[..]);
        data.extend_from_slice(&bytes[..]);
        // Read first message
        let result = mllp.decode(&mut data);
        match result {
            Ok(Some(message)) => {
                // Ensure that a single message was parsed out correctly
                assert_eq!(message.len(), 338);
                // Check to make sure data is two messages and two encapsulations in size
                assert_eq!(data.len(), (message.len() * 2) + 6);
            }
            _ => assert!(false),
        }
        // Read second message
        let result = mllp.decode(&mut data);
        match result {
            Ok(Some(message)) => {
                // Ensure that a single message was parsed out correctly
                assert_eq!(message.len(), 338);
                // Check to make sure remaining data is the size of the message and encap
                assert_eq!(data.len(), message.len() + 3);
            }
            _ => assert!(false),
        }
    }
}
