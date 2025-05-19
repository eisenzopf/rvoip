use std::time::{Duration, Instant};

use tracing::{debug, info, warn};

use crate::quality::metrics::QualityMetrics;
use crate::quality::estimation::{QualityScore, QualityLevel};
use crate::codec::audio::common::{BitrateMode, QualityMode};

/// Configuration for quality adaptation
#[derive(Debug, Clone)]
pub struct AdaptationConfig {
    /// Minimum acceptable MOS score
    pub min_mos: f32,
    /// Target MOS score
    pub target_mos: f32,
    /// How quickly to adapt to changes (0.0-1.0)
    pub adaptation_rate: f32,
    /// Minimum time between adaptations (ms)
    pub min_adaptation_interval_ms: u32,
    /// Maximum bitrate in kbps
    pub max_bitrate_kbps: u32,
    /// Minimum bitrate in kbps
    pub min_bitrate_kbps: u32,
    /// Whether to adapt the codec
    pub adapt_codec: bool,
    /// Whether to adapt the buffer size
    pub adapt_buffer: bool,
    /// Whether to use redundant encoding on poor networks
    pub use_redundancy: bool,
    /// Whether to adapt FEC level
    pub adapt_fec: bool,
}

impl Default for AdaptationConfig {
    fn default() -> Self {
        Self {
            min_mos: 3.0,
            target_mos: 4.0,
            adaptation_rate: 0.2,
            min_adaptation_interval_ms: 5000, // 5 seconds
            max_bitrate_kbps: 64,
            min_bitrate_kbps: 8,
            adapt_codec: true,
            adapt_buffer: true,
            use_redundancy: true,
            adapt_fec: true,
        }
    }
}

/// Network conditions category
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkCondition {
    /// Excellent network
    Excellent,
    /// Good network
    Good,
    /// Fair network
    Fair,
    /// Poor network
    Poor,
    /// Bad network
    Bad,
}

impl NetworkCondition {
    /// Create from quality metrics
    pub fn from_metrics(metrics: &QualityMetrics) -> Self {
        // Packet loss is a key factor
        let packet_loss = metrics.network.packet_loss;
        
        if packet_loss < 0.01 && metrics.network.jitter_ms < 10.0 && metrics.network.rtt_ms < 150.0 {
            Self::Excellent
        } else if packet_loss < 0.03 && metrics.network.jitter_ms < 30.0 && metrics.network.rtt_ms < 250.0 {
            Self::Good
        } else if packet_loss < 0.05 && metrics.network.jitter_ms < 50.0 && metrics.network.rtt_ms < 300.0 {
            Self::Fair
        } else if packet_loss < 0.10 && metrics.network.jitter_ms < 100.0 && metrics.network.rtt_ms < 500.0 {
            Self::Poor
        } else {
            Self::Bad
        }
    }
    
    /// Get a description of the network condition
    pub fn description(&self) -> &'static str {
        match self {
            Self::Excellent => "Excellent network conditions",
            Self::Good => "Good network conditions",
            Self::Fair => "Fair network conditions",
            Self::Poor => "Poor network conditions",
            Self::Bad => "Bad network conditions",
        }
    }
}

/// Adaptation action to take in response to quality changes
#[derive(Debug, Clone)]
pub enum AdaptationAction {
    /// No action needed
    NoAction,
    /// Change codec bitrate
    ChangeBitrate {
        /// New bitrate in kbps
        bitrate_kbps: u32,
    },
    /// Change codec mode
    ChangeCodecMode {
        /// New bitrate mode
        bitrate_mode: BitrateMode,
        /// New quality mode
        quality_mode: QualityMode,
    },
    /// Change buffer size
    ChangeBufferSize {
        /// New buffer size in milliseconds
        buffer_ms: u32,
    },
    /// Enable or disable redundancy
    SetRedundancy {
        /// Whether redundancy should be enabled
        enabled: bool,
    },
    /// Change FEC level
    ChangeFec {
        /// FEC level (0.0-1.0)
        level: f32,
    },
}

/// Quality adapter for dynamic quality control
pub struct QualityAdapter {
    /// Configuration
    config: AdaptationConfig,
    /// Current network condition
    network_condition: NetworkCondition,
    /// Last adaptation time
    last_adaptation: Instant,
    /// Current bitrate in kbps
    current_bitrate_kbps: u32,
    /// Current buffer size in ms
    current_buffer_ms: u32,
    /// Current redundancy state
    redundancy_enabled: bool,
    /// Current FEC level
    fec_level: f32,
    /// Current bitrate mode
    bitrate_mode: BitrateMode,
    /// Current quality mode
    quality_mode: QualityMode,
}

impl QualityAdapter {
    /// Create a new quality adapter
    pub fn new(config: AdaptationConfig) -> Self {
        Self {
            config,
            network_condition: NetworkCondition::Good,
            last_adaptation: Instant::now(),
            current_bitrate_kbps: 32, // Start with reasonable defaults
            current_buffer_ms: 50,
            redundancy_enabled: false,
            fec_level: 0.0,
            bitrate_mode: BitrateMode::Constant,
            quality_mode: QualityMode::Voice,
        }
    }
    
    /// Create a new quality adapter with default configuration
    pub fn new_default() -> Self {
        Self::new(AdaptationConfig::default())
    }
    
    /// Set the current bitrate
    pub fn set_bitrate(&mut self, bitrate_kbps: u32) {
        self.current_bitrate_kbps = bitrate_kbps.clamp(
            self.config.min_bitrate_kbps,
            self.config.max_bitrate_kbps
        );
    }
    
    /// Set the current buffer size
    pub fn set_buffer_size(&mut self, buffer_ms: u32) {
        self.current_buffer_ms = buffer_ms;
    }
    
    /// Set the current codec modes
    pub fn set_codec_modes(&mut self, bitrate_mode: BitrateMode, quality_mode: QualityMode) {
        self.bitrate_mode = bitrate_mode;
        self.quality_mode = quality_mode;
    }
    
    /// Set redundancy state
    pub fn set_redundancy(&mut self, enabled: bool) {
        self.redundancy_enabled = enabled;
    }
    
    /// Set FEC level
    pub fn set_fec_level(&mut self, level: f32) {
        self.fec_level = level.clamp(0.0, 1.0);
    }
    
    /// Adapt to quality metrics
    pub fn adapt(&mut self, metrics: &QualityMetrics, quality: &QualityScore) -> Option<AdaptationAction> {
        let now = Instant::now();
        
        // Check if we should adapt yet
        let min_interval = Duration::from_millis(self.config.min_adaptation_interval_ms as u64);
        if now.duration_since(self.last_adaptation) < min_interval {
            return None;
        }
        
        // Update network condition
        self.network_condition = NetworkCondition::from_metrics(metrics);
        
        // Determine if adaptation is needed
        if quality.mos >= self.config.target_mos && 
           self.network_condition != NetworkCondition::Poor && 
           self.network_condition != NetworkCondition::Bad {
            // Quality is good enough, no need to adapt
            return Some(AdaptationAction::NoAction);
        }
        
        // Quality is below target, adapt based on network conditions
        let action = self.adapt_to_network(self.network_condition, metrics, quality);
        
        // Update last adaptation time
        self.last_adaptation = now;
        
        // Return the action
        Some(action)
    }
    
    /// Adapt based on network conditions
    fn adapt_to_network(
        &mut self,
        condition: NetworkCondition,
        metrics: &QualityMetrics,
        quality: &QualityScore
    ) -> AdaptationAction {
        match condition {
            NetworkCondition::Excellent => {
                // Excellent conditions - can increase quality
                self.adapt_to_excellent(metrics)
            },
            NetworkCondition::Good => {
                // Good conditions - maintain or slightly improve
                self.adapt_to_good(metrics)
            },
            NetworkCondition::Fair => {
                // Fair conditions - be conservative
                self.adapt_to_fair(metrics)
            },
            NetworkCondition::Poor => {
                // Poor conditions - reduce quality to maintain stability
                self.adapt_to_poor(metrics)
            },
            NetworkCondition::Bad => {
                // Bad conditions - minimize bandwidth, maximize robustness
                self.adapt_to_bad(metrics)
            },
        }
    }
    
    /// Adapt to excellent network conditions
    fn adapt_to_excellent(&mut self, metrics: &QualityMetrics) -> AdaptationAction {
        if self.config.adapt_codec {
            // Can use higher bitrate
            let target_bitrate = self.config.max_bitrate_kbps;
            
            if target_bitrate > self.current_bitrate_kbps {
                // Increase bitrate gradually
                let new_bitrate = (self.current_bitrate_kbps as f32 * (1.0 + self.config.adaptation_rate)).min(target_bitrate as f32);
                let new_bitrate = new_bitrate.ceil() as u32;
                
                if new_bitrate != self.current_bitrate_kbps {
                    self.current_bitrate_kbps = new_bitrate;
                    info!("Adapting to excellent network: increasing bitrate to {}kbps", new_bitrate);
                    return AdaptationAction::ChangeBitrate { bitrate_kbps: new_bitrate };
                }
            }
            
            // Can use variable bitrate for better quality
            if self.bitrate_mode != BitrateMode::Variable || self.quality_mode != QualityMode::Balanced {
                self.bitrate_mode = BitrateMode::Variable;
                self.quality_mode = QualityMode::Balanced;
                return AdaptationAction::ChangeCodecMode { 
                    bitrate_mode: self.bitrate_mode,
                    quality_mode: self.quality_mode,
                };
            }
        }
        
        // Turn off redundancy if enabled
        if self.config.use_redundancy && self.redundancy_enabled {
            self.redundancy_enabled = false;
            return AdaptationAction::SetRedundancy { enabled: false };
        }
        
        // No action needed
        AdaptationAction::NoAction
    }
    
    /// Adapt to good network conditions
    fn adapt_to_good(&mut self, metrics: &QualityMetrics) -> AdaptationAction {
        if self.config.adapt_codec {
            // Target a reasonably high bitrate
            let target_bitrate = (self.config.max_bitrate_kbps * 3) / 4; // 75% of max
            
            if (target_bitrate as i32 - self.current_bitrate_kbps as i32).abs() > 8 {
                // Adjust bitrate towards target
                let direction = if target_bitrate > self.current_bitrate_kbps { 1.0 } else { -1.0 };
                let new_bitrate = (self.current_bitrate_kbps as f32 * (1.0 + direction * self.config.adaptation_rate * 0.5)).clamp(
                    self.config.min_bitrate_kbps as f32,
                    self.config.max_bitrate_kbps as f32
                );
                let new_bitrate = new_bitrate.ceil() as u32;
                
                if new_bitrate != self.current_bitrate_kbps {
                    self.current_bitrate_kbps = new_bitrate;
                    debug!("Adapting to good network: adjusting bitrate to {}kbps", new_bitrate);
                    return AdaptationAction::ChangeBitrate { bitrate_kbps: new_bitrate };
                }
            }
            
            // Variable bitrate is good for this condition
            if self.bitrate_mode != BitrateMode::Variable {
                self.bitrate_mode = BitrateMode::Variable;
                self.quality_mode = QualityMode::Voice;
                return AdaptationAction::ChangeCodecMode { 
                    bitrate_mode: self.bitrate_mode,
                    quality_mode: self.quality_mode,
                };
            }
        }
        
        // Adjust buffer size if needed
        if self.config.adapt_buffer && metrics.network.jitter_ms > 5.0 {
            let target_buffer = (metrics.network.jitter_ms * 3.0) as u32;
            if (target_buffer as i32 - self.current_buffer_ms as i32).abs() > 10 {
                let new_buffer_ms = (self.current_buffer_ms as f32 + (target_buffer as f32 - self.current_buffer_ms as f32) * self.config.adaptation_rate) as u32;
                self.current_buffer_ms = new_buffer_ms;
                return AdaptationAction::ChangeBufferSize { buffer_ms: new_buffer_ms };
            }
        }
        
        // No action needed
        AdaptationAction::NoAction
    }
    
    /// Adapt to fair network conditions
    fn adapt_to_fair(&mut self, metrics: &QualityMetrics) -> AdaptationAction {
        if self.config.adapt_codec {
            // Target a moderate bitrate
            let target_bitrate = self.config.max_bitrate_kbps / 2; // 50% of max
            
            if (target_bitrate as i32 - self.current_bitrate_kbps as i32).abs() > 8 {
                // Adjust bitrate towards target
                let direction = if target_bitrate > self.current_bitrate_kbps { 1.0 } else { -1.0 };
                let new_bitrate = (self.current_bitrate_kbps as f32 * (1.0 + direction * self.config.adaptation_rate * 0.5)).clamp(
                    self.config.min_bitrate_kbps as f32,
                    self.config.max_bitrate_kbps as f32
                );
                let new_bitrate = new_bitrate.ceil() as u32;
                
                if new_bitrate != self.current_bitrate_kbps {
                    self.current_bitrate_kbps = new_bitrate;
                    debug!("Adapting to fair network: adjusting bitrate to {}kbps", new_bitrate);
                    return AdaptationAction::ChangeBitrate { bitrate_kbps: new_bitrate };
                }
            }
            
            // Use constant bitrate for stability
            if self.bitrate_mode != BitrateMode::Constant {
                self.bitrate_mode = BitrateMode::Constant;
                self.quality_mode = QualityMode::Voice;
                return AdaptationAction::ChangeCodecMode { 
                    bitrate_mode: self.bitrate_mode,
                    quality_mode: self.quality_mode,
                };
            }
        }
        
        // Adjust buffer size
        if self.config.adapt_buffer {
            let target_buffer = (metrics.network.jitter_ms * 4.0) as u32;
            if (target_buffer as i32 - self.current_buffer_ms as i32).abs() > 10 {
                let new_buffer_ms = (self.current_buffer_ms as f32 + (target_buffer as f32 - self.current_buffer_ms as f32) * self.config.adaptation_rate) as u32;
                self.current_buffer_ms = new_buffer_ms;
                return AdaptationAction::ChangeBufferSize { buffer_ms: new_buffer_ms };
            }
        }
        
        // Enable FEC if loss is starting to occur
        if self.config.adapt_fec && metrics.network.packet_loss > 0.01 && self.fec_level < 0.5 {
            let new_fec = (metrics.network.packet_loss * 10.0).clamp(0.0, 1.0);
            self.fec_level = new_fec;
            return AdaptationAction::ChangeFec { level: new_fec };
        }
        
        // No action needed
        AdaptationAction::NoAction
    }
    
    /// Adapt to poor network conditions
    fn adapt_to_poor(&mut self, metrics: &QualityMetrics) -> AdaptationAction {
        // Enable redundancy if needed and not already enabled
        if self.config.use_redundancy && !self.redundancy_enabled && metrics.network.packet_loss > 0.05 {
            self.redundancy_enabled = true;
            return AdaptationAction::SetRedundancy { enabled: true };
        }
        
        if self.config.adapt_codec {
            // Target a low bitrate to conserve bandwidth
            let target_bitrate = self.config.min_bitrate_kbps + 
                                (self.config.max_bitrate_kbps - self.config.min_bitrate_kbps) / 4; // 25% above min
            
            if self.current_bitrate_kbps > target_bitrate {
                // Decrease bitrate to save bandwidth
                let new_bitrate = (self.current_bitrate_kbps as f32 * (1.0 - self.config.adaptation_rate)).max(target_bitrate as f32);
                let new_bitrate = new_bitrate.ceil() as u32;
                
                if new_bitrate != self.current_bitrate_kbps {
                    self.current_bitrate_kbps = new_bitrate;
                    warn!("Adapting to poor network: decreasing bitrate to {}kbps", new_bitrate);
                    return AdaptationAction::ChangeBitrate { bitrate_kbps: new_bitrate };
                }
            }
            
            // Use constant bitrate for stability
            if self.bitrate_mode != BitrateMode::Constant {
                self.bitrate_mode = BitrateMode::Constant;
                self.quality_mode = QualityMode::Voice;
                return AdaptationAction::ChangeCodecMode { 
                    bitrate_mode: self.bitrate_mode,
                    quality_mode: self.quality_mode,
                };
            }
        }
        
        // Increase buffer size
        if self.config.adapt_buffer {
            let target_buffer = (metrics.network.jitter_ms * 5.0).clamp(100.0, 300.0) as u32;
            if target_buffer > self.current_buffer_ms {
                let new_buffer_ms = (self.current_buffer_ms as f32 + (target_buffer as f32 - self.current_buffer_ms as f32) * self.config.adaptation_rate) as u32;
                self.current_buffer_ms = new_buffer_ms;
                return AdaptationAction::ChangeBufferSize { buffer_ms: new_buffer_ms };
            }
        }
        
        // Maximize FEC
        if self.config.adapt_fec && self.fec_level < 1.0 {
            self.fec_level = 1.0;
            return AdaptationAction::ChangeFec { level: 1.0 };
        }
        
        // No action needed
        AdaptationAction::NoAction
    }
    
    /// Adapt to bad network conditions
    fn adapt_to_bad(&mut self, metrics: &QualityMetrics) -> AdaptationAction {
        // Enable redundancy
        if self.config.use_redundancy && !self.redundancy_enabled {
            self.redundancy_enabled = true;
            return AdaptationAction::SetRedundancy { enabled: true };
        }
        
        if self.config.adapt_codec {
            // Use minimum bitrate
            if self.current_bitrate_kbps > self.config.min_bitrate_kbps {
                let new_bitrate = self.config.min_bitrate_kbps;
                self.current_bitrate_kbps = new_bitrate;
                warn!("Adapting to bad network: using minimum bitrate {}kbps", new_bitrate);
                return AdaptationAction::ChangeBitrate { bitrate_kbps: new_bitrate };
            }
            
            // Use constant bitrate
            if self.bitrate_mode != BitrateMode::Constant {
                self.bitrate_mode = BitrateMode::Constant;
                self.quality_mode = QualityMode::Voice;
                return AdaptationAction::ChangeCodecMode { 
                    bitrate_mode: self.bitrate_mode,
                    quality_mode: self.quality_mode,
                };
            }
        }
        
        // Maximize buffer size for stability
        if self.config.adapt_buffer {
            let target_buffer = 300; // Maximum buffer
            if target_buffer > self.current_buffer_ms {
                self.current_buffer_ms = target_buffer;
                return AdaptationAction::ChangeBufferSize { buffer_ms: target_buffer };
            }
        }
        
        // Maximize FEC
        if self.config.adapt_fec && self.fec_level < 1.0 {
            self.fec_level = 1.0;
            return AdaptationAction::ChangeFec { level: 1.0 };
        }
        
        // No action needed
        AdaptationAction::NoAction
    }
} 