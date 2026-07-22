//! RFC 4733 telephone-event (DTMF) codec.
//!
//! Moved to `rvoip-rtp-core` so a caller with its own I/O can use the pure
//! encode/decode without depending on the rest of the media pipeline; see
//! `rvoip_rtp_core::dtmf` for the implementation and tests. Re-exported here
//! under the original path so existing `rvoip-media-core` consumers don't
//! need to change their imports.

pub use rvoip_rtp_core::{DtmfEvent, TelephoneEvent};
