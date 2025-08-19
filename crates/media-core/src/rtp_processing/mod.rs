//! RTP media processing (moved from rtp-core)
//!
//! This module contains RTP media processing functionality that was moved
//! from rtp-core as part of the Transport/Media plane separation.
//! 
//! rtp-core now handles pure transport (packets, encryption, sockets)
//! while media-core handles media processing (payloads, jitter, quality)

pub mod payload;
pub mod jitter;
#[path = "quality_mod.rs"]
pub mod quality;
pub mod buffer;
pub mod session;
pub mod media;
pub mod codec;
pub mod rtcp;

// Re-export actual implemented types
pub use payload::{PayloadFormat, PayloadFormatFactory};
pub use jitter::{JitterBuffer, JitterBufferConfig, JitterBufferStats};
pub use quality::{MediaQualityMonitor, MediaQualityLevel, MediaQualityMetrics, MediaStreamStats, MediaSessionStats};
pub use buffer::{MediaBufferPool, PooledMediaBuffer, MediaBufferPoolStats};
pub use session::{MediaSession, MediaSessionConfig, MediaSessionState};
pub use media::{
    MediaMixer, mix_audio_frames, mix_active_speakers, calculate_audio_level, detect_voice_activity,
    CsrcMapping, CsrcManager, MediaCsrcService, RtpSsrc, RtpCsrc,
    HeaderExtension, ExtensionFormat, HeaderExtensionManager, MediaHeaderExtensionService,
};
pub use codec::{
    PayloadTypeInfo, PayloadTypeRegistry, get_global_registry, get_media_frame_type, get_codec_name, get_payload_info,
    CodecCapability, CodecNegotiator, NegotiationPreferences, NegotiationResult,
    SdpMediaLine, SdpAttribute, SdpFormatParameter, SdpMediaDescription, SdpRtpMap, SdpMediaProcessor,
};
pub use rtcp::{
    FeedbackPacketType, PayloadFeedbackFormat, TransportCcFormat, FeedbackPriority, 
    FeedbackContext, CongestionState, FeedbackConfig, QualityDegradation, FeedbackDecision,
    FeedbackGenerator, FeedbackGeneratorFactory, StreamStats, StreamDirection,
    LossFeedbackGenerator, CongestionFeedbackGenerator, QualityFeedbackGenerator, ComprehensiveFeedbackGenerator,
    RtcpFeedbackHeader, PliPacket, FirPacket, RembPacket, NackPacket,
    GoogleCongestionControl, GccState, TransportCcProcessor, TransportCcFeedback,
    calculate_mos_from_rfactor, calculate_rfactor,
};