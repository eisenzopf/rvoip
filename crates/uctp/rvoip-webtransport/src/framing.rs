//! UCTP envelope framing on concrete WebTransport streams.
//!
//! `web_transport_quinn` exposes both Tokio `AsyncRead`/`AsyncWrite` adapters
//! and native async methods. Chromium can deliver the WebTransport stream
//! header and the first application bytes in the same QUIC flight. Using the
//! native methods here retains the wake registration after the library has
//! consumed that header; wrapping the stream in `tokio_util::FramedRead`
//! could otherwise leave the first envelope parked until the authentication
//! deadline closed the peer.

use bytes::{Buf, BytesMut};
use rvoip_uctp::envelope::UctpEnvelope;

use crate::errors::{Result, UctpWtError};

const MAX_ENVELOPE_BYTES: usize = 1024 * 1024;

pub(crate) struct EnvelopeReader {
    stream: web_transport_quinn::RecvStream,
    buffered: BytesMut,
    eof: bool,
}

impl EnvelopeReader {
    pub(crate) fn new(stream: web_transport_quinn::RecvStream) -> Self {
        Self {
            stream,
            buffered: BytesMut::new(),
            eof: false,
        }
    }

    pub(crate) async fn next(&mut self) -> Result<Option<UctpEnvelope>> {
        loop {
            if self.buffered.len() >= 4 {
                let length = u32::from_be_bytes(
                    self.buffered[..4]
                        .try_into()
                        .expect("four-byte prefix was length checked"),
                ) as usize;
                if length > MAX_ENVELOPE_BYTES {
                    return Err(UctpWtError::Session("envelope frame too large".into()));
                }
                if self.buffered.len() >= 4 + length {
                    self.buffered.advance(4);
                    let payload = self.buffered.split_to(length);
                    let envelope: UctpEnvelope = serde_json::from_slice(&payload)
                        .map_err(|_| UctpWtError::Session("invalid envelope JSON".into()))?;
                    tracing::trace!(
                        envelope = envelope.msg_type.diagnostic_label(),
                        frame_bytes = length,
                        "decoded WebTransport envelope frame"
                    );
                    return Ok(Some(envelope));
                }
            }

            if self.eof {
                return if self.buffered.is_empty() {
                    Ok(None)
                } else {
                    Err(UctpWtError::Session("truncated envelope frame".into()))
                };
            }

            let mut chunk = [0_u8; 64 * 1024];
            match self.stream.read(&mut chunk).await {
                Ok(Some(size)) => {
                    tracing::trace!(size, "read WebTransport signaling bytes");
                    self.buffered.extend_from_slice(&chunk[..size]);
                }
                Ok(None) => self.eof = true,
                Err(_) => {
                    return Err(UctpWtError::Session(
                        "WebTransport stream read failed".into(),
                    ));
                }
            }
        }
    }
}

pub(crate) async fn write_envelope(
    stream: &mut web_transport_quinn::SendStream,
    envelope: &UctpEnvelope,
) -> Result<()> {
    let payload = serde_json::to_vec(envelope)
        .map_err(|_| UctpWtError::Session("envelope serialization failed".into()))?;
    if payload.len() > MAX_ENVELOPE_BYTES {
        return Err(UctpWtError::Session("envelope frame too large".into()));
    }
    let length = u32::try_from(payload.len())
        .map_err(|_| UctpWtError::Session("envelope frame too large".into()))?;
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(&payload);
    stream
        .write_all(&frame)
        .await
        .map_err(|_| UctpWtError::Session("WebTransport stream write failed".into()))?;
    tokio::io::AsyncWriteExt::flush(stream)
        .await
        .map_err(|_| UctpWtError::Session("WebTransport stream flush failed".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maximum_frame_size_fits_wire_prefix() {
        assert!(u32::try_from(MAX_ENVELOPE_BYTES).is_ok());
    }
}
