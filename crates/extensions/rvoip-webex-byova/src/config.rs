use crate::{auth::WebexAuthenticator, session::AudioOutput, Result, WebexError};
use std::{fmt, sync::Arc, time::Duration};

/// The virtual agent this adapter exposes to Webex. Its ID is what Webex
/// selects in Flow Designer and repeats on every request.
#[derive(Clone, Debug)]
pub struct WebexVirtualAgent {
    pub id: String,
    pub name: String,
    /// Returned as the default agent when Webex asks for one.
    pub is_default: bool,
}

impl WebexVirtualAgent {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            is_default: true,
        }
    }
}

/// gRPC reply framing (`grpc` feature).
#[cfg(feature = "grpc")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum WebexGrpcReplyFraming {
    /// Cisco's published raw PCMU CHUNK profile, aggregated to one-second
    /// chunks with a 100-byte legal tail (the default).
    #[default]
    OneSecondChunks,
    /// Raw PCMU CHUNKs emitted as soon as a legal chunk is available.
    StreamingChunks,
}

/// Adapter configuration. Every bound is per adapter or per connection; no
/// queue is unbounded. Authentication is mandatory: there is no constructor
/// without a [`WebexAuthenticator`].
pub struct WebexByovaConfig {
    pub authenticator: Arc<dyn WebexAuthenticator>,
    pub virtual_agent: WebexVirtualAgent,
    /// WebSocket path for conversations. Upgrades to any other path are
    /// rejected, except [`Self::list_agents_path`].
    pub websocket_path: String,
    /// WebSocket path that lists the virtual agent.
    pub list_agents_path: String,
    /// WebSocket output profile. Raw PCMU is the documented default.
    pub audio_output: AudioOutput,
    #[cfg(feature = "grpc")]
    pub grpc_reply_framing: WebexGrpcReplyFraming,
    pub max_connections: usize,
    /// Bounds the input, command, event and media queues of each connection.
    pub queue_capacity: usize,
    pub max_frame_bytes: usize,
    /// From transport open to a valid SESSION_START.
    pub start_timeout: Duration,
    /// From publication to Orchestrator acceptance.
    pub admission_timeout: Duration,
    pub write_timeout: Duration,
    /// How long [`crate::WebexByovaCall::complete_reply`] waits for agent audio
    /// to reach its boundary before the call fails.
    pub reply_boundary_timeout: Duration,
    /// Budget a bridge's lossless media graph gives a stalled consumer.
    pub media_stall_timeout: Duration,
    /// Hard ceiling on one conversation.
    pub max_session_duration: Duration,
    /// gRPC only: maximum agent output held while no RPC is open.
    pub max_pending_output_bytes: usize,
}

impl WebexByovaConfig {
    pub fn new(
        authenticator: Arc<dyn WebexAuthenticator>,
        virtual_agent: WebexVirtualAgent,
    ) -> Self {
        Self {
            authenticator,
            virtual_agent,
            websocket_path: "/v1/va".into(),
            list_agents_path: "/v1/listVirtualAgents".into(),
            audio_output: AudioOutput::RawPcmu,
            #[cfg(feature = "grpc")]
            grpc_reply_framing: WebexGrpcReplyFraming::default(),
            max_connections: 100,
            queue_capacity: 128,
            max_frame_bytes: crate::protocol::MAX_TEXT_BYTES,
            start_timeout: Duration::from_secs(10),
            admission_timeout: Duration::from_secs(15),
            write_timeout: Duration::from_secs(5),
            reply_boundary_timeout: Duration::from_secs(5),
            media_stall_timeout: Duration::from_secs(5),
            max_session_duration: Duration::from_secs(2 * 60 * 60),
            max_pending_output_bytes: 256 * 1024,
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        let agent = &self.virtual_agent;
        if agent.id.is_empty()
            || agent.id.len() > 256
            || agent.name.is_empty()
            || agent.name.len() > 256
            || !self.websocket_path.starts_with('/')
            || !self.list_agents_path.starts_with('/')
            || self.websocket_path == self.list_agents_path
            || self.max_connections == 0
            || self.max_connections > 10_000
            || self.queue_capacity < 4
            || self.queue_capacity > 4096
            || self.max_frame_bytes < 4096
            || self.max_frame_bytes > 1024 * 1024
            || self.max_pending_output_bytes < 64 * 1024
            || self.max_pending_output_bytes > 16 * 1024 * 1024
            || [
                self.start_timeout,
                self.admission_timeout,
                self.write_timeout,
                self.reply_boundary_timeout,
                self.media_stall_timeout,
                self.max_session_duration,
            ]
            .iter()
            .any(Duration::is_zero)
        {
            return Err(WebexError::Configuration);
        }
        Ok(())
    }
}

impl fmt::Debug for WebexByovaConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebexByovaConfig")
            .field("virtual_agent", &self.virtual_agent)
            .field("websocket_path", &self.websocket_path)
            .field("audio_output", &self.audio_output)
            .field("max_connections", &self.max_connections)
            .field("queue_capacity", &self.queue_capacity)
            .finish_non_exhaustive()
    }
}
