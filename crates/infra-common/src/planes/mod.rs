//! Federated Plane Architecture
//!
//! This module provides the core abstractions for RVOIP's three-plane architecture:
//! - Transport Plane: Network transport (SIP/RTP)
//! - Media Plane: Media processing (codecs, mixing)
//! - Signaling Plane: Call control and session management
//!
//! The architecture supports both monolithic (all planes in-process) and
//! distributed (planes as separate services) deployments.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::fmt::Debug;
use anyhow::Result;

pub mod deployment;
pub mod routing;
pub mod task_management;

pub use deployment::{DeploymentMode, PlaneConfig};
pub use routing::{PlaneRouter, EventAffinity, PlaneType};
pub use task_management::{LayerTaskManager, TaskHandle, TaskPriority};

/// Core trait for all federated planes
#[async_trait]
pub trait FederatedPlane: Send + Sync + Debug {
    /// Get the plane type
    fn plane_type(&self) -> PlaneType;
    
    /// Get the plane's unique identifier
    fn plane_id(&self) -> &str;
    
    /// Check if the plane is healthy
    async fn health_check(&self) -> Result<PlaneHealth>;
    
    /// Start the plane
    async fn start(&self) -> Result<()>;
    
    /// Stop the plane gracefully
    async fn stop(&self) -> Result<()>;
    
    /// Get plane metrics
    async fn metrics(&self) -> Result<PlaneMetrics>;
}

/// Health status of a plane
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaneHealth {
    pub status: HealthStatus,
    pub latency_ms: u64,
    pub active_connections: usize,
    pub error_rate: f64,
    #[serde(skip, default = "std::time::Instant::now")]
    pub last_check: std::time::Instant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
    Unknown,
}

/// Metrics for monitoring plane performance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaneMetrics {
    pub requests_per_second: f64,
    pub average_latency_ms: f64,
    pub error_count: u64,
    pub active_sessions: usize,
    pub memory_usage_mb: f64,
    pub cpu_usage_percent: f64,
}

/// Transport Plane abstraction
#[async_trait]
pub trait TransportPlane: FederatedPlane {
    /// Send a SIP message
    async fn send_sip_message(&self, message: Vec<u8>, destination: &str) -> Result<()>;
    
    /// Send RTP packet
    async fn send_rtp_packet(&self, packet: Vec<u8>, session_id: &str) -> Result<()>;
    
    /// Register SIP endpoint
    async fn register_endpoint(&self, uri: &str, transport: &str) -> Result<()>;
    
    /// Get transport statistics
    async fn transport_stats(&self) -> Result<TransportStats>;
}

/// Media Plane abstraction
#[async_trait]
pub trait MediaPlane: FederatedPlane {
    /// Start media stream
    async fn start_media_stream(&self, session_id: &str, config: MediaConfig) -> Result<()>;
    
    /// Stop media stream
    async fn stop_media_stream(&self, session_id: &str) -> Result<()>;
    
    /// Process audio frame
    async fn process_audio_frame(&self, session_id: &str, frame: AudioFrame) -> Result<()>;
    
    /// Get media quality metrics
    async fn media_quality(&self, session_id: &str) -> Result<MediaQuality>;
    
    /// Mix conference streams
    async fn mix_conference(&self, conference_id: &str, participant_ids: Vec<String>) -> Result<()>;
}

/// Signaling Plane abstraction
#[async_trait]
pub trait SignalingPlane: FederatedPlane {
    /// Create new session
    async fn create_session(&self, from: &str, to: &str) -> Result<String>;
    
    /// Terminate session
    async fn terminate_session(&self, session_id: &str) -> Result<()>;
    
    /// Update session state
    async fn update_session_state(&self, session_id: &str, state: SessionState) -> Result<()>;
    
    /// Handle incoming call
    async fn handle_incoming_call(&self, call_info: IncomingCallInfo) -> Result<CallDecision>;
    
    /// Get session information
    async fn get_session(&self, session_id: &str) -> Result<SessionInfo>;
}

// Supporting types

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportStats {
    pub packets_sent: u64,
    pub packets_received: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub packet_loss_rate: f64,
    pub jitter_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaConfig {
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u8,
    pub bitrate: u32,
    pub enable_dtx: bool,
    pub enable_fec: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioFrame {
    pub timestamp: u64,
    pub data: Vec<u8>,
    pub sample_rate: u32,
    pub channels: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaQuality {
    pub mos_score: f64, // Mean Opinion Score (1-5)
    pub packet_loss: f64,
    pub jitter_ms: f64,
    pub delay_ms: u64,
    pub echo_return_loss: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionState {
    Initiating,
    Ringing,
    Active,
    OnHold,
    Transferring,
    Terminating,
    Terminated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomingCallInfo {
    pub from: String,
    pub to: String,
    pub call_id: String,
    pub sdp: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CallDecision {
    Accept(Option<String>), // Optional SDP answer
    Reject(String),         // Reason
    Forward(String),        // Forward destination
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub from: String,
    pub to: String,
    pub state: SessionState,
    #[serde(skip)]
    pub start_time: Option<std::time::Instant>,
    pub media_info: Option<MediaInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaInfo {
    pub local_sdp: Option<String>,
    pub remote_sdp: Option<String>,
    pub codec: String,
    pub local_port: u16,
    pub remote_port: u16,
}

/// Plane factory for creating plane instances
pub struct PlaneFactory;

impl PlaneFactory {
    /// Create a plane based on deployment configuration
    pub async fn create_plane(
        plane_type: PlaneType,
        config: PlaneConfig,
    ) -> Result<Arc<dyn FederatedPlane>> {
        match config {
            PlaneConfig::Local => {
                // Create in-process plane instance
                Self::create_local_plane(plane_type).await
            }
            PlaneConfig::Remote { endpoints, .. } => {
                // Create remote proxy for distributed deployment
                Self::create_remote_proxy(plane_type, endpoints).await
            }
            PlaneConfig::Hybrid { .. } => {
                // Create hybrid plane with local and remote components
                Self::create_hybrid_plane(plane_type, config).await
            }
        }
    }
    
    async fn create_local_plane(plane_type: PlaneType) -> Result<Arc<dyn FederatedPlane>> {
        // This will be implemented by the actual plane implementations
        // in their respective crates (dialog-core, media-core, etc.)
        todo!("Implement local plane creation")
    }
    
    async fn create_remote_proxy(
        plane_type: PlaneType,
        endpoints: Vec<String>,
    ) -> Result<Arc<dyn FederatedPlane>> {
        // Create a proxy that communicates with remote plane
        todo!("Implement remote proxy creation")
    }
    
    async fn create_hybrid_plane(
        plane_type: PlaneType,
        config: PlaneConfig,
    ) -> Result<Arc<dyn FederatedPlane>> {
        // Create hybrid plane with both local and remote components
        todo!("Implement hybrid plane creation")
    }
}