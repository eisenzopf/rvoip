//! Direct DTLS media-streaming availability check for rvoip 0.3.5.
//!
//! No media socket is opened: DTLS-SRTP construction must fail before a caller
//! can advertise or stream with the incomplete protocol implementation.

use rvoip_rtp_core::dtls::{create_connection, DtlsConfig};
use rvoip_rtp_core::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match create_connection(DtlsConfig::default()).await {
        Err(Error::UnsupportedFeature(message)) => {
            println!("Direct DTLS media streaming is unavailable in 0.3.5: {message}");
            Ok(())
        }
        Err(error) => Err(format!("unexpected DTLS construction error: {error}").into()),
        Ok(_) => Err("incomplete DTLS media construction unexpectedly succeeded".into()),
    }
}
