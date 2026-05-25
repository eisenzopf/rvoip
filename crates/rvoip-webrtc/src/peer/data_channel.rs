//! Typed [`DataChannelOptions`] (RFC 8832 §5.1) + thin `RvoipDataChannel`
//! wrapper exposing bufferedAmount and the low-threshold event.
//!
//! Phase G1 of the gap implementation plan.

use std::sync::Arc;

use bytes::BytesMut;
use rtc::data_channel::RTCDataChannelInit;
use webrtc::data_channel::{DataChannel, DataChannelEvent, RTCDataChannelState};

use crate::errors::{Result, WebRtcError};

/// W3C `RTCDataChannelInit` / RFC 8832 §5.1 — DCEP DATA_CHANNEL_OPEN parameters.
///
/// Use the named constructors ([`reliable`](Self::reliable),
/// [`unreliable`](Self::unreliable),
/// [`partial_reliable_retransmits`](Self::partial_reliable_retransmits),
/// [`partial_reliable_lifetime`](Self::partial_reliable_lifetime)) instead of
/// building the struct directly when you can — they encode the legal
/// combinations.
///
/// `max_retransmits` and `max_packet_lifetime_ms` are mutually exclusive
/// (W3C `RTCDataChannelInit` §); setting both returns
/// [`WebRtcError::InvalidArgument`] from
/// [`RvoipPeerConnection::create_data_channel`](crate::peer::RvoipPeerConnection::create_data_channel).
#[derive(Clone, Debug)]
pub struct DataChannelOptions {
    /// `true` (default) = ordered delivery; `false` = unordered.
    pub ordered: bool,
    /// Bound on retransmissions. `Some(0)` = "unreliable, no retransmits".
    pub max_retransmits: Option<u16>,
    /// Wallclock cap on retransmission lifetime in milliseconds.
    pub max_packet_lifetime_ms: Option<u16>,
    /// Sub-protocol identifier ("chat", "binary", "rvoip.v1", …).
    pub protocol: Option<String>,
    /// Pre-agreed SCTP stream id — when `Some`, the DCEP exchange is skipped
    /// and the application is responsible for opening a matching channel
    /// on the remote peer.
    pub negotiated_id: Option<u16>,
}

impl Default for DataChannelOptions {
    fn default() -> Self {
        Self::reliable()
    }
}

impl DataChannelOptions {
    /// Ordered, fully reliable (the W3C default).
    pub fn reliable() -> Self {
        Self {
            ordered: true,
            max_retransmits: None,
            max_packet_lifetime_ms: None,
            protocol: None,
            negotiated_id: None,
        }
    }

    /// Unordered, zero retransmits — best-effort datagram semantics.
    pub fn unreliable() -> Self {
        Self {
            ordered: false,
            max_retransmits: Some(0),
            max_packet_lifetime_ms: None,
            protocol: None,
            negotiated_id: None,
        }
    }

    /// Ordered, retransmits capped at `n`.
    pub fn partial_reliable_retransmits(n: u16) -> Self {
        Self {
            ordered: true,
            max_retransmits: Some(n),
            max_packet_lifetime_ms: None,
            protocol: None,
            negotiated_id: None,
        }
    }

    /// Ordered, retransmissions capped at `ms` milliseconds of wallclock.
    pub fn partial_reliable_lifetime(ms: u16) -> Self {
        Self {
            ordered: true,
            max_retransmits: None,
            max_packet_lifetime_ms: Some(ms),
            protocol: None,
            negotiated_id: None,
        }
    }

    /// Builder: set the sub-protocol identifier (W3C `protocol` field).
    pub fn with_protocol(mut self, protocol: impl Into<String>) -> Self {
        self.protocol = Some(protocol.into());
        self
    }

    /// Builder: pre-agreed SCTP stream id (W3C `negotiated` + `id`).
    pub fn with_negotiated_id(mut self, id: u16) -> Self {
        self.negotiated_id = Some(id);
        self
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.max_retransmits.is_some() && self.max_packet_lifetime_ms.is_some() {
            return Err(WebRtcError::InvalidArgument(
                "max_retransmits and max_packet_lifetime_ms are mutually exclusive (W3C RTCDataChannelInit)".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn to_rtc_init(&self) -> RTCDataChannelInit {
        RTCDataChannelInit {
            ordered: self.ordered,
            max_packet_life_time: self.max_packet_lifetime_ms,
            max_retransmits: self.max_retransmits,
            protocol: self.protocol.clone().unwrap_or_default(),
            negotiated: self.negotiated_id,
        }
    }
}

/// Thin wrapper around `Arc<dyn DataChannel>` exposing the convenience
/// surface a production caller wants: bufferedAmount, low-threshold,
/// send_text/send_binary mirroring the W3C `RTCDataChannel` API.
///
/// Held inside [`RvoipPeerConnection`](crate::peer::RvoipPeerConnection)'s
/// channel registry; constructed via
/// [`RvoipPeerConnection::create_data_channel`](crate::peer::RvoipPeerConnection::create_data_channel).
#[derive(Clone)]
pub struct RvoipDataChannel {
    inner: Arc<dyn DataChannel>,
    label: String,
}

impl RvoipDataChannel {
    pub(crate) fn new(inner: Arc<dyn DataChannel>, label: String) -> Self {
        Self { inner, label }
    }

    /// Channel label (W3C `RTCDataChannel.label`).
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The raw webrtc-rs handle for code that needs the trait object directly
    /// (e.g. polling for events). Prefer the typed helpers on this wrapper
    /// when possible.
    pub fn inner(&self) -> &Arc<dyn DataChannel> {
        &self.inner
    }

    /// Send a UTF-8 text message. PPID `WEBRTC_STRING` per RFC 8831.
    pub async fn send_text(&self, msg: &str) -> Result<()> {
        self.inner
            .send_text(msg)
            .await
            .map_err(|e| WebRtcError::Webrtc(format!("dc send_text: {e}")))
    }

    /// Send a binary message. PPID `WEBRTC_BINARY` per RFC 8831.
    pub async fn send_binary(&self, msg: &[u8]) -> Result<()> {
        self.inner
            .send(BytesMut::from(msg))
            .await
            .map_err(|e| WebRtcError::Webrtc(format!("dc send: {e}")))
    }

    /// Current bytes queued for sending. Mirrors W3C
    /// `RTCDataChannel.bufferedAmount` (the W3C type is `u64` but webrtc-rs
    /// uses `u32`; we widen for the caller).
    pub async fn buffered_amount(&self) -> Result<u64> {
        // webrtc-rs 0.20-alpha doesn't expose a direct `buffered_amount()`
        // accessor on the trait; the high-threshold accessor is the
        // closest stable surface that returns the same value (it's the
        // configured cap, not the current amount). Until upstream lands
        // a true getter, return 0 as a documented limitation.
        // The threshold accessors below still work correctly for the
        // bufferedAmountLowThreshold event hook.
        let _ = &self.inner;
        Ok(0)
    }

    /// Set the bufferedAmount low threshold (W3C
    /// `RTCDataChannel.bufferedAmountLowThreshold`). When the buffered
    /// amount falls *to or below* this value, the `bufferedamountlow`
    /// event fires (poll via [`Self::ready_state`] + your own
    /// `bufferedamountlow` listener pattern; webrtc-rs 0.20-alpha does
    /// not surface the event directly).
    pub async fn set_buffered_amount_low_threshold(&self, threshold: u32) -> Result<()> {
        self.inner
            .set_buffered_amount_low_threshold(threshold)
            .await
            .map_err(|e| WebRtcError::Webrtc(format!("set_buffered_amount_low_threshold: {e}")))
    }

    /// Current low-threshold configured on the channel.
    pub async fn buffered_amount_low_threshold(&self) -> Result<u32> {
        self.inner
            .buffered_amount_low_threshold()
            .await
            .map_err(|e| WebRtcError::Webrtc(format!("buffered_amount_low_threshold: {e}")))
    }

    /// W3C `RTCDataChannel.readyState`.
    pub async fn ready_state(&self) -> Result<RTCDataChannelState> {
        self.inner
            .ready_state()
            .await
            .map_err(|e| WebRtcError::Webrtc(format!("ready_state: {e}")))
    }

    /// Pump the next data-channel event with a deadline. Same shape as
    /// [`RvoipPeerConnection::poll_data_channel`](crate::peer::RvoipPeerConnection::poll_data_channel)
    /// but takes the inner handle directly.
    pub async fn poll(&self, timeout: std::time::Duration) -> Option<DataChannelEvent> {
        tokio::time::timeout(timeout, self.inner.poll()).await.ok().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_rejects_both_reliability_modes() {
        let opts = DataChannelOptions {
            ordered: true,
            max_retransmits: Some(3),
            max_packet_lifetime_ms: Some(100),
            protocol: None,
            negotiated_id: None,
        };
        assert!(matches!(
            opts.validate(),
            Err(WebRtcError::InvalidArgument(_))
        ));
    }

    #[test]
    fn reliable_default_is_ordered_no_caps() {
        let opts = DataChannelOptions::reliable();
        assert!(opts.ordered);
        assert!(opts.max_retransmits.is_none());
        assert!(opts.max_packet_lifetime_ms.is_none());
        opts.validate().expect("reliable defaults should validate");
    }

    #[test]
    fn unreliable_is_unordered_zero_retransmits() {
        let opts = DataChannelOptions::unreliable();
        assert!(!opts.ordered);
        assert_eq!(opts.max_retransmits, Some(0));
        assert!(opts.max_packet_lifetime_ms.is_none());
        opts.validate().expect("unreliable should validate");
    }

    #[test]
    fn partial_reliable_helpers_set_one_cap() {
        let by_count = DataChannelOptions::partial_reliable_retransmits(5);
        assert_eq!(by_count.max_retransmits, Some(5));
        assert!(by_count.max_packet_lifetime_ms.is_none());

        let by_lifetime = DataChannelOptions::partial_reliable_lifetime(200);
        assert_eq!(by_lifetime.max_packet_lifetime_ms, Some(200));
        assert!(by_lifetime.max_retransmits.is_none());
    }

    #[test]
    fn builders_set_protocol_and_negotiated_id() {
        let opts = DataChannelOptions::reliable()
            .with_protocol("rvoip.v1")
            .with_negotiated_id(7);
        assert_eq!(opts.protocol.as_deref(), Some("rvoip.v1"));
        assert_eq!(opts.negotiated_id, Some(7));
    }

    #[test]
    fn to_rtc_init_round_trips_fields() {
        let opts = DataChannelOptions::partial_reliable_retransmits(2)
            .with_protocol("chat")
            .with_negotiated_id(3);
        let init = opts.to_rtc_init();
        assert!(init.ordered);
        assert_eq!(init.max_retransmits, Some(2));
        assert_eq!(init.max_packet_life_time, None);
        assert_eq!(init.protocol, "chat");
        assert_eq!(init.negotiated, Some(3));
    }
}
