//! Length-prefixed framing for UCTP envelopes on QUIC / WebTransport
//! bidi streams.
//!
//! 4-byte big-endian length prefix, max frame size 1 MiB. CONVERSATION_PROTOCOL.md
//! §4.1 / §4.2.

use bytes::Bytes;
use futures::{Sink, SinkExt, Stream, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};

use crate::envelope::UctpEnvelope;
use crate::errors::SubstrateError;

const MAX_FRAME: usize = 1024 * 1024;

/// Build the LengthDelimitedCodec used by both directions.
pub fn length_prefixed_codec() -> LengthDelimitedCodec {
    LengthDelimitedCodec::builder()
        .length_field_length(4)
        .big_endian()
        .max_frame_length(MAX_FRAME)
        .new_codec()
}

/// Wrap a read half so each item is a decoded [`UctpEnvelope`].
pub fn envelope_reader<R>(
    rx: R,
) -> impl Stream<Item = Result<UctpEnvelope, SubstrateError>>
where
    R: AsyncRead + Send + Unpin,
{
    FramedRead::new(rx, length_prefixed_codec()).map(|frame| -> Result<UctpEnvelope, SubstrateError> {
        let bytes = frame.map_err(SubstrateError::from)?;
        let env: UctpEnvelope = serde_json::from_slice(&bytes)?;
        Ok(env)
    })
}

/// Wrap a write half so caller can `.send(env).await`.
pub fn envelope_writer<W>(
    tx: W,
) -> impl Sink<UctpEnvelope, Error = SubstrateError>
where
    W: AsyncWrite + Send + Unpin,
{
    let frames = FramedWrite::new(tx, length_prefixed_codec());
    frames.with(|env: UctpEnvelope| async move {
        let bytes = serde_json::to_vec(&env)?;
        if bytes.len() > MAX_FRAME {
            return Err(SubstrateError::FrameTooLarge(bytes.len()));
        }
        Ok::<Bytes, SubstrateError>(Bytes::from(bytes))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::MessageType;
    use chrono::Utc;

    #[tokio::test]
    async fn duplex_roundtrip_through_codec() {
        let (a, b) = tokio::io::duplex(8192);
        let (a_rd, a_wr) = tokio::io::split(a);
        let (b_rd, b_wr) = tokio::io::split(b);

        let mut writer = Box::pin(envelope_writer(a_wr));
        let mut reader = Box::pin(envelope_reader(b_rd));

        let env = UctpEnvelope {
            v: 1,
            msg_type: MessageType::AuthHello,
            id: "env_x".into(),
            ts: Utc::now(),
            cid: None,
            sid: None,
            connid: None,
            in_reply_to: None,
            payload: serde_json::json!({"hi": "there"}),
        };

        writer.send(env.clone()).await.expect("send");
        // Drop writer so reader sees EOF after the single frame.
        drop(writer);
        drop(a_rd);
        drop(b_wr);

        let got = reader.next().await.unwrap().unwrap();
        assert_eq!(got.id, "env_x");
        assert_eq!(got.msg_type, MessageType::AuthHello);
    }
}
