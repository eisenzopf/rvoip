//! Vapi bidirectional WebSocket voice-agent adapter for rvoip.
//!
//! This crate lets rvoip retain ownership of a SIP or WebRTC caller leg while
//! Vapi owns ASR, dialog, synthesis, and interruption handling on a raw-audio
//! WebSocket call transport.

#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]
#![doc = include_str!("../README.md")]

mod client;
mod media;

pub mod adapter;
pub mod agent;
pub mod config;
pub mod error;
pub mod events;
pub mod types;

pub use adapter::{
    VapiAdapter, VapiTransportHandle, ADAPTER_EVENT_CAPACITY, VAPI_CALL_REFERENCE_KIND,
};
pub use agent::{VapiAgentCall, VapiAgentOutcome};
pub use config::{VapiApiKey, VapiConfig};
pub use error::{Result, VapiError};
pub use events::{VapiEvent, VapiEventEnvelope};
pub use types::{VapiAssistant, VapiAudioFormat, VapiCallOptions, VapiPeerFailurePolicy};
