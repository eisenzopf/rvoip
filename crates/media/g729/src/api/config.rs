/// Convenience runtime options shared by encoder and decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct G729Config {
    /// Enable Annex B VAD/DTX/CNG processing when the feature is compiled in.
    pub annex_b: bool,
}

impl Default for G729Config {
    fn default() -> Self {
        Self {
            annex_b: cfg!(feature = "annex_b"),
        }
    }
}

/// Runtime options for encoder behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderConfig {
    /// Enable Annex B VAD/DTX/CNG processing when the feature is compiled in.
    pub annex_b: bool,
}

impl Default for EncoderConfig {
    fn default() -> Self {
        Self {
            annex_b: cfg!(feature = "annex_b"),
        }
    }
}

impl From<G729Config> for EncoderConfig {
    fn from(value: G729Config) -> Self {
        Self {
            annex_b: value.annex_b,
        }
    }
}

/// Runtime options for decoder behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecoderConfig {
    /// Enable Annex B SID/no-data handling when the feature is compiled in.
    pub annex_b: bool,
    /// Enable Annex A post-filter stage on decoded speech.
    pub post_filter: bool,
    /// Maximum tolerated consecutive erasures before output is muted.
    ///
    /// `None` disables muting regardless of erasure burst length.
    pub max_consecutive_erasures: Option<usize>,
}

impl Default for DecoderConfig {
    fn default() -> Self {
        Self {
            annex_b: cfg!(feature = "annex_b"),
            post_filter: true,
            max_consecutive_erasures: Some(10),
        }
    }
}

impl From<G729Config> for DecoderConfig {
    fn from(value: G729Config) -> Self {
        Self {
            annex_b: value.annex_b,
            post_filter: true,
            max_consecutive_erasures: Some(10),
        }
    }
}
