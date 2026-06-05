//! Typed [`DataChannelOptions`] (RFC 8832 §5.1) + thin `RvoipDataChannel`
//! wrapper exposing bufferedAmount and the low-threshold event.
//!
//! Phase G1 of the gap implementation plan; G-tail closeout adds a
//! background pump + broadcast subscription for `OnBufferedAmountLow`.

use std::sync::{Arc, Mutex};

use bytes::BytesMut;
use rtc::data_channel::RTCDataChannelInit;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
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
/// send_text/send_binary mirroring the W3C `RTCDataChannel` API, plus an
/// optional broadcast subscription for the `bufferedamountlow` event.
///
/// Held inside [`RvoipPeerConnection`](crate::peer::RvoipPeerConnection)'s
/// channel registry; constructed via
/// [`RvoipPeerConnection::create_data_channel`](crate::peer::RvoipPeerConnection::create_data_channel).
///
/// ## Event subscription model
///
/// [`subscribe_buffered_amount_low`](Self::subscribe_buffered_amount_low)
/// and [`subscribe_events`](Self::subscribe_events) spawn a background pump
/// task on first call. The pump owns
/// `inner().poll()`, fans out each [`DataChannelEvent`] on a
/// `tokio::sync::broadcast` channel, and exits when the underlying channel
/// closes.
///
/// **Once any `subscribe_*` has been called**, raw access to
/// [`inner()`](Self::inner)`.poll()` (or the
/// [`RvoipPeerConnection::poll_data_channel`](crate::peer::RvoipPeerConnection::poll_data_channel)
/// helper pointed at the same trait object) races with the pump and is
/// unsupported. Pick one model per data channel.
pub struct RvoipDataChannel {
    state: Arc<DcState>,
}

impl Clone for RvoipDataChannel {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
        }
    }
}

struct DcState {
    dc: Arc<dyn DataChannel>,
    label: String,
    low_tx: broadcast::Sender<()>,
    events_tx: broadcast::Sender<DataChannelEvent>,
    pump: Mutex<Option<JoinHandle<()>>>,
}

impl Drop for DcState {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.pump.lock() {
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }
    }
}

impl RvoipDataChannel {
    pub(crate) fn new(inner: Arc<dyn DataChannel>, label: String) -> Self {
        let (low_tx, _) = broadcast::channel(16);
        let (events_tx, _) = broadcast::channel(64);
        Self {
            state: Arc::new(DcState {
                dc: inner,
                label,
                low_tx,
                events_tx,
                pump: Mutex::new(None),
            }),
        }
    }

    /// Channel label (W3C `RTCDataChannel.label`).
    pub fn label(&self) -> &str {
        &self.state.label
    }

    /// The raw webrtc-rs handle for code that needs the trait object directly.
    /// Calling `inner().poll()` directly is only safe **before** any
    /// `subscribe_*` call on this wrapper; after that the background pump
    /// owns the poll stream.
    pub fn inner(&self) -> &Arc<dyn DataChannel> {
        &self.state.dc
    }

    /// Send a UTF-8 text message. PPID `WEBRTC_STRING` per RFC 8831.
    pub async fn send_text(&self, msg: &str) -> Result<()> {
        self.state
            .dc
            .send_text(msg)
            .await
            .map_err(|e| WebRtcError::Webrtc(format!("dc send_text: {e}")))
    }

    /// Send a binary message. PPID `WEBRTC_BINARY` per RFC 8831.
    pub async fn send_binary(&self, msg: &[u8]) -> Result<()> {
        self.state
            .dc
            .send(BytesMut::from(msg))
            .await
            .map_err(|e| WebRtcError::Webrtc(format!("dc send: {e}")))
    }

    /// Current bytes queued for sending. Mirrors W3C
    /// `RTCDataChannel.bufferedAmount`.
    ///
    /// **Limitation:** webrtc-rs 0.20-alpha does not expose a direct
    /// `buffered_amount()` accessor on the trait, so this method always
    /// returns 0. Use
    /// [`subscribe_buffered_amount_low`](Self::subscribe_buffered_amount_low)
    /// instead — it surfaces the `bufferedamountlow` event that fires when
    /// the underlying buffer drops below the configured low threshold,
    /// which is the W3C-standard backpressure signal.
    pub async fn buffered_amount(&self) -> Result<u64> {
        Ok(0)
    }

    /// Set the bufferedAmount low threshold (W3C
    /// `RTCDataChannel.bufferedAmountLowThreshold`). When the buffered
    /// amount falls *to or below* this value, webrtc-rs emits
    /// [`DataChannelEvent::OnBufferedAmountLow`]; subscribe via
    /// [`Self::subscribe_buffered_amount_low`].
    pub async fn set_buffered_amount_low_threshold(&self, threshold: u32) -> Result<()> {
        self.state
            .dc
            .set_buffered_amount_low_threshold(threshold)
            .await
            .map_err(|e| WebRtcError::Webrtc(format!("set_buffered_amount_low_threshold: {e}")))
    }

    /// Current low-threshold configured on the channel.
    pub async fn buffered_amount_low_threshold(&self) -> Result<u32> {
        self.state
            .dc
            .buffered_amount_low_threshold()
            .await
            .map_err(|e| WebRtcError::Webrtc(format!("buffered_amount_low_threshold: {e}")))
    }

    /// W3C `RTCDataChannel.readyState`.
    pub async fn ready_state(&self) -> Result<RTCDataChannelState> {
        self.state
            .dc
            .ready_state()
            .await
            .map_err(|e| WebRtcError::Webrtc(format!("ready_state: {e}")))
    }

    /// Pump the next data-channel event with a deadline.
    ///
    /// **Only safe before any `subscribe_*` call.** Once the broadcast pump
    /// is running this would race; callers should use
    /// [`Self::subscribe_events`] instead.
    pub async fn poll(&self, timeout: std::time::Duration) -> Option<DataChannelEvent> {
        tokio::time::timeout(timeout, self.state.dc.poll())
            .await
            .ok()
            .flatten()
    }

    /// Subscribe to the `bufferedamountlow` event stream. Each subscription
    /// receives a `()` every time the underlying buffer drops to or below
    /// the configured low threshold (set via
    /// [`Self::set_buffered_amount_low_threshold`]).
    ///
    /// Spawns a single background pump task on first call; subsequent calls
    /// re-use it. The pump exits cleanly when the data channel closes.
    pub fn subscribe_buffered_amount_low(&self) -> broadcast::Receiver<()> {
        self.ensure_pump();
        self.state.low_tx.subscribe()
    }

    /// Subscribe to the full stream of [`DataChannelEvent`]s. Useful when
    /// the caller wants `OnMessage` / `OnOpen` / `OnClose` in addition to
    /// `OnBufferedAmountLow`.
    ///
    /// Same lazy-pump semantics as
    /// [`Self::subscribe_buffered_amount_low`].
    pub fn subscribe_events(&self) -> broadcast::Receiver<DataChannelEvent> {
        self.ensure_pump();
        self.state.events_tx.subscribe()
    }

    fn ensure_pump(&self) {
        let Ok(mut guard) = self.state.pump.lock() else {
            // Mutex poisoned — another task panicked while holding the
            // pump slot. Skip silently rather than propagating; the data
            // channel itself is still usable for sends.
            return;
        };
        if guard.is_some() {
            return;
        }
        let dc = Arc::clone(&self.state.dc);
        let low_tx = self.state.low_tx.clone();
        let events_tx = self.state.events_tx.clone();
        let handle = tokio::spawn(async move {
            loop {
                match dc.poll().await {
                    Some(ev) => {
                        if matches!(ev, DataChannelEvent::OnBufferedAmountLow) {
                            let _ = low_tx.send(());
                        }
                        let _ = events_tx.send(ev);
                    }
                    None => break,
                }
            }
        });
        *guard = Some(handle);
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
