//! Comprehensive integration tests for SimplePeer API with audio exchange
//! 
//! These tests verify full peer-to-peer communication including:
//! - SIP signaling between peers
//! - Audio channel establishment
//! - Bidirectional audio exchange
//! - Audio recording for verification

pub mod peer_audio_tests;
pub mod peer_call_tests;
pub mod peer_concurrent_tests;