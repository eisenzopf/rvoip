use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;
use tokio::sync::{RwLock, Mutex};
use bytes::{Bytes, BytesMut, BufMut};
use tracing::{debug, error, info, warn};

use rvoip_rtp_core::rtcp::{
    RtcpPacket, RtcpPacketType, RtcpReceiverReport, RtcpSenderReport, 
    RtcpReportBlock, NtpTimestamp
};

use crate::error::{Error, Result};

/// RTCP statistics
#[derive(Debug, Clone, Default)]
pub struct RtcpStats {
    /// Packets sent
    pub packets_sent: u64,
    
    /// Packets received
    pub packets_received: u64,
    
    /// Bytes sent
    pub bytes_sent: u64,
    
    /// Bytes received
    pub bytes_received: u64,
    
    /// Packets lost (reported by remote)
    pub packets_lost: i32,
    
    /// Fraction of packets lost (0-255, where 255 is 100%)
    pub fraction_lost: u8,
    
    /// Interarrival jitter in timestamp units
    pub jitter: u32,
    
    /// Round-trip time (if available) in milliseconds
    pub round_trip_time: Option<u32>,
    
    /// Time since last SR packet was received (in milliseconds)
    pub last_sr_delay: Option<u32>,
    
    /// NTP timestamp of last SR packet received
    pub last_sr_timestamp: Option<u64>,
    
    /// Local time when the last SR packet was received
    pub last_sr_time: Option<Instant>,
}

/// RTCP session for monitoring and controlling RTP sessions
pub struct RtcpSession {
    /// Local RTCP socket address
    local_addr: SocketAddr,
    
    /// Remote RTCP destination address
    remote_addr: SocketAddr,
    
    /// UDP socket for RTCP
    socket: Arc<UdpSocket>,
    
    /// SSRC of the associated RTP session
    ssrc: u32,
    
    /// Statistics
    stats: Arc<RwLock<RtcpStats>>,
    
    /// Is the session active
    is_active: Arc<RwLock<bool>>,
    
    /// Report sending interval
    report_interval: Duration,
    
    /// Task handle for the receive loop
    receive_task: Option<tokio::task::JoinHandle<()>>,
    
    /// Task handle for the report sender loop
    sender_task: Option<tokio::task::JoinHandle<()>>,
    
    /// Last time a packet was sent
    last_send_time: Arc<Mutex<Option<Instant>>>,
    
    /// Last time a packet was received
    last_receive_time: Arc<Mutex<Option<Instant>>>,
}

impl RtcpSession {
    /// Create a new RTCP session
    pub async fn new(
        local_addr: SocketAddr,
        remote_addr: SocketAddr,
    ) -> Result<Self> {
        // Start receiver task
        let socket = UdpSocket::bind(local_addr)
            .await
            .map_err(|e| Error::Network(e.to_string()))?;
        
        // Connect to remote to simplify send/recv
        socket.connect(remote_addr)
            .await
            .map_err(|e| Error::Network(e.to_string()))?;
        
        debug!("RTCP session created - local: {}, remote: {}", local_addr, remote_addr);
        
        // Create random SSRC (should match the RTP SSRC)
        let ssrc = rand::random::<u32>();
        
        Ok(Self {
            local_addr,
            remote_addr,
            socket: Arc::new(socket),
            ssrc,
            stats: Arc::new(RwLock::new(RtcpStats::default())),
            is_active: Arc::new(RwLock::new(false)),
            report_interval: Duration::from_secs(5),
            receive_task: None,
            sender_task: None,
            last_send_time: Arc::new(Mutex::new(None)),
            last_receive_time: Arc::new(Mutex::new(None)),
        })
    }
    
    /// Start the RTCP session
    pub async fn start(&mut self) -> Result<()> {
        if *self.is_active.read().await {
            return Ok(());
        }
        
        // Set active flag
        *self.is_active.write().await = true;
        
        // Start receive task
        let socket = self.socket.clone();
        let stats = self.stats.clone();
        let is_active = self.is_active.clone();
        let last_receive_time = self.last_receive_time.clone();
        let ssrc = self.ssrc;
        
        let receive_task = tokio::spawn(async move {
            debug!("RTCP receive task started");
            
            let mut buf = vec![0u8; 1500];
            
            while *is_active.read().await {
                // Receive with timeout
                let result = tokio::time::timeout(
                    Duration::from_secs(1),
                    socket.recv(&mut buf)
                ).await;
                
                match result {
                    Ok(Ok(len)) => {
                        // Update last receive time
                        *last_receive_time.lock().await = Some(Instant::now());
                        
                        // Process RTCP packet using rtp-core
                        match RtcpPacket::parse(&buf[..len]) {
                            Ok(packet) => {
                                // Update stats based on the packet
                                match process_rtcp_packet(&packet, stats.clone(), ssrc).await {
                                    Ok(_) => {},
                                    Err(e) => warn!("Error processing RTCP packet: {}", e),
                                }
                            },
                            Err(e) => {
                                warn!("Failed to parse RTCP packet: {}", e);
                            }
                        }
                    },
                    Ok(Err(e)) => {
                        error!("Error receiving RTCP packet: {}", e);
                        break;
                    },
                    Err(_) => {
                        // Timeout, continue
                    }
                }
            }
            
            debug!("RTCP receive task ended");
        });
        
        // Start sender task
        let socket = self.socket.clone();
        let stats = self.stats.clone();
        let is_active = self.is_active.clone();
        let last_send_time = self.last_send_time.clone();
        let ssrc = self.ssrc;
        let report_interval = self.report_interval;
        
        let sender_task = tokio::spawn(async move {
            debug!("RTCP sender task started");
            
            while *is_active.read().await {
                // Wait for report interval
                tokio::time::sleep(report_interval).await;
                
                // Check if still active
                if !*is_active.read().await {
                    break;
                }
                
                // Create RTCP receiver report using rtp-core
                let rr = RtcpReceiverReport::new(ssrc);
                let packet = RtcpPacket::ReceiverReport(rr);
                
                // Serialize the packet
                let mut buf = BytesMut::with_capacity(128);
                // Implement serialization if needed - rtp-core doesn't expose this yet
                // For now, we create a simple RR packet
                let data = create_simple_receiver_report(ssrc);
                
                if let Err(e) = socket.send(&data).await {
                    error!("Error sending RTCP packet: {}", e);
                } else {
                    // Update last send time
                    *last_send_time.lock().await = Some(Instant::now());
                    
                    // Update stats
                    let mut stats_write = stats.write().await;
                    stats_write.packets_sent += 1;
                    stats_write.bytes_sent += data.len() as u64;
                }
            }
            
            debug!("RTCP sender task ended");
        });
        
        self.receive_task = Some(receive_task);
        self.sender_task = Some(sender_task);
        
        Ok(())
    }
    
    /// Stop the RTCP session
    pub async fn stop(&mut self) -> Result<()> {
        if !*self.is_active.read().await {
            return Ok(());
        }
        
        // Set inactive flag
        *self.is_active.write().await = false;
        
        // Wait for tasks to end
        if let Some(task) = self.receive_task.take() {
            task.abort();
            let _ = tokio::time::timeout(Duration::from_millis(100), task).await;
        }
        
        if let Some(task) = self.sender_task.take() {
            task.abort();
            let _ = tokio::time::timeout(Duration::from_millis(100), task).await;
        }
        
        // Send BYE packet using rtp-core
        let bye_packet = create_simple_bye_packet(self.ssrc);
        if let Err(e) = self.socket.send(&bye_packet).await {
            warn!("Failed to send RTCP BYE packet: {}", e);
        }
        
        Ok(())
    }
    
    /// Get RTCP statistics
    pub fn get_stats(&self) -> &Arc<RwLock<RtcpStats>> {
        &self.stats
    }
    
    /// Set report interval
    pub fn set_report_interval(&mut self, interval: Duration) {
        self.report_interval = interval;
    }
    
    /// Set new SSRC
    pub fn set_ssrc(&mut self, ssrc: u32) {
        self.ssrc = ssrc;
    }
    
    /// Set new remote address
    pub async fn set_remote_addr(&mut self, remote_addr: SocketAddr) -> Result<()> {
        if remote_addr == self.remote_addr {
            return Ok(());
        }
        
        // Update remote address
        self.remote_addr = remote_addr;
        
        // Reconnect socket
        self.socket.connect(remote_addr).await
            .map_err(|e| Error::Network(e.to_string()))?;
        
        Ok(())
    }
    
    /// Process and handle an RTCP packet
    pub async fn handle_packet(&mut self, data: &[u8]) -> Result<()> {
        // If we have a socket, send the packet
        if let Some(socket) = &self.socket {
            socket.send(data)
                .await
                .map_err(|e| Error::Network(e.to_string()))?;
        }
        
        Ok(())
    }
    
    /// Send a receiver report
    pub async fn send_receiver_report(&self) -> Result<()> {
        // Process the RTCP report if we have a socket
        let socket = match &self.socket {
            Some(s) => s,
            None => return Err(Error::Media(rvoip_media_core::Error::Other("RTCP socket not available".into()))),
        };
        
        // Create receiver report
        let stats = self.stats.read().await;
        let rr = rvoip_rtp_core::rtcp::RtcpReceiverReport {
            ssrc: self.ssrc,
            report_blocks: vec![
                rvoip_rtp_core::rtcp::RtcpReportBlock {
                    ssrc: stats.packets_lost as u32, // Use as remote SSRC for now
                    fraction_lost: stats.fraction_lost,
                    cumulative_lost: stats.packets_lost as u32,
                    highest_seq: stats.jitter, // Use jitter as seq num for now
                    jitter: stats.jitter,
                    last_sr: stats.last_sr_timestamp.unwrap_or(0) as u32,
                    delay_since_last_sr: stats.last_sr_delay.unwrap_or(0),
                }
            ]
        };
        
        // For now, just use a simple method to create RTCP data
        let rtcp_data = create_simple_receiver_report(self.ssrc);
        
        // Send the packet
        if let Err(e) = socket.send(&rtcp_data).await {
            return Err(Error::Network(e.to_string()));
        }
        
        // Update timestamp for last report sent
        *self.last_send_time.lock().await = Some(Instant::now());
        
        Ok(())
    }
}

/// Process an RTCP packet and update statistics
async fn process_rtcp_packet(
    packet: &RtcpPacket,
    stats: Arc<RwLock<RtcpStats>>,
    local_ssrc: u32
) -> Result<()> {
    let mut stats_write = stats.write().await;
    stats_write.packets_received += 1;
    
    match packet {
        RtcpPacket::SenderReport(sr) => {
            // Process sender report
            let ntp_timestamp = sr.ntp_timestamp.to_u64();
            stats_write.last_sr_timestamp = Some(ntp_timestamp);
            stats_write.last_sr_time = Some(Instant::now());
            
            // Check if this report contains info about us
            for block in &sr.report_blocks {
                if block.ssrc == local_ssrc {
                    stats_write.fraction_lost = block.fraction_lost;
                    stats_write.packets_lost = block.cumulative_lost as i32;
                    stats_write.jitter = block.jitter;
                    break;
                }
            }
        },
        RtcpPacket::ReceiverReport(rr) => {
            // Check if this report contains info about us
            for block in &rr.report_blocks {
                if block.ssrc == local_ssrc {
                    stats_write.fraction_lost = block.fraction_lost;
                    stats_write.packets_lost = block.cumulative_lost as i32;
                    stats_write.jitter = block.jitter;
                    break;
                }
            }
        },
        RtcpPacket::Goodbye(_) => {
            debug!("Received RTCP BYE packet");
        },
        _ => {
            // Ignore other packet types
        }
    }
    
    Ok(())
}

/// Create a simple RTCP Receiver Report packet
fn create_simple_receiver_report(ssrc: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(8);
    
    // Header: version=2, padding=0, report count=0, packet type=RR (201)
    buf.push(0x80);
    buf.push(201);
    
    // Length in 32-bit words minus one
    buf.push(0);
    buf.push(1);
    
    // SSRC of packet sender
    buf.extend_from_slice(&ssrc.to_be_bytes());
    
    buf
}

/// Create a simple RTCP BYE packet
fn create_simple_bye_packet(ssrc: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(8);
    
    // Header: version=2, padding=0, source count=1, packet type=BYE (203)
    buf.push(0x81);
    buf.push(203);
    
    // Length in 32-bit words minus one
    buf.push(0);
    buf.push(1);
    
    // SSRC
    buf.extend_from_slice(&ssrc.to_be_bytes());
    
    buf
} 