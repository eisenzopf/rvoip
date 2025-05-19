use crate::quality::metrics::{QualityMetrics, NetworkMetrics, AudioMetrics};

/// Quality score (including MOS, R-factor)
#[derive(Debug, Clone, Copy)]
pub struct QualityScore {
    /// Mean Opinion Score (1.0-5.0)
    pub mos: f32,
    /// R-factor (0-100, 0=worst, 100=best)
    pub r_factor: f32,
    /// Quality level category
    pub level: QualityLevel,
}

impl Default for QualityScore {
    fn default() -> Self {
        Self {
            mos: 4.0, // Default to "good"
            r_factor: 80.0, // Default to "good"
            level: QualityLevel::Good,
        }
    }
}

/// Quality level categorization
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityLevel {
    /// Excellent quality (MOS > 4.3)
    Excellent,
    /// Good quality (MOS 4.0-4.3)
    Good,
    /// Fair quality (MOS 3.6-4.0)
    Fair,
    /// Poor quality (MOS 3.1-3.6)
    Poor,
    /// Bad quality (MOS < 3.1)
    Bad,
}

impl QualityLevel {
    /// Create a quality level from a MOS score
    pub fn from_mos(mos: f32) -> Self {
        if mos >= 4.3 {
            Self::Excellent
        } else if mos >= 4.0 {
            Self::Good
        } else if mos >= 3.6 {
            Self::Fair
        } else if mos >= 3.1 {
            Self::Poor
        } else {
            Self::Bad
        }
    }
    
    /// Create a quality level from R-factor
    pub fn from_r_factor(r: f32) -> Self {
        if r >= 90.0 {
            Self::Excellent
        } else if r >= 80.0 {
            Self::Good
        } else if r >= 70.0 {
            Self::Fair
        } else if r >= 60.0 {
            Self::Poor
        } else {
            Self::Bad
        }
    }
    
    /// Get a description of the quality level
    pub fn description(&self) -> &'static str {
        match self {
            Self::Excellent => "Excellent quality, very satisfied",
            Self::Good => "Good quality, satisfied",
            Self::Fair => "Fair quality, some users dissatisfied",
            Self::Poor => "Poor quality, many users dissatisfied",
            Self::Bad => "Bad quality, nearly all users dissatisfied",
        }
    }
}

/// Quality estimator using E-model (ITU-T G.107)
pub struct QualityEstimator {
    /// Last calculated quality score
    last_score: QualityScore,
    /// Codec-specific base R factor
    base_r_factor: f32,
}

impl QualityEstimator {
    /// Create a new quality estimator
    pub fn new() -> Self {
        Self {
            last_score: QualityScore::default(),
            base_r_factor: 93.2, // Default for G.711
        }
    }
    
    /// Create a new estimator with a specific codec base quality
    pub fn with_codec_quality(base_r_factor: f32) -> Self {
        Self {
            last_score: QualityScore::default(),
            base_r_factor: base_r_factor.clamp(0.0, 100.0),
        }
    }
    
    /// Set the base R-factor for the codec
    /// 
    /// Typical values:
    /// - G.711: 93.2
    /// - G.722: 94.3
    /// - G.729: 82.0
    /// - Opus: 93.5
    pub fn set_codec_quality(&mut self, base_r_factor: f32) {
        self.base_r_factor = base_r_factor.clamp(0.0, 100.0);
    }
    
    /// Estimate quality from metrics
    pub fn estimate(&mut self, metrics: &QualityMetrics) -> QualityScore {
        // Calculate R-factor using the E-model
        let r_factor = self.calculate_r_factor(&metrics.network, &metrics.audio);
        
        // Convert R-factor to MOS using standard formula
        let mos = if r_factor < 0.0 {
            1.0
        } else if r_factor > 100.0 {
            4.5
        } else {
            1.0 + 0.035 * r_factor + r_factor * (r_factor - 60.0) * (100.0 - r_factor) * 7.0e-6
        };
        
        // Determine quality level
        let level = QualityLevel::from_mos(mos);
        
        // Store and return the score
        self.last_score = QualityScore { mos, r_factor, level };
        self.last_score
    }
    
    /// Get the last calculated quality score
    pub fn score(&self) -> QualityScore {
        self.last_score
    }
    
    /// Calculate R-factor using E-model
    fn calculate_r_factor(&self, network: &NetworkMetrics, audio: &AudioMetrics) -> f32 {
        // Start with the base R-factor for the codec
        let r0 = self.base_r_factor;
        
        // Calculate impairments
        
        // Id - Delay impairment
        let one_way_delay_ms = network.rtt_ms / 2.0;
        let id = self.delay_impairment(one_way_delay_ms);
        
        // Ie_eff - Equipment impairment including packet loss
        let ie_eff = self.loss_impairment(network.packet_loss, network.consecutive_losses);
        
        // Is - Simultaneous impairment (jitter, etc.)
        let is_factor = self.simultaneous_impairment(network.jitter_ms, audio.clarity);
        
        // Calculate final R-factor: R = R0 - Id - Ie_eff - Is + A
        // Where A is advantage factor (e.g., mobility advantage for cell phones)
        let a = 0.0; // No advantage factor in this implementation
        
        let r = r0 - id - ie_eff - is_factor + a;
        
        // Clamp to valid range
        r.clamp(0.0, 100.0)
    }
    
    /// Calculate delay impairment factor
    fn delay_impairment(&self, one_way_delay_ms: f32) -> f32 {
        if one_way_delay_ms < 100.0 {
            // Minimal impact below 100ms
            return 0.0;
        }
        
        // ITU-T G.107 delay impairment formula (simplified)
        let h = 0.024 * one_way_delay_ms + 0.11 * (one_way_delay_ms - 177.3) * if one_way_delay_ms - 177.3 > 0.0 { 1.0 } else { 0.0 };
        
        // Scale the impairment
        (0.024 * one_way_delay_ms + h).clamp(0.0, 30.0)
    }
    
    /// Calculate loss impairment factor
    fn loss_impairment(&self, packet_loss: f32, consecutive_losses: u32) -> f32 {
        // Base impairment from random packet loss
        let mut ie = 30.0 * packet_loss / (packet_loss + 10.0);
        
        // Additional impairment from burst losses (consecutive losses)
        if consecutive_losses > 1 {
            // Burst losses have more impact
            let burst_factor = 1.0 + 0.2 * consecutive_losses as f32;
            ie *= burst_factor;
        }
        
        ie.clamp(0.0, 40.0)
    }
    
    /// Calculate simultaneous impairment factor (jitter, clarity, etc.)
    fn simultaneous_impairment(&self, jitter_ms: f32, clarity: f32) -> f32 {
        // Jitter impairment
        let jitter_impairment = if jitter_ms < 20.0 {
            0.0 // Minimal impact for low jitter
        } else {
            // Exponential impact for higher jitter
            (jitter_ms - 20.0) * 0.2
        };
        
        // Clarity impairment (inverse of clarity)
        let clarity_impairment = (1.0 - clarity) * 15.0;
        
        // Combine impairments
        (jitter_impairment + clarity_impairment).clamp(0.0, 15.0)
    }
    
    /// Convert a MOS score to a user-friendly string
    pub fn mos_to_string(mos: f32) -> &'static str {
        let level = QualityLevel::from_mos(mos);
        level.description()
    }
}

impl Default for QualityEstimator {
    fn default() -> Self {
        Self::new()
    }
} 