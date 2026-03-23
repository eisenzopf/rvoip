// Media module

//! Media operations for the client-core library
//! 
//! This module contains all media-related operations including mute/unmute,
//! audio transmission, codec management, SDP handling, and media session lifecycle.
//! 
//! The operations are organized into sub-modules by functional area:
//! 
//! - **`mute_codec`** - Mute controls (microphone/speaker) and codec management
//! - **`transmission`** - Audio transmission start/stop, custom audio, tone generation
//! - **`session`** - Media session lifecycle (create/start/stop/update) and capabilities
//! - **`sdp_stats`** - SDP offer/answer generation, statistics, and audio streaming

mod mute_codec;
mod transmission;
mod session;
mod sdp_stats;
