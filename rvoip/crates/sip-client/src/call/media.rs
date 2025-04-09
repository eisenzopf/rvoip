use tracing::{debug, warn};

use rvoip_session_core::sdp::SessionDescription;

use crate::error::{Error, Result};
use crate::media::{MediaSession, SdpHandler, MediaType};
use crate::config::{DEFAULT_RTP_PORT_MIN, DEFAULT_RTP_PORT_MAX};

use super::struct::Call;

impl Call {
    /// Setup local SDP for the call
    pub(crate) async fn setup_local_sdp(&self) -> Result<Option<SessionDescription>> {
        debug!("Setting up local SDP for call {}", self.id());
        
        // Create an SDP handler
        let local_ip = if let Ok(addr) = self.transaction_manager.transport().local_addr() {
            addr.ip()
        } else {
            "127.0.0.1".parse().unwrap()
        };
        
        let sdp_handler = SdpHandler::new(
            local_ip,
            self.config.rtp_port_range_start.unwrap_or(DEFAULT_RTP_PORT_MIN),
            self.config.rtp_port_range_end.unwrap_or(DEFAULT_RTP_PORT_MAX),
            self.config.clone(),
            self.local_sdp.clone(),
            self.remote_sdp.clone(),
        );
        
        // Create a new local SDP
        let local_sdp = sdp_handler.create_local_sdp().await?;
        
        // Store the created SDP
        if let Some(sdp) = &local_sdp {
            *self.local_sdp.write().await = Some(sdp.clone());
        }
        
        Ok(local_sdp)
    }
    
    /// Setup media from remote SDP
    pub async fn setup_media_from_sdp(&self, sdp: &SessionDescription) -> Result<()> {
        debug!("Setting up media from SDP for call {}", self.id());
        
        // Update our remote SDP
        *self.remote_sdp.write().await = Some(sdp.clone());
        
        // Create SDP handler
        let local_ip = if let Ok(addr) = self.transaction_manager.transport().local_addr() {
            addr.ip()
        } else {
            "127.0.0.1".parse().unwrap()
        };
        
        let sdp_handler = SdpHandler::new(
            local_ip,
            self.config.rtp_port_range_start.unwrap_or(DEFAULT_RTP_PORT_MIN),
            self.config.rtp_port_range_end.unwrap_or(DEFAULT_RTP_PORT_MAX),
            self.config.clone(),
            self.local_sdp.clone(),
            self.remote_sdp.clone(),
        );
        
        // Setup the media based on remote SDP
        let media_session = sdp_handler.setup_media(sdp).await?;
        
        // Store the media session
        self.media_sessions.write().await.push(media_session);
        
        Ok(())
    }
} 