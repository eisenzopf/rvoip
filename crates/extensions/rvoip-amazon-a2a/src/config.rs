use crate::{A2aError, Result};
use std::{fmt, time::Duration};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

/// Bearer credential checked at the WebSocket upgrade, never logged.
pub struct ApiKey(Zeroizing<String>);
impl ApiKey {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.len() < 16
            || value.len() > 4096
            || value
                .bytes()
                .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
        {
            return Err(A2aError::Configuration);
        }
        Ok(Self(Zeroizing::new(value)))
    }
    pub(crate) fn matches(&self, value: &str) -> bool {
        bool::from(self.0.as_bytes().ct_eq(value.as_bytes()))
    }
}
impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey([redacted])")
    }
}

/// Resource limits are per adapter or per socket; no unbounded media queues.
#[derive(Debug)]
pub struct AmazonA2aConfig {
    pub api_key: ApiKey,
    /// Counter-proposed for both directions (8, 16, or 24 kHz PCM16 mono).
    pub sample_rate_hz: u32,
    pub max_connections: usize,
    pub queue_capacity: usize,
    pub max_frame_bytes: usize,
    pub max_payload_bytes: usize,
    pub max_fragment_parts: usize,
    pub initialization_timeout: Duration,
    pub fragment_timeout: Duration,
    pub write_timeout: Duration,
    pub idle_timeout: Duration,
}
impl AmazonA2aConfig {
    pub fn new(api_key: ApiKey) -> Self {
        Self {
            api_key,
            sample_rate_hz: 16000,
            max_connections: 100,
            queue_capacity: 64,
            max_frame_bytes: 16 * 1024,
            max_payload_bytes: 1024 * 1024,
            max_fragment_parts: 256,
            initialization_timeout: Duration::from_secs(10),
            fragment_timeout: Duration::from_secs(10),
            write_timeout: Duration::from_secs(5),
            idle_timeout: Duration::from_secs(60),
        }
    }
    pub(crate) fn validate(&self) -> Result<()> {
        if ![8000, 16000, 24000].contains(&self.sample_rate_hz)
            || self.max_connections == 0
            || self.max_connections > 10000
            || self.queue_capacity == 0
            || self.queue_capacity > 4096
            || self.max_frame_bytes < 2048
            || self.max_frame_bytes > 1024 * 1024
            || self.max_payload_bytes < self.max_frame_bytes
            || self.max_payload_bytes > 16 * 1024 * 1024
            || self.max_fragment_parts == 0
            || self.max_fragment_parts > 4096
            || [
                self.initialization_timeout,
                self.fragment_timeout,
                self.write_timeout,
                self.idle_timeout,
            ]
            .iter()
            .any(Duration::is_zero)
        {
            return Err(A2aError::Configuration);
        }
        Ok(())
    }
}
