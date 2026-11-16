//! Provider-neutral Webex BYOVA protocol prototype.
//!
//! This crate enforces wire validation, identity, sequence and reply ordering.
//! It deliberately does not infer endpointing from audio or VAD silence.
//! Network authentication and the choice of agent runtime belong to the host.
//! Full rvoip ConnectionAdapter registration is a subsequent integration phase.
//!
//! # One reply without a network transport
//!
//! The host supplies authenticated bindings and authoritative reply boundaries.
//! Write every returned batch in order before processing the next input.
//!
//! ```
//! use rvoip_webex_byob::{Input, ProtocolError, Session};
//! use serde_json::json;
//!
//! # fn main() -> Result<(), ProtocolError> {
//! let mut session = Session::new("test-org".into(), "test-agent".into());
//! let start = json!({
//!     "type": "VOICE_VA_REQUEST", "seq": 1,
//!     "ts": "2026-11-04T00:00:00Z", "conversation_id": "example-call",
//!     "payload": {
//!         "conversation_id": "example-call", "customer_org_id": "test-org",
//!         "virtual_agent_id": "test-agent",
//!         "voice_va_input_type": {"event_input": {"event_type": "SESSION_START"}}
//!     }
//! });
//! assert!(matches!(session.receive(&start.to_string())?, Input::Start));
//! let reply = session.begin_reply()?;
//! // Synthetic PCMU silence illustrates framing only, not a real agent reply.
//! let mut output = session.audio(reply, &[0xff; 640])?;
//! output.extend(session.finish(reply)?);
//! assert_eq!(output.last().unwrap()["payload"]["response_type"], "FINAL");
//! # Ok(())
//! # }
//! ```

#[cfg(feature = "grpc")]
pub mod grpc;
pub mod protocol;
pub mod session;
#[cfg(feature = "wav-compat")]
pub mod wav;

pub use protocol::{Input, ProtocolError, MAX_AUDIO_BYTES, MAX_TEXT_BYTES};
pub use session::{AudioOutput, ReplyId, Session};
