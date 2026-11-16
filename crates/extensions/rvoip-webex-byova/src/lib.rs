//! Webex Contact Center BYOVA (Bring Your Own Virtual Agent) adapter.
#![doc = include_str!("../README.md")]

mod adapter;
pub mod auth;
mod config;
mod error;
#[cfg(feature = "grpc")]
pub mod grpc;
#[cfg(feature = "grpc")]
mod grpc_server;
mod media;
pub mod protocol;
pub mod session;
#[cfg(feature = "wav-compat")]
pub mod wav;
mod websocket;

pub use adapter::{
    ReplyAction, ReplyBoundary, ReplyMark, ReplyMarks, WebexByovaAdapter, WebexByovaCall,
    WebexInput, WebexOutputEvent,
};
pub use auth::{
    WebexAuthenticator, WebexClaimNames, WebexJwsConfig, WebexJwsValidator, WebexPrincipal,
    WebexRegistration, WebexTransportKind,
};
#[cfg(feature = "grpc")]
pub use config::WebexGrpcReplyFraming;
pub use config::{WebexByovaConfig, WebexVirtualAgent};
pub use error::{Result, WebexError};
#[cfg(feature = "grpc")]
pub use grpc_server::WebexByovaGrpcService;
pub use protocol::{Input, ProtocolError, MAX_AUDIO_BYTES, MAX_TEXT_BYTES};
pub use session::{AudioOutput, ReplyId, Session};
pub use websocket::WebexWebSocket;
