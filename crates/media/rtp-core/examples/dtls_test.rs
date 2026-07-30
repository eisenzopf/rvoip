//! DTLS-SRTP availability check for rvoip 0.3.5.
//!
//! The public constructor remains available, but the incomplete DTLS state
//! machine fails closed with a typed error before a connection is created.

use rvoip_rtp_core::dtls::{create_connection, DtlsConfig};
use rvoip_rtp_core::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match create_connection(DtlsConfig::default()).await {
        Err(Error::UnsupportedFeature(message)) => {
            println!("DTLS-SRTP is unavailable in 0.3.5: {message}");
            Ok(())
        }
        Err(error) => Err(format!("unexpected DTLS construction error: {error}").into()),
        Ok(_) => Err("incomplete DTLS-SRTP construction unexpectedly succeeded".into()),
    }
}
