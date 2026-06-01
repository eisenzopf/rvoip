//! RTP media processing (moved from rtp-core)
//!
//! This module contains RTP media processing functionality that was moved
//! from rtp-core as part of the Transport/Media plane separation.
//!
//! rtp-core now handles pure transport (packets, encryption, sockets)
//! while media-core handles media processing (payloads, jitter, quality)

pub mod buffer;
pub mod codec;
pub mod jitter;
pub mod media;
pub mod payload;
#[path = "quality_mod.rs"]
pub mod quality;
pub mod rtcp;
pub mod session;

// Re-export actual implemented types
pub use buffer::{MediaBufferPool, MediaBufferPoolStats, PooledMediaBuffer};
pub use codec::{
    get_codec_name, get_global_registry, get_media_frame_type, get_payload_info, CodecCapability,
    CodecNegotiator, NegotiationPreferences, NegotiationResult, PayloadTypeInfo,
    PayloadTypeRegistry, SdpAttribute, SdpFormatParameter, SdpMediaDescription, SdpMediaLine,
    SdpMediaProcessor, SdpRtpMap,
};
pub use jitter::{JitterBuffer, JitterBufferConfig, JitterBufferStats};
pub use media::{
    calculate_audio_level, detect_voice_activity, mix_active_speakers, mix_audio_frames,
    CsrcManager, CsrcMapping, ExtensionFormat, HeaderExtension, HeaderExtensionManager,
    MediaCsrcService, MediaHeaderExtensionService, MediaMixer, RtpCsrc, RtpSsrc,
};
pub use payload::{PayloadFormat, PayloadFormatFactory};
pub use quality::{
    MediaQualityLevel, MediaQualityMetrics, MediaQualityMonitor, MediaSessionStats,
    MediaStreamStats,
};
pub use rtcp::{
    calculate_mos_from_rfactor, calculate_rfactor, ComprehensiveFeedbackGenerator,
    CongestionFeedbackGenerator, CongestionState, FeedbackConfig, FeedbackContext,
    FeedbackDecision, FeedbackGenerator, FeedbackGeneratorFactory, FeedbackPacketType,
    FeedbackPriority, FirPacket, GccState, GoogleCongestionControl, LossFeedbackGenerator,
    NackPacket, PayloadFeedbackFormat, PliPacket, QualityDegradation, QualityFeedbackGenerator,
    RembPacket, RtcpFeedbackHeader, StreamDirection, StreamStats, TransportCcFeedback,
    TransportCcFormat, TransportCcProcessor,
};
pub use session::{MediaSession, MediaSessionConfig, MediaSessionState};
