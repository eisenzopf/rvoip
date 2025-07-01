//! Configuration utilities for RVOIP Simple

// Re-export main config types from lib.rs
pub use crate::{ClientConfig, SecurityConfig, MediaConfig, AudioCodec, VideoCodec, AudioQuality};

impl SecurityConfig {
    /// Create a configuration for WebRTC applications
    pub fn webrtc() -> Self {
        Self::DtlsSrtp
    }

    /// Create a configuration for SIP applications
    pub fn sip() -> Self {
        Self::Auto
    }

    /// Create a configuration for peer-to-peer calling
    pub fn p2p() -> Self {
        Self::Zrtp
    }

    /// Create an enterprise configuration with pre-shared key
    pub fn enterprise_psk(key: Vec<u8>) -> Self {
        Self::MikeyPsk { key }
    }

    /// Create an enterprise configuration with certificates
    pub fn enterprise_pke(certificate: Vec<u8>, private_key: Vec<u8>) -> Self {
        Self::MikeyPke {
            certificate,
            private_key,
            peer_certificate: None,
        }
    }
}

impl MediaConfig {
    /// Create a configuration optimized for mobile devices
    pub fn mobile() -> Self {
        Self {
            audio_codecs: vec![AudioCodec::Opus, AudioCodec::G722],
            video_codecs: vec![VideoCodec::H264, VideoCodec::VP8],
            echo_cancellation: true,
            noise_suppression: true,
            auto_gain_control: true,
            audio_quality: AudioQuality::Bandwidth,
        }
    }

    /// Create a configuration optimized for desktop applications
    pub fn desktop() -> Self {
        Self {
            audio_codecs: vec![AudioCodec::Opus, AudioCodec::G722, AudioCodec::G711u],
            video_codecs: vec![VideoCodec::H264, VideoCodec::VP8, VideoCodec::VP9],
            echo_cancellation: true,
            noise_suppression: true,
            auto_gain_control: true,
            audio_quality: AudioQuality::Quality,
        }
    }

    /// Create a configuration for voice-only applications
    pub fn voice_only() -> Self {
        Self {
            audio_codecs: vec![AudioCodec::Opus, AudioCodec::G722, AudioCodec::G711u, AudioCodec::G711a],
            video_codecs: vec![], // No video
            echo_cancellation: true,
            noise_suppression: true,
            auto_gain_control: true,
            audio_quality: AudioQuality::Quality,
        }
    }

    /// Create a configuration for conferencing applications
    pub fn conferencing() -> Self {
        Self {
            audio_codecs: vec![AudioCodec::Opus, AudioCodec::G722],
            video_codecs: vec![VideoCodec::H264, VideoCodec::VP8, VideoCodec::VP9],
            echo_cancellation: true,
            noise_suppression: true,
            auto_gain_control: true,
            audio_quality: AudioQuality::Balanced,
        }
    }

    /// Create a low-bandwidth configuration
    pub fn low_bandwidth() -> Self {
        Self {
            audio_codecs: vec![AudioCodec::G711u, AudioCodec::G711a],
            video_codecs: vec![VideoCodec::H264], // Most efficient
            echo_cancellation: false, // Reduce processing
            noise_suppression: false,
            auto_gain_control: false,
            audio_quality: AudioQuality::Bandwidth,
        }
    }

    /// Create a high-quality configuration
    pub fn high_quality() -> Self {
        Self {
            audio_codecs: vec![AudioCodec::Opus], // Best quality
            video_codecs: vec![VideoCodec::H264, VideoCodec::VP9, VideoCodec::AV1],
            echo_cancellation: true,
            noise_suppression: true,
            auto_gain_control: true,
            audio_quality: AudioQuality::Quality,
        }
    }
} 