use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use anyhow::Result;

use rvoip_rtp_core::{RtpSession, RtpSessionConfig, RtpPacket};
use rvoip_media_core::{
    AudioBuffer, AudioFormat, SampleRate,
    codec::{Codec, G711Codec, G711Variant}
};

/// Supported media types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    /// Audio media
    Audio,
    /// Video media
    Video,
}

/// Supported audio codecs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioCodecType {
    /// G.711 μ-law
    PCMU,
    /// G.711 A-law
    PCMA,
}

/// Media stream configuration
#[derive(Debug, Clone)]
pub struct MediaConfig {
    /// Local address for RTP
    pub local_addr: SocketAddr,
    
    /// Remote address for RTP
    pub remote_addr: Option<SocketAddr>,
    
    /// Media type
    pub media_type: MediaType,
    
    /// RTP payload type
    pub payload_type: u8,
    
    /// RTP clock rate
    pub clock_rate: u32,
    
    /// Audio codec type
    pub audio_codec: AudioCodecType,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            local_addr: "127.0.0.1:10000".parse().unwrap(),
            remote_addr: None,
            media_type: MediaType::Audio,
            payload_type: 0,
            clock_rate: 8000,
            audio_codec: AudioCodecType::PCMU,
        }
    }
}

/// Media stream for a SIP session
pub struct MediaStream {
    /// RTP session
    rtp_session: Arc<Mutex<RtpSession>>,
    
    /// Audio codec
    codec: Arc<dyn Codec>,
    
    /// Media configuration
    config: MediaConfig,
    
    /// Channel for sending audio
    audio_tx: mpsc::Sender<AudioBuffer>,
    
    /// Channel for receiving audio
    audio_rx: Mutex<mpsc::Receiver<AudioBuffer>>,
    
    /// Whether the stream is active
    active: Mutex<bool>,
}

impl MediaStream {
    /// Create a new media stream
    pub async fn new(config: MediaConfig) -> Result<Self> {
        // Configure RTP session
        let rtp_config = RtpSessionConfig {
            local_addr: config.local_addr,
            remote_addr: config.remote_addr,
            payload_type: config.payload_type,
            clock_rate: config.clock_rate,
            ..Default::default()
        };
        
        let rtp_session = RtpSession::new(rtp_config).await?;
        
        // Create codec
        let codec: Arc<dyn Codec> = match config.audio_codec {
            AudioCodecType::PCMU => Arc::new(G711Codec::new(G711Variant::PCMU)),
            AudioCodecType::PCMA => Arc::new(G711Codec::new(G711Variant::PCMA)),
        };
        
        // Create channels for audio data
        let (audio_tx, audio_rx) = mpsc::channel(100);
        
        Ok(Self {
            rtp_session: Arc::new(Mutex::new(rtp_session)),
            codec,
            config,
            audio_tx,
            audio_rx: Mutex::new(audio_rx),
            active: Mutex::new(false),
        })
    }
    
    /// Start the media stream
    pub async fn start(&self) -> Result<()> {
        let mut active = self.active.lock().await;
        if *active {
            return Ok(());
        }
        
        *active = true;
        
        // Clone necessary components for the receiver task
        let codec = self.codec.clone();
        let audio_tx = self.audio_tx.clone();
        let rtp_session = Arc::clone(&self.rtp_session);
        
        // Start a task to process incoming RTP packets
        tokio::spawn(async move {
            let mut rtp_session_lock = rtp_session.lock().await;
            while let Ok(packet) = rtp_session_lock.receive_packet().await {
                // Decode audio data
                if let Ok(audio) = codec.decode(&packet.payload) {
                    let _ = audio_tx.send(audio).await;
                }
            }
        });
        
        Ok(())
    }
    
    /// Stop the media stream
    pub async fn stop(&self) -> Result<()> {
        let mut active = self.active.lock().await;
        if !*active {
            return Ok(());
        }
        
        *active = false;
        
        // Close the RTP session
        let mut rtp_session = self.rtp_session.lock().await;
        rtp_session.close().await;
        
        Ok(())
    }
    
    /// Send audio data via RTP
    pub async fn send_audio(&self, audio: AudioBuffer) -> Result<()> {
        // Only send if active
        if !*self.active.lock().await {
            return Ok(());
        }
        
        // Encode audio
        let encoded = self.codec.encode(&audio)?;
        
        // Send via RTP
        let mut rtp_session = self.rtp_session.lock().await;
        // Use a simple timestamp based on audio duration
        let timestamp = (audio.duration_ms() * self.config.clock_rate / 1000) as u32;
        rtp_session.send_packet(timestamp, encoded, false).await?;
        
        Ok(())
    }
    
    /// Get a sender for audio data
    pub fn audio_sender(&self) -> mpsc::Sender<AudioBuffer> {
        self.audio_tx.clone()
    }
    
    /// Set the remote address for RTP
    pub async fn set_remote_addr(&self, addr: SocketAddr) -> Result<()> {
        let mut rtp_session = self.rtp_session.lock().await;
        rtp_session.set_remote_addr(addr);
        Ok(())
    }
    
    /// Get the current media stream active state
    pub async fn is_active(&self) -> bool {
        *self.active.lock().await
    }
} 