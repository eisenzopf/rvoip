use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CodecInfo {
    pub name: String,
    pub clock_rate_hz: u32,
    pub channels: u8,
    pub fmtp: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CapabilityDescriptor {
    pub audio_codecs: Vec<CodecInfo>,
    pub video_codecs: Vec<CodecInfo>,
    pub supports_dtmf_rfc4733: bool,
    pub supports_message_text: bool,
    pub supports_srtp: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CapabilityIntersection {
    pub audio: Option<CodecInfo>,
    pub video: Option<CodecInfo>,
    pub dtmf_method: Option<DtmfMethod>,
    pub messaging_enabled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DtmfMethod {
    Rfc4733,
    SipInfo,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NegotiatedCodecs {
    pub audio: Option<CodecInfo>,
    pub video: Option<CodecInfo>,
}
