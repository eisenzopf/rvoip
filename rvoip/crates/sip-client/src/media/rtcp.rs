use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;
use tokio::sync::{RwLock, Mutex};
use bytes::{Bytes, BytesMut, BufMut};
use tracing::{debug, error, info, warn};

use crate::error::{Error, Result};

/// RTCP packet types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RtcpPacketType {
    /// Sender Report (SR)
    SenderReport = 200,
    /// Receiver Report (RR)
    ReceiverReport = 201,
    /// Source Description (SDES)
    SourceDescription = 202,
    /// Goodbye (BYE)
    Goodbye = 203,
    /// Application-defined (APP)
    Application = 204,
}

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
        // Create UDP socket
        let socket = UdpSocket::bind(local_addr).await
            .map_err(|e| Error::Network(e))?;
        
        // Connect socket to remote address
        socket.connect(remote_addr).await
            .map_err(|e| Error::Network(e))?;
        
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
                        
                        // Process RTCP packet
                        if let Err(e) = process_rtcp_packet(&buf[..len], stats.clone(), ssrc).await {
                            warn!("Error processing RTCP packet: {}", e);
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
                
                // Send RTCP report
                let packet = create_rtcp_receiver_report(ssrc, stats.clone()).await;
                if let Err(e) = socket.send(&packet).await {
                    error!("Error sending RTCP packet: {}", e);
                } else {
                    // Update last send time
                    *last_send_time.lock().await = Some(Instant::now());
                    
                    // Update stats
                    let mut stats_write = stats.write().await;
                    stats_write.packets_sent += 1;
                    stats_write.bytes_sent += packet.len() as u64;
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
        
        // Send BYE packet
        let bye_packet = create_rtcp_bye_packet(self.ssrc);
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
            .map_err(|e| Error::Network(e))?;
        
        Ok(())
    }
}

/// Process a received RTCP packet
async fn process_rtcp_packet(
    packet: &[u8],
    stats: Arc<RwLock<RtcpStats>>,
    local_ssrc: u32
) -> Result<()> {
    if packet.len() < 8 {
        return Err(Error::Media("RTCP packet too short".into()));
    }
    
    // Update received stats
    {
        let mut stats_write = stats.write().await;
        stats_write.packets_received += 1;
        stats_write.bytes_received += packet.len() as u64;
    }
    
    // Get packet type
    let packet_type = packet[1];
    
    match packet_type {
        200 => {
            // SR packet (Sender Report)
            process_sender_report(packet, stats, local_ssrc).await?;
        },
        201 => {
            // RR packet (Receiver Report)
            process_receiver_report(packet, stats, local_ssrc).await?;
        },
        203 => {
            // BYE packet
            debug!("Received RTCP BYE packet");
        },
        _ => {
            // Ignore other packet types
            debug!("Received RTCP packet type {}", packet_type);
        }
    }
    
    Ok(())
}

/// Process a Sender Report
async fn process_sender_report(
    packet: &[u8],
    stats: Arc<RwLock<RtcpStats>>,
    local_ssrc: u32
) -> Result<()> {
    if packet.len() < 28 {
        return Err(Error::Media("RTCP SR packet too short".into()));
    }
    
    // Extract NTP timestamp (64 bits)
    let ntp_msw = u32::from_be_bytes([packet[8], packet[9], packet[10], packet[11]]);
    let ntp_lsw = u32::from_be_bytes([packet[12], packet[13], packet[14], packet[15]]);
    let ntp_timestamp = ((ntp_msw as u64) << 32) | (ntp_lsw as u64);
    
    // Extract SSRC of sender
    let ssrc = u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]);
    
    // Update stats with SR information
    let mut stats_write = stats.write().await;
    stats_write.last_sr_timestamp = Some(ntp_timestamp);
    stats_write.last_sr_time = Some(Instant::now());
    
    // If there are reports for us, process them
    let report_count = packet[0] & 0x1F;
    if report_count > 0 && packet.len() >= 28 + (report_count as usize * 24) {
        // Find report block for our SSRC
        for i in 0..report_count {
            let block_offset = 28 + (i as usize * 24);
            let report_ssrc = u32::from_be_bytes([
                packet[block_offset],
                packet[block_offset + 1],
                packet[block_offset + 2],
                packet[block_offset + 3]
            ]);
            
            if report_ssrc == local_ssrc {
                // Extract statistics
                let fraction_lost = packet[block_offset + 4];
                let cumulative_lost = i32::from_be_bytes([
                    0, // Ensure sign bit is 0 (we're treating this as 24-bit)
                    packet[block_offset + 5],
                    packet[block_offset + 6],
                    packet[block_offset + 7]
                ]);
                let jitter = u32::from_be_bytes([
                    packet[block_offset + 12],
                    packet[block_offset + 13],
                    packet[block_offset + 14],
                    packet[block_offset + 15]
                ]);
                
                // Update our stats
                stats_write.fraction_lost = fraction_lost;
                stats_write.packets_lost = cumulative_lost;
                stats_write.jitter = jitter;
                
                break;
            }
        }
    }
    
    Ok(())
}

/// Process a Receiver Report
async fn process_receiver_report(
    packet: &[u8],
    stats: Arc<RwLock<RtcpStats>>,
    local_ssrc: u32
) -> Result<()> {
    if packet.len() < 8 {
        return Err(Error::Media("RTCP RR packet too short".into()));
    }
    
    // Extract SSRC of sender
    let ssrc = u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]);
    
    // Check if there are report blocks for us
    let report_count = packet[0] & 0x1F;
    if report_count > 0 && packet.len() >= 8 + (report_count as usize * 24) {
        // Find report block for our SSRC
        for i in 0..report_count {
            let block_offset = 8 + (i as usize * 24);
            let report_ssrc = u32::from_be_bytes([
                packet[block_offset],
                packet[block_offset + 1],
                packet[block_offset + 2],
                packet[block_offset + 3]
            ]);
            
            if report_ssrc == local_ssrc {
                // Extract statistics
                let fraction_lost = packet[block_offset + 4];
                let cumulative_lost = i32::from_be_bytes([
                    0, // Ensure sign bit is 0 (we're treating this as 24-bit)
                    packet[block_offset + 5],
                    packet[block_offset + 6],
                    packet[block_offset + 7]
                ]);
                let jitter = u32::from_be_bytes([
                    packet[block_offset + 12],
                    packet[block_offset + 13],
                    packet[block_offset + 14],
                    packet[block_offset + 15]
                ]);
                
                // Update stats
                let mut stats_write = stats.write().await;
                stats_write.fraction_lost = fraction_lost;
                stats_write.packets_lost = cumulative_lost;
                stats_write.jitter = jitter;
                
                break;
            }
        }
    }
    
    Ok(())
}

/// Create an RTCP Receiver Report packet
async fn create_rtcp_receiver_report(
    ssrc: u32,
    stats: Arc<RwLock<RtcpStats>>
) -> Vec<u8> {
    let mut buf = BytesMut::with_capacity(32);
    
    // RTCP header (version=2, padding=0, report_count=0, packet_type=201, length=7)
    buf.put_u8(0x80); // Version=2, P=0, RC=0
    buf.put_u8(201);  // Packet type: Receiver Report
    buf.put_u16(7);   // Length in 32-bit words minus 1 (7 words = 32 bytes total)
    
    // SSRC of sender
    buf.put_u32(ssrc);
    
    // No report blocks for now
    
    // Get current NTP time (64-bit)
    let now = chrono::Utc::now();
    let unix_seconds = now.timestamp() as u64;
    let ntp_seconds = unix_seconds + 2_208_988_800; // Seconds from 1900 to 1970
    let ntp_fraction = ((now.timestamp_subsec_nanos() as u64) << 32) / 1_000_000_000;
    
    // RTCP header (version=2, padding=0, report_count=0, packet_type=203, length=1)
    buf.put_u8(0x80); // Version=2, P=0, RC=0
    buf.put_u8(203);  // Packet type: Bye
    buf.put_u16(1);   // Length in 32-bit words minus 1 (1 word = 8 bytes total)
    
    // SSRC
    buf.put_u32(ssrc);
    
    buf.freeze();
    buf.to_vec()
}

/// Create an RTCP BYE packet
fn create_rtcp_bye_packet(ssrc: u32) -> Vec<u8> {
    let mut buf = BytesMut::with_capacity(8);
    
    // RTCP header (version=2, padding=0, count=1, packet_type=203, length=1)
    buf.put_u8(0x81); // Version=2, P=0, SC=1
    buf.put_u8(203);  // Packet type: Bye
    buf.put_u16(1);   // Length in 32-bit words minus 1 (1 word = 8 bytes total)
    
    // SSRC
    buf.put_u32(ssrc);
    
    buf.freeze();
    buf.to_vec()
} 