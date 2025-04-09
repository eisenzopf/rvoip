// Media module for managing RTP/RTCP sessions and codecs

mod rtp;
mod rtcp;
mod sdp_handler;

pub use rtp::*;
pub use rtcp::*;
pub use sdp_handler::{SdpHandler, media_direction_to_can_send, media_direction_to_can_receive};

// Re-export media components from media-core
pub use rvoip_media_core::{
    SrtpSession, SrtpConfig, SrtpKeys,
    DtlsConnection, DtlsConfig, DtlsEvent, TransportConn,
    codec::Codec, codec::CodecParams, codec::CodecType,
    codec::OpusCodec,
};

use std::sync::Arc;
use std::net::SocketAddr;
use tokio::sync::RwLock;
use bytes::Bytes;
use tokio::sync::{mpsc, Mutex};
use tokio::time::Duration;
use tokio::net::UdpSocket;
use uuid::Uuid;
use tracing::{debug, error, warn};

use crate::config::CodecType as ClientCodecType;
use crate::error::{Error, Result};
use crate::media::rtcp::{RtcpSession, RtcpStats};
use crate::ice::{IceSession, IceSessionState, IceConfig};

/// Type of media stream
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaType {
    /// Audio stream
    Audio,
    /// Video stream
    Video,
    /// Application data stream
    Application,
}

/// Media session managing RTP and optional RTCP
#[derive(Clone)]
pub struct MediaSession {
    /// Session ID
    id: String,
    
    /// Media type
    media_type: MediaType,
    
    /// RTP session
    rtp_session: Arc<RwLock<RtpSession>>,
    
    /// RTCP session (if enabled)
    rtcp_session: Option<Arc<RwLock<RtcpSession>>>,
    
    /// Local RTP address
    local_rtp_addr: SocketAddr,
    
    /// Remote RTP address
    remote_rtp_addr: SocketAddr,
    
    /// Local RTCP address (if enabled)
    local_rtcp_addr: Option<SocketAddr>,
    
    /// Remote RTCP address (if enabled)
    remote_rtcp_addr: Option<SocketAddr>,
    
    /// Active codec
    codec: ClientCodecType,
    
    /// Muted state
    muted: Arc<RwLock<bool>>,
    
    /// Holding state
    holding: Arc<RwLock<bool>>,
    
    /// ICE session for NAT traversal
    ice_session: Option<Arc<IceSession>>,
}

impl MediaSession {
    /// Create a new media session
    pub async fn new(
        media_type: MediaType,
        local_rtp_addr: SocketAddr,
        remote_rtp_addr: SocketAddr,
        codec: ClientCodecType,
        enable_rtcp: bool,
        enable_ice: bool,
    ) -> Result<Self> {
        // Set up ICE session if enabled
        let ice_session = if enable_ice {
            // Create ICE configuration
            let ice_config = IceConfig::default()
                .with_stun_servers(vec![
                    "stun:stun.l.google.com:19302".to_string(),
                    "stun:stun1.l.google.com:19302".to_string(),
                ])
                .with_gathering_policy(crate::ice::GatheringPolicy::All);
            
            // Create ICE session
            let session = IceSession::new(ice_config).await?;
            
            // Start gathering candidates
            session.start_gathering().await?;
            
            Some(Arc::new(session))
        } else {
            None
        };
    
        // Will be implemented in rtp.rs
        let rtp_session = RtpSession::new(
            local_rtp_addr,
            remote_rtp_addr,
            codec,
        ).await?;
        
        // If RTCP is enabled, set up RTCP session
        let (rtcp_session, local_rtcp_addr, remote_rtcp_addr) = if enable_rtcp {
            // RTCP typically uses RTP port + 1
            let local_rtcp_addr = SocketAddr::new(
                local_rtp_addr.ip(),
                local_rtp_addr.port() + 1
            );
            let remote_rtcp_addr = SocketAddr::new(
                remote_rtp_addr.ip(),
                remote_rtp_addr.port() + 1
            );
            
            // Will be implemented in rtcp.rs
            let rtcp_session = RtcpSession::new(
                local_rtcp_addr,
                remote_rtcp_addr,
            ).await?;
            
            (Some(Arc::new(RwLock::new(rtcp_session))), Some(local_rtcp_addr), Some(remote_rtcp_addr))
        } else {
            (None, None, None)
        };
        
        Ok(Self {
            id: Uuid::new_v4().to_string(),
            media_type,
            rtp_session: Arc::new(RwLock::new(rtp_session)),
            rtcp_session,
            local_rtp_addr,
            remote_rtp_addr,
            local_rtcp_addr,
            remote_rtcp_addr,
            codec,
            muted: Arc::new(RwLock::new(false)),
            holding: Arc::new(RwLock::new(false)),
            ice_session,
        })
    }
    
    /// Get media session ID
    pub fn id(&self) -> &str {
        &self.id
    }
    
    /// Get media type
    pub fn media_type(&self) -> MediaType {
        self.media_type
    }
    
    /// Get local RTP address
    pub fn local_rtp_addr(&self) -> SocketAddr {
        self.local_rtp_addr
    }
    
    /// Get remote RTP address
    pub fn remote_rtp_addr(&self) -> SocketAddr {
        self.remote_rtp_addr
    }
    
    /// Get local RTCP address if enabled
    pub fn local_rtcp_addr(&self) -> Option<SocketAddr> {
        self.local_rtcp_addr
    }
    
    /// Get remote RTCP address if enabled
    pub fn remote_rtcp_addr(&self) -> Option<SocketAddr> {
        self.remote_rtcp_addr
    }
    
    /// Get active codec
    pub fn codec(&self) -> ClientCodecType {
        self.codec
    }
    
    /// Check if session is muted
    pub async fn is_muted(&self) -> bool {
        *self.muted.read().await
    }
    
    /// Set mute state
    pub async fn set_muted(&self, muted: bool) -> Result<()> {
        *self.muted.write().await = muted;
        Ok(())
    }
    
    /// Check if session is on hold
    pub async fn is_holding(&self) -> bool {
        *self.holding.read().await
    }
    
    /// Set hold state
    pub async fn set_holding(&self, holding: bool) -> Result<()> {
        *self.holding.write().await = holding;
        Ok(())
    }
    
    /// Send audio data
    pub async fn send_audio(&self, data: Bytes) -> Result<()> {
        if self.media_type != MediaType::Audio {
            return Err(crate::error::Error::Media(
                "Cannot send audio on non-audio media session".into()
            ));
        }
        
        if *self.muted.read().await {
            // Silently drop if muted
            return Ok(());
        }
        
        let mut rtp_session = self.rtp_session.write().await;
        rtp_session.send_packet(data).await
    }
    
    /// Get the ICE session if enabled
    pub fn ice_session(&self) -> Option<Arc<IceSession>> {
        self.ice_session.clone()
    }
    
    /// Check if ICE is enabled for this session
    pub fn is_ice_enabled(&self) -> bool {
        self.ice_session.is_some()
    }
    
    /// Get ICE state if ICE is enabled
    pub async fn ice_state(&self) -> Option<IceSessionState> {
        if let Some(ice) = &self.ice_session {
            Some(ice.state().await)
        } else {
            None
        }
    }
    
    /// Start media flow
    pub async fn start(&self) -> Result<()> {
        // If ICE is enabled, wait for ICE to connect first
        if let Some(ice_session) = &self.ice_session {
            let ice_state = ice_session.state().await;
            
            // Only proceed if ICE is connected or we're not using ICE
            match ice_state {
                IceSessionState::Connected => {
                    // If we have a selected candidate pair, use those addresses
                    if let Some((local, remote)) = ice_session.selected_pair().await {
                        // Update RTP session with ICE-selected addresses
                        let mut rtp_session = self.rtp_session.write().await;
                        rtp_session.update_remote_addr(remote.socket_addr()).await?;
                    }
                }
                IceSessionState::Failed => {
                    return Err(crate::error::Error::Media("ICE connection failed".into()));
                }
                _ if ice_state != IceSessionState::New => {
                    // ICE is still in progress, wait for it to complete
                    return Err(crate::error::Error::Media("ICE connection not established yet".into()));
                }
                _ => {
                    // ICE is in New state or otherwise not active, proceed without ICE
                }
            }
        }
        
        let mut rtp_session = self.rtp_session.write().await;
        rtp_session.start().await?;
        
        if let Some(rtcp_session) = &self.rtcp_session {
            let mut rtcp = rtcp_session.write().await;
            rtcp.start().await?;
        }
        
        Ok(())
    }
    
    /// Stop media flow
    pub async fn stop(&self) -> Result<()> {
        let mut rtp_session = self.rtp_session.write().await;
        rtp_session.stop().await?;
        
        if let Some(rtcp_session) = &self.rtcp_session {
            let mut rtcp = rtcp_session.write().await;
            rtcp.stop().await?;
        }
        
        // Terminate ICE session if active
        if let Some(ice_session) = &self.ice_session {
            ice_session.terminate().await?;
        }
        
        Ok(())
    }
    
    /// Get current RTCP statistics if available
    pub async fn get_rtcp_stats(&self) -> Option<RtcpStats> {
        if let Some(rtcp_session) = &self.rtcp_session {
            let rtcp = rtcp_session.read().await;
            let stats = rtcp.get_stats().read().await.clone();
            return Some(stats);
        }
        None
    }
}

impl std::fmt::Debug for MediaSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaSession")
            .field("id", &self.id)
            .field("media_type", &self.media_type)
            .field("local_rtp_addr", &self.local_rtp_addr)
            .field("remote_rtp_addr", &self.remote_rtp_addr)
            .field("local_rtcp_addr", &self.local_rtcp_addr)
            .field("remote_rtcp_addr", &self.remote_rtcp_addr)
            .field("codec", &self.codec)
            .field("rtcp_enabled", &self.rtcp_session.is_some())
            .field("ice_enabled", &self.ice_session.is_some())
            .finish()
    }
} 