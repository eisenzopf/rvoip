//! Amazon Connect A2A external-agent adapter.
#![doc = include_str!("../README.md")]

mod adapter;
mod config;
mod error;
mod fragment;
mod media;
pub mod protocol;
mod session;
pub mod trace;
mod websocket;

pub use adapter::{AmazonA2aAdapter, AmazonA2aCall, CallBinding, InputEvent};
pub use config::{AmazonA2aConfig, ApiKey};
pub use error::{A2aError, Result};
pub use protocol::{AudioConfiguration, FinishType, InitSession, InterruptionType};
pub use trace::{TraceContext, TurnTrace};
