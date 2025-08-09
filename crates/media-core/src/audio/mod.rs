//! Audio processing and utilities module
//!
//! This module provides audio-related functionality including:
//! - WAV file loading and conversion for music-on-hold
//! - Audio format utilities

pub mod wav_loader;

pub use wav_loader::{
    WavAudio,
    load_wav_file,
    wav_to_ulaw,
    load_music_on_hold,
    MusicOnHoldCache,
};