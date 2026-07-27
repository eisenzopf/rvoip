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
    pub graceful_shutdown_timeout: Duration,
    pub heartbeat_interval: Duration,
    pub media_queue_capacity: usize,
    pub control_queue_capacity: usize,
    pub event_queue_capacity: usize,
    pub max_message_bytes: usize,
    pub startup_audio_frames: usize,
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
            graceful_shutdown_timeout: Duration::from_secs(2),
            heartbeat_interval: Duration::from_secs(20),
            media_queue_capacity: 100,
            control_queue_capacity: 32,
            event_queue_capacity: 100,
            max_message_bytes: 1024 * 1024,
            startup_audio_frames: 100,
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
            .field("graceful_shutdown_timeout", &self.graceful_shutdown_timeout)
            .field("heartbeat_interval", &self.heartbeat_interval)
            .field("media_queue_capacity", &self.media_queue_capacity)
            .field("control_queue_capacity", &self.control_queue_capacity)
            .field("event_queue_capacity", &self.event_queue_capacity)
            .field("max_message_bytes", &self.max_message_bytes)
            .field("startup_audio_frames", &self.startup_audio_frames)
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
