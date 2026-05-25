#[cfg(feature = "client")]
pub mod comprehensive;
#[cfg(feature = "client")]
pub mod media_source;
#[cfg(feature = "client")]
pub mod native;
#[cfg(feature = "client")]
pub mod perfect_negotiation;
#[cfg(feature = "client")]
pub mod pool;
#[cfg(all(feature = "client", feature = "signaling-ws"))]
pub mod ws_signaler;

// D3a — cpal-backed microphone + speaker. Optional dep; opt in via the
// `client-cpal` feature.
#[cfg(feature = "client-cpal")]
pub mod audio_cpal;

// D3 — `VideoSource` / `VideoSink` trait skeleton plus RFC 7741/6184
// packetizers (pure Rust). Concrete encoders are opt-in via
// `client-video-vp8` / `client-video-h264`.
#[cfg(feature = "client-video")]
pub mod video;

// D3b — VP8 encoder via vpx-encode + RFC 7741 packetizer.
#[cfg(feature = "client-video-vp8")]
pub mod video_vp8;

// D3c — H.264 encoder via openh264 + RFC 6184 packetizer.
#[cfg(feature = "client-video-h264")]
pub mod video_h264;

#[cfg(feature = "client")]
pub use comprehensive::ComprehensiveReport;
#[cfg(feature = "client")]
pub use media_source::{
    run_audio, AudioPacing, AudioSink, AudioSource, CountingAudioSink, FixtureAudioSource,
    NullAudioSink,
};
#[cfg(feature = "client")]
pub use native::{
    Answer, CallTarget, IceCandidate, Offer, SessionHandle, SessionMedium, Signaler, WebRtcClient,
};
#[cfg(feature = "client")]
pub use perfect_negotiation::{NegotiationAction, PerfectNegotiation};
#[cfg(feature = "client")]
pub use pool::SignalingPool;
#[cfg(all(feature = "client", feature = "signaling-ws"))]
pub use ws_signaler::{send_ice_for, WsSignaler, WsSignalerConfig};

#[cfg(feature = "client-cpal")]
pub use audio_cpal::{CpalAudioConfig, CpalAudioSource, CpalSpeakerSink};

#[cfg(feature = "client-video")]
pub use video::{
    NullVideoSink, NullVideoSource, VideoCodec, VideoFrame, VideoSink, VideoSource, YuvFrame,
};

#[cfg(feature = "client-video-vp8")]
pub use video_vp8::{Vp8Settings, Vp8VideoSource};

#[cfg(feature = "client-video-h264")]
pub use video_h264::{H264Settings, H264VideoSource};
