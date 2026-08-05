//! Static Vapi adapter configuration.

use std::fmt;
use std::time::Duration;

use url::Url;
use zeroize::Zeroize;

use crate::error::{Result, VapiError};

/// API credential whose diagnostics and drop behavior are secret-safe.
#[derive(Clone, Eq, PartialEq)]
pub struct VapiApiKey(String);

impl VapiApiKey {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let mut value = value.into();
        if value.trim().is_empty() {
            value.zeroize();
            return Err(VapiError::InvalidConfiguration(
                "the API key must not be empty",
            ));
        }
        if value.chars().any(char::is_control) {
            value.zeroize();
            return Err(VapiError::InvalidConfiguration(
                "the API key contains control characters",
            ));
        }
        Ok(Self(value))
    }

    pub(crate) fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for VapiApiKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VapiApiKey([redacted])")
    }
}

impl Drop for VapiApiKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// Static configuration shared by all calls placed through an adapter.
#[derive(Clone)]
pub struct VapiConfig {
    pub api_key: VapiApiKey,
    pub api_base: Url,
    pub http_timeout: Duration,
    pub websocket_timeout: Duration,
    pub websocket_io_timeout: Duration,
    /// Deadline for a single *media* frame write, distinct from
    /// [`Self::websocket_io_timeout`] which governs control traffic.
    ///
    /// Media writes are awaited inside the session's select loop, so a slow
    /// write parks the whole loop — inbound audio processing included. At the
    /// control timeout (10 s) that stall outlives the downstream media graph's
    /// slow-consumer budget, so a transient TCP hiccup gets the call's routes
    /// evicted and its audio blackholed before the write ever fails.
    ///
    /// A media frame that cannot be written within a few frame times is stale
    /// anyway: real-time audio is better dropped than delayed. This bounds how
    /// long one write may hold the loop.
    pub media_write_timeout: Duration,
    /// Consecutive media-write timeouts tolerated before the session is
    /// declared failed. Dropping frames is recoverable; a peer that has not
    /// accepted a write for this many attempts is not.
    pub media_write_timeout_limit: u32,
    pub graceful_shutdown_timeout: Duration,
    pub heartbeat_interval: Duration,
    pub media_queue_capacity: usize,
    pub control_queue_capacity: usize,
    pub event_queue_capacity: usize,
    pub max_message_bytes: usize,
    /// Frames buffered before the first is handed downstream.
    ///
    /// Historically this also served as the inbound queue bound. It no longer
    /// does: see [`VapiConfig::inbound_queue_capacity`].
    pub startup_audio_frames: usize,
    /// Inbound jitter buffer depth, in frames. **This is a latency ceiling.**
    ///
    /// The drain releases one 20 ms frame per tick and never bursts to catch
    /// up, so once a backlog forms the depth does not recover: queue depth
    /// becomes permanent one-way delay for the rest of the call. Capacity is
    /// therefore a conversational-latency budget, not merely an overflow
    /// bound — `N` frames is `N * 20 ms` of worst-case added delay.
    ///
    /// Sizing: Vapi's transport is a raw byte stream with no pacing guarantee,
    /// measured delivering ~90 ms of audio in a single burst (chunks of
    /// 170-743 bytes, p50 inter-arrival 50 ms, minimum 0 ms). A healthy call
    /// sits at 3-4 frames. 25 frames (500 ms) absorbs a measured burst five
    /// times over while capping added delay below the point where turn-taking
    /// and barge-in break down.
    pub inbound_queue_capacity: usize,
    /// Depth the jitter buffers converge back down to, in frames.
    ///
    /// A capacity alone only *bounds* delay; it does not remove it. Because the
    /// drain releases one frame per tick, a queue that fills stays filled, so a
    /// 500 ms ceiling becomes a permanent 500 ms of delay. FreeSWITCH hit this
    /// on RTP and concluded a cap was insufficient
    /// (signalwire/freeswitch#2069: "I noticed it was not
    /// accelerating/shrinking... we decided to cap the maximum size" — the cap
    /// was tried first and judged not to be the fix). Its remedy is
    /// `skip_timer`: above a threshold the read loop yields instead of waiting
    /// the tick, draining faster than real time until depth recovers.
    ///
    /// This is the same valve. Above `jitter_target_frames` the drain releases
    /// up to `max_catchup_frames_per_tick` extra frames per tick, so a backlog
    /// re-converges instead of persisting for the rest of the call. Audio is
    /// preserved rather than dropped: frames carry sequential RTP timestamps,
    /// so the receiver's own buffer absorbs the early arrival.
    pub jitter_target_frames: usize,
    /// Extra frames the drain may release per tick while re-converging. One
    /// extra frame is 2x real time, which clears a full 500 ms buffer in 500 ms
    /// without flooding the peer.
    pub max_catchup_frames_per_tick: usize,
    /// Outbound jitter buffer depth, in frames. **This is a latency ceiling**,
    /// with the same semantics as [`Self::inbound_queue_capacity`].
    ///
    /// Kept separate from `media_queue_capacity`, which sizes the media stream
    /// channels rather than this jitter buffer.
    pub outbound_queue_capacity: usize,
    pub(crate) allow_insecure_transport: bool,
}

impl VapiConfig {
    pub fn new(api_key: VapiApiKey) -> Self {
        let api_base = match Url::parse("https://api.vapi.ai/") {
            Ok(url) => url,
            Err(_) => unreachable!("the built-in Vapi API URL is valid"),
        };
        Self {
            api_key,
            api_base,
            http_timeout: Duration::from_secs(10),
            websocket_timeout: Duration::from_secs(10),
            websocket_io_timeout: Duration::from_secs(10),
            media_write_timeout: Duration::from_millis(200),
            media_write_timeout_limit: 50,
            graceful_shutdown_timeout: Duration::from_secs(2),
            heartbeat_interval: Duration::from_secs(20),
            media_queue_capacity: 100,
            control_queue_capacity: 32,
            event_queue_capacity: 100,
            max_message_bytes: 1024 * 1024,
            startup_audio_frames: 100,
            inbound_queue_capacity: 25,
            outbound_queue_capacity: 25,
            jitter_target_frames: 5,
            max_catchup_frames_per_tick: 1,
            allow_insecure_transport: false,
        }
    }

    pub fn with_api_base(mut self, api_base: Url) -> Self {
        self.api_base = api_base;
        self
    }

    /// Permit HTTP and WS endpoints for a loopback-only test server.
    ///
    /// Production callers should never enable this. Validation rejects an
    /// insecure host that is not an IP loopback or `localhost`.
    pub fn with_loopback_test_transport(mut self) -> Self {
        self.allow_insecure_transport = true;
        self
    }

    pub fn validate(&self) -> Result<()> {
        if self.api_base.scheme() != "https"
            && !(self.allow_insecure_transport && is_loopback_url(&self.api_base))
        {
            return Err(VapiError::InvalidConfiguration(
                "the API base must use HTTPS",
            ));
        }
        if self.http_timeout.is_zero()
            || self.websocket_timeout.is_zero()
            || self.websocket_io_timeout.is_zero()
            || self.media_write_timeout.is_zero()
            || self.graceful_shutdown_timeout.is_zero()
            || self.heartbeat_interval.is_zero()
        {
            return Err(VapiError::InvalidConfiguration(
                "timeouts and heartbeat interval must be non-zero",
            ));
        }
        if self.media_queue_capacity == 0
            || self.control_queue_capacity == 0
            || self.event_queue_capacity == 0
            || self.startup_audio_frames == 0
            || self.inbound_queue_capacity == 0
            || self.outbound_queue_capacity == 0
        {
            return Err(VapiError::InvalidConfiguration(
                "queue capacities must be non-zero",
            ));
        }
        if self.startup_audio_frames > 100 {
            return Err(VapiError::InvalidConfiguration(
                "startup audio buffering cannot exceed 100 frames",
            ));
        }
        if self.max_message_bytes < 640 {
            return Err(VapiError::InvalidConfiguration(
                "maximum message size is too small for one PCM frame",
            ));
        }
        Ok(())
    }

    pub(crate) fn permits_websocket_url(&self, url: &Url) -> bool {
        url.scheme() == "wss"
            || (self.allow_insecure_transport && url.scheme() == "ws" && is_loopback_url(url))
    }
}

impl fmt::Debug for VapiConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VapiConfig")
            .field("api_key", &self.api_key)
            .field("api_base", &"[redacted]")
            .field("http_timeout", &self.http_timeout)
            .field("websocket_timeout", &self.websocket_timeout)
            .field("websocket_io_timeout", &self.websocket_io_timeout)
            .field("media_write_timeout", &self.media_write_timeout)
            .field("graceful_shutdown_timeout", &self.graceful_shutdown_timeout)
            .field("heartbeat_interval", &self.heartbeat_interval)
            .field("media_queue_capacity", &self.media_queue_capacity)
            .field("control_queue_capacity", &self.control_queue_capacity)
            .field("event_queue_capacity", &self.event_queue_capacity)
            .field("max_message_bytes", &self.max_message_bytes)
            .field("startup_audio_frames", &self.startup_audio_frames)
            .field("inbound_queue_capacity", &self.inbound_queue_capacity)
            .field("outbound_queue_capacity", &self.outbound_queue_capacity)
            .field("insecure_transport", &self.allow_insecure_transport)
            .finish()
    }
}

fn is_loopback_url(url: &Url) -> bool {
    url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_redact_key_and_endpoint() {
        let config = VapiConfig::new(VapiApiKey::new("key-canary").unwrap())
            .with_api_base(Url::parse("https://endpoint-canary.invalid/").unwrap());
        let debug = format!("{config:?}");
        assert!(!debug.contains("key-canary"));
        assert!(!debug.contains("endpoint-canary"));
    }

    /// The jitter-buffer depths are latency ceilings: the drain never bursts
    /// to catch up, so a queue that fills stays filled and its depth becomes
    /// permanent one-way delay. Raising these past a conversational budget
    /// silently destroys turn-taking and barge-in while every metric still
    /// reports success, so the defaults are guarded here deliberately.
    #[test]
    fn jitter_buffer_defaults_stay_within_a_conversational_latency_budget() {
        let config = VapiConfig::new(VapiApiKey::new("k").expect("key"));
        const FRAME_MS: usize = 20;
        const BUDGET_MS: usize = 600;
        assert!(
            config.inbound_queue_capacity * FRAME_MS <= BUDGET_MS,
            "inbound jitter buffer is {} ms of worst-case added delay",
            config.inbound_queue_capacity * FRAME_MS
        );
        assert!(
            config.outbound_queue_capacity * FRAME_MS <= BUDGET_MS,
            "outbound jitter buffer is {} ms of worst-case added delay",
            config.outbound_queue_capacity * FRAME_MS
        );
        // Still comfortably larger than the ~90 ms burst measured from live
        // Vapi, or the fix for fatal overflow just trades one bug for another.
        assert!(config.inbound_queue_capacity >= 10);
        assert!(config.outbound_queue_capacity >= 10);
    }

    #[test]
    fn insecure_transport_is_loopback_only() {
        let key = VapiApiKey::new("test").unwrap();
        let loopback = VapiConfig::new(key.clone())
            .with_api_base(Url::parse("http://127.0.0.1:8000/").unwrap())
            .with_loopback_test_transport();
        assert!(loopback.validate().is_ok());

        let remote = VapiConfig::new(key)
            .with_api_base(Url::parse("http://example.com/").unwrap())
            .with_loopback_test_transport();
        assert!(remote.validate().is_err());
    }

    #[test]
    fn startup_audio_is_capped_at_two_seconds() {
        let mut config = VapiConfig::new(VapiApiKey::new("test").unwrap());
        config.startup_audio_frames = 100;
        assert!(config.validate().is_ok());
        config.startup_audio_frames = 101;
        assert!(config.validate().is_err());
    }
}
