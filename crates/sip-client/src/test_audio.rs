//! Test audio backend for integration testing
//! 
//! This module provides memory-based audio devices that can be used
//! to test two SIP clients on the same machine without requiring
//! actual audio hardware.

use std::collections::{VecDeque, HashMap};
use std::sync::Arc;
use tokio::sync::Mutex;
use rvoip_audio_core::{
    AudioDevice, AudioDeviceInfo, AudioDirection, AudioFormat, 
    AudioFrame, AudioError, AudioResult
};

/// Shared audio buffers for test clients
/// Each client has separate input/output buffers to simulate hardware devices
#[derive(Debug)]
pub struct TestAudioBuffers {
    /// Client A's input buffer (microphone)
    pub a_input: Arc<Mutex<VecDeque<AudioFrame>>>,
    /// Client A's output buffer (speakers)
    pub a_output: Arc<Mutex<VecDeque<AudioFrame>>>,
    /// Client B's input buffer (microphone)
    pub b_input: Arc<Mutex<VecDeque<AudioFrame>>>,
    /// Client B's output buffer (speakers)
    pub b_output: Arc<Mutex<VecDeque<AudioFrame>>>,
}

impl TestAudioBuffers {
    /// Create new test audio buffers
    pub fn new() -> Self {
        Self {
            a_input: Arc::new(Mutex::new(VecDeque::new())),
            a_output: Arc::new(Mutex::new(VecDeque::new())),
            b_input: Arc::new(Mutex::new(VecDeque::new())),
            b_output: Arc::new(Mutex::new(VecDeque::new())),
        }
    }
}

// Note: We use the test audio devices from audio-core directly
// The TestAudioBuffers above provides the memory buffers for testing