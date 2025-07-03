//! Media Transport API
//!
//! This module provides the high-level API for media transport, including both client and server interfaces.
//! It abstracts the underlying RTP/RTCP implementation details and provides a simpler interface for applications.

pub mod common;
pub mod client;
pub mod server;

// Re-export common types
pub use common::frame::{MediaFrame, MediaFrameType};
pub use common::error::MediaTransportError;
pub use common::events::{MediaTransportEvent, MediaEventCallback};
pub use common::config::{SecurityMode, SecurityInfo, IdentityValidation};
pub use common::stats::{MediaStats, QualityLevel, StreamStats, StatsFactory};

// Re-export client types
pub use client::transport::MediaTransportClient;
pub use client::config::{ClientConfig, ClientConfigBuilder};
pub use client::ClientFactory;
pub use client::transport::DefaultMediaTransportClient;

// Re-export server types
pub use server::transport::MediaTransportServer;
pub use server::transport::ClientInfo;
pub use server::config::{ServerConfig, ServerConfigBuilder};
pub use server::ServerFactory;
pub use server::transport::DefaultMediaTransportServer;

// Re-export client security
pub use client::security::ClientSecurityContext;
pub use client::security::ClientSecurityConfig;
pub use client::security::DefaultClientSecurityContext;

// Re-export server security
pub use server::security::ServerSecurityContext;
pub use server::security::ServerSecurityConfig;
pub use server::security::DefaultServerSecurityContext;

/// Creates a client for the given configuration
pub async fn create_client(config: ClientConfig) -> Result<client::transport::DefaultMediaTransportClient, MediaTransportError> {
    client::ClientFactory::create_client(config).await
}

/// Creates a server for the given configuration
pub async fn create_server(config: ServerConfig) -> Result<server::transport::DefaultMediaTransportServer, MediaTransportError> {
    server::ServerFactory::create_server(config).await
}

/// Creates a WebRTC client for the given remote address
pub async fn create_webrtc_client(
    remote_addr: std::net::SocketAddr
) -> Result<client::transport::DefaultMediaTransportClient, MediaTransportError> {
    client::ClientFactory::create_webrtc_client(remote_addr).await
}

/// Creates a WebRTC server for the given local address
pub async fn create_webrtc_server(
    local_addr: std::net::SocketAddr
) -> Result<server::transport::DefaultMediaTransportServer, MediaTransportError> {
    server::ServerFactory::create_webrtc_server(local_addr).await
} 