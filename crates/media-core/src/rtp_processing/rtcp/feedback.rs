//! RTCP Feedback Mechanisms (moved from rtp-core)
//!
//! This module implements advanced RTCP feedback packets for real-time media quality adaptation,
//! including Picture Loss Indication (PLI), Full Intra Request (FIR), Slice Loss Indication (SLI),
//! Temporal-Spatial Trade-off (TSTO), Receiver Estimated Max Bitrate (REMB), and 
//! Transport-wide Congestion Control feedback.

use std::time::Instant;
use crate::api::error::MediaError;

/// RTP SSRC type
pub type RtpSsrc = u32;

/// Feedback packet types as defined in RFC 4585 and extensions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FeedbackPacketType {
    /// Generic NACK (RFC 4585)
    GenericNack = 205,
    
    /// Payload-specific feedback (RFC 4585)
    PayloadSpecificFeedback = 206,
}

/// Payload-specific feedback message types (FMT field)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PayloadFeedbackFormat {
    /// Picture Loss Indication (PLI) - RFC 4585
    PictureLossIndication = 1,
    
    /// Slice Loss Indication (SLI) - RFC 4585
    SliceLossIndication = 2,
    
    /// Reference Picture Selection Indication (RPSI) - RFC 4585
    ReferencePictureSelectionIndication = 3,
    
    /// Full Intra Request (FIR) - RFC 5104
    FullIntraRequest = 4,
    
    /// Temporal-Spatial Trade-off (TSTO) - RFC 5104
    TemporalSpatialTradeoff = 5,
    
    /// Temporal-Spatial Trade-off Notification (TSTN) - RFC 5104
    TemporalSpatialTradeoffNotification = 6,
    
    /// Video Back Channel Message (VBCM) - RFC 5104
    VideoBackChannelMessage = 7,
    
    /// Application Layer Feedback (ALF) - RFC 5104
    ApplicationLayerFeedback = 15,
}

/// Transport-wide Congestion Control feedback types (WebRTC extension)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TransportCcFormat {
    /// Transport-wide CC feedback
    TransportCcFeedback = 15,
}

/// Feedback message priority levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FeedbackPriority {
    /// Low priority - routine feedback
    Low = 0,
    
    /// Normal priority - standard feedback
    Normal = 1,
    
    /// High priority - quality impacting
    High = 2,
    
    /// Critical priority - immediate action required
    Critical = 3,
}

/// Feedback generation context
#[derive(Debug, Clone)]
pub struct FeedbackContext {
    /// Local SSRC
    pub local_ssrc: RtpSsrc,
    
    /// Remote SSRC
    pub media_ssrc: RtpSsrc,
    
    /// Last feedback generation time
    pub last_feedback: Option<Instant>,
    
    /// Feedback generation statistics
    pub feedback_count: u32,
    
    /// Current congestion state
    pub congestion_state: CongestionState,
}

/// Network congestion state tracking
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CongestionState {
    /// No congestion detected
    None,
    
    /// Light congestion - minor packet loss
    Light,
    
    /// Moderate congestion - noticeable quality impact
    Moderate,
    
    /// Severe congestion - significant quality degradation
    Severe,
    
    /// Critical congestion - emergency measures needed
    Critical,
}

/// Feedback generation configuration
#[derive(Debug, Clone)]
pub struct FeedbackConfig {
    /// Enable Picture Loss Indication
    pub enable_pli: bool,
    
    /// Enable Full Intra Request
    pub enable_fir: bool,
    
    /// Enable Slice Loss Indication
    pub enable_sli: bool,
    
    /// Enable REMB (bandwidth estimation)
    pub enable_remb: bool,
    
    /// Enable Transport-wide Congestion Control
    pub enable_transport_cc: bool,
    
    /// Minimum interval between PLI packets (milliseconds)
    pub pli_interval_ms: u32,
    
    /// Minimum interval between FIR packets (milliseconds)
    pub fir_interval_ms: u32,
    
    /// Maximum feedback rate (packets per second)
    pub max_feedback_rate: u32,
    
    /// Congestion detection sensitivity (0.0 - 1.0)
    pub congestion_sensitivity: f32,
}

impl Default for FeedbackConfig {
    fn default() -> Self {
        Self {
            enable_pli: true,
            enable_fir: true,
            enable_sli: false,  // Less commonly used
            enable_remb: true,
            enable_transport_cc: true,
            pli_interval_ms: 500,   // 500ms minimum between PLI
            fir_interval_ms: 2000,  // 2s minimum between FIR
            max_feedback_rate: 10,  // 10 feedback packets per second max
            congestion_sensitivity: 0.7,  // Moderate sensitivity
        }
    }
}

/// Quality degradation types for feedback decisions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityDegradation {
    /// Packet loss detected
    PacketLoss {
        /// Loss rate (0.0 - 1.0)
        rate: u8,  // Stored as percentage (0-100)
        
        /// Consecutive packets lost
        consecutive: u16,
    },
    
    /// High jitter detected
    HighJitter {
        /// Current jitter (in timestamp units)
        jitter: u32,
        
        /// Jitter threshold exceeded
        threshold_exceeded: bool,
    },
    
    /// Bandwidth limitation detected
    BandwidthLimited {
        /// Available bandwidth (bits per second)
        available_bps: u32,
        
        /// Required bandwidth (bits per second)
        required_bps: u32,
    },
    
    /// Frame corruption detected
    FrameCorruption {
        /// Number of corrupted frames
        count: u16,
        
        /// Corruption type indicator
        corruption_type: u8,
    },
}

/// Feedback generation result
#[derive(Debug, Clone)]
pub enum FeedbackDecision {
    /// No feedback needed
    None,
    
    /// Generate Picture Loss Indication
    Pli {
        priority: FeedbackPriority,
        reason: QualityDegradation,
    },
    
    /// Generate Full Intra Request
    Fir {
        priority: FeedbackPriority,
        sequence_number: u8,
    },
    
    /// Generate REMB (bitrate recommendation)
    Remb {
        bitrate_bps: u32,
        confidence: f32,
    },
    
    /// Generate multiple feedback messages
    Multiple(Vec<FeedbackDecision>),
}

impl FeedbackContext {
    /// Create a new feedback context
    pub fn new(local_ssrc: RtpSsrc, media_ssrc: RtpSsrc) -> Self {
        Self {
            local_ssrc,
            media_ssrc,
            last_feedback: None,
            feedback_count: 0,
            congestion_state: CongestionState::None,
        }
    }
    
    /// Update congestion state based on network conditions
    pub fn update_congestion_state(&mut self, loss_rate: f32, rtt_ms: u32, jitter: u32) {
        // Simple congestion detection algorithm
        let congestion_score = self.calculate_congestion_score(loss_rate, rtt_ms, jitter);
        
        self.congestion_state = match congestion_score {
            score if score < 0.1 => CongestionState::None,
            score if score < 0.3 => CongestionState::Light,
            score if score < 0.6 => CongestionState::Moderate,
            score if score < 0.8 => CongestionState::Severe,
            _ => CongestionState::Critical,
        };
    }
    
    /// Calculate a congestion score (0.0 - 1.0) based on network metrics
    fn calculate_congestion_score(&self, loss_rate: f32, rtt_ms: u32, jitter: u32) -> f32 {
        // Weight factors for different metrics
        let loss_weight = 0.5;
        let rtt_weight = 0.3;
        let jitter_weight = 0.2;
        
        // Normalize metrics to 0.0 - 1.0 range
        let normalized_loss = (loss_rate * 100.0).min(20.0) / 20.0;  // 20% loss = max score
        let normalized_rtt = (rtt_ms as f32).min(1000.0) / 1000.0;   // 1000ms RTT = max score
        let normalized_jitter = (jitter as f32).min(100.0) / 100.0;  // 100 jitter units = max score
        
        // Weighted sum
        (normalized_loss * loss_weight + 
         normalized_rtt * rtt_weight + 
         normalized_jitter * jitter_weight).min(1.0)
    }
    
    /// Record feedback generation
    pub fn record_feedback(&mut self) {
        self.last_feedback = Some(Instant::now());
        self.feedback_count += 1;
    }
    
    /// Check if enough time has passed for the next feedback
    pub fn can_send_feedback(&self, interval_ms: u32) -> bool {
        match self.last_feedback {
            None => true,
            Some(last) => last.elapsed().as_millis() >= interval_ms as u128,
        }
    }
}

/// Stream statistics for feedback generation
#[derive(Debug, Clone)]
pub struct StreamStats {
    /// Stream direction
    pub direction: StreamDirection,
    /// Stream SSRC
    pub ssrc: u32,
    /// Media type
    pub media_type: MediaFrameType,
    /// Total packets processed
    pub packet_count: u64,
    /// Total bytes processed
    pub byte_count: u64,
    /// Packets lost
    pub packets_lost: u32,
    /// Fraction lost (0.0-1.0)
    pub fraction_lost: f32,
    /// Jitter in milliseconds
    pub jitter_ms: f32,
    /// Round-trip time
    pub rtt_ms: Option<f32>,
    /// Current bitrate in bps
    pub bitrate_bps: u32,
    /// Discard rate
    pub discard_rate: f32,
    /// Remote address
    pub remote_addr: std::net::SocketAddr,
}

impl Default for StreamStats {
    fn default() -> Self {
        Self {
            direction: StreamDirection::default(),
            ssrc: 0,
            media_type: MediaFrameType::default(),
            packet_count: 0,
            byte_count: 0,
            packets_lost: 0,
            fraction_lost: 0.0,
            jitter_ms: 0.0,
            rtt_ms: None,
            bitrate_bps: 0,
            discard_rate: 0.0,
            remote_addr: "0.0.0.0:0".parse().unwrap(),
        }
    }
}

/// Stream direction for statistics
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StreamDirection {
    #[default]
    Inbound,
    Outbound,
}

/// Media frame type for statistics
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MediaFrameType {
    #[default]
    Audio,
    Video,
    Data,
}

/// Trait for feedback packet generation
pub trait FeedbackGenerator {
    /// Generate feedback decision based on current conditions
    fn generate_feedback(&self, context: &FeedbackContext, config: &FeedbackConfig) -> Result<FeedbackDecision, MediaError>;
    
    /// Update internal state with new statistics
    fn update_statistics(&mut self, stats: &StreamStats);
    
    /// Get the feedback generator name
    fn name(&self) -> &'static str;
}

/// Factory for creating feedback generators
pub struct FeedbackGeneratorFactory;

impl FeedbackGeneratorFactory {
    /// Create a loss-based feedback generator
    pub fn create_loss_generator() -> Box<dyn FeedbackGenerator> {
        Box::new(crate::rtp_processing::rtcp::generators::LossFeedbackGenerator::new())
    }
    
    /// Create a congestion-based feedback generator
    pub fn create_congestion_generator() -> Box<dyn FeedbackGenerator> {
        Box::new(crate::rtp_processing::rtcp::generators::CongestionFeedbackGenerator::new())
    }
    
    /// Create a quality-based feedback generator
    pub fn create_quality_generator() -> Box<dyn FeedbackGenerator> {
        Box::new(crate::rtp_processing::rtcp::generators::QualityFeedbackGenerator::new())
    }
    
    /// Create a comprehensive feedback generator (combines all strategies)
    pub fn create_comprehensive_generator() -> Box<dyn FeedbackGenerator> {
        Box::new(crate::rtp_processing::rtcp::generators::ComprehensiveFeedbackGenerator::new())
    }
}