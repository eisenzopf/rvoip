use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time;
use tracing::{debug, warn};

use crate::error::Error;
use crate::packet::RtpPacket;
use crate::{Result, RtpSequenceNumber, RtpTimestamp};

/// RTP packet scheduler for periodic sending of packets
pub struct RtpScheduler {
    /// Current sequence number for outgoing packets
    sequence: RtpSequenceNumber,
    
    /// Current timestamp
    timestamp: RtpTimestamp,
    
    /// Clock rate (samples per second)
    clock_rate: u32,
    
    /// Timestamp increment per packet
    timestamp_increment: RtpTimestamp,
    
    /// Initial timestamp
    initial_timestamp: RtpTimestamp,
    
    /// Time when scheduler was started
    start_time: Option<Instant>,
    
    /// Packet interval
    interval: Duration,
    
    /// Buffer to store outgoing packets before they're sent
    packet_queue: Arc<Mutex<Vec<(RtpPacket, Instant)>>>,
    
    /// Sending task handle
    send_task: Option<JoinHandle<()>>,
    
    /// Channel to send packets to the transport
    sender: Option<mpsc::Sender<RtpPacket>>,
    
    /// Whether the scheduler is currently running
    running: bool,
    
    /// Number of packets scheduled
    packets_scheduled: u64,
    
    /// Number of packets sent
    packets_sent: u64,
}

impl RtpScheduler {
    /// Create a new RTP scheduler
    pub fn new(clock_rate: u32, initial_seq: RtpSequenceNumber, initial_ts: RtpTimestamp) -> Self {
        Self {
            sequence: initial_seq,
            timestamp: initial_ts,
            clock_rate,
            timestamp_increment: 0,
            initial_timestamp: initial_ts,
            start_time: None,
            interval: Duration::from_millis(20), // Default: 20ms (common for audio)
            packet_queue: Arc::new(Mutex::new(Vec::new())),
            send_task: None,
            sender: None,
            running: false,
            packets_scheduled: 0,
            packets_sent: 0,
        }
    }
    
    /// Set the packet interval
    /// 
    /// # Arguments
    /// * `interval_ms` - Packet interval in milliseconds
    /// * `samples_per_packet` - Number of samples in each packet
    pub fn set_interval(&mut self, interval_ms: u64, samples_per_packet: u32) {
        self.interval = Duration::from_millis(interval_ms);
        self.timestamp_increment = samples_per_packet;
        debug!("Set packet interval to {}ms ({} samples per packet)", 
               interval_ms, samples_per_packet);
    }
    
    /// Set the output channel
    pub fn set_sender(&mut self, sender: mpsc::Sender<RtpPacket>) {
        self.sender = Some(sender);
    }
    
    /// Schedule a packet to be sent at the appropriate time
    pub fn schedule_packet(&mut self, mut packet: RtpPacket) -> Result<()> {
        if !self.running {
            return Err(Error::SessionError("Scheduler not running".to_string()));
        }
        
        // Update sequence number and timestamp
        packet.header.sequence_number = self.sequence;
        packet.header.timestamp = self.timestamp;
        
        // Advance sequence number (wraps around automatically due to u16)
        self.sequence = self.sequence.wrapping_add(1);
        
        // Advance timestamp
        self.timestamp = self.timestamp.wrapping_add(self.timestamp_increment);
        
        // Calculate when this packet should be sent
        let now = Instant::now();
        let start = self.start_time.unwrap_or(now);
        let elapsed = now.duration_since(start);
        
        // Calculate the number of intervals since start
        let intervals = elapsed.as_millis() / self.interval.as_millis();
        let next_interval = (intervals + 1) as u64 * self.interval.as_millis() as u64;
        let next_send_time = start + Duration::from_millis(next_interval);
        
        // Add to queue
        if let Ok(mut queue) = self.packet_queue.lock() {
            queue.push((packet, next_send_time));
            self.packets_scheduled += 1;
            debug!("Scheduled packet with seq={}, ts={} for {:?}", 
                   self.sequence.wrapping_sub(1), self.timestamp.wrapping_sub(self.timestamp_increment),
                   next_send_time);
            Ok(())
        } else {
            Err(Error::SessionError("Failed to lock packet queue".to_string()))
        }
    }
    
    /// Start the scheduler
    pub fn start(&mut self) -> Result<()> {
        if self.running {
            return Ok(());
        }
        
        if self.sender.is_none() {
            return Err(Error::SessionError("No sender channel configured".to_string()));
        }
        
        self.start_time = Some(Instant::now());
        self.running = true;
        
        // Clone necessary data for the task
        let queue = self.packet_queue.clone();
        let sender = self.sender.clone().unwrap();
        let interval = self.interval;
        
        // Start a task to send packets at their scheduled times
        let handle = tokio::spawn(async move {
            let mut interval_timer = time::interval(Duration::from_millis(1));
            
            loop {
                interval_timer.tick().await;
                
                // Check for packets that need to be sent
                let now = Instant::now();
                let mut packets_to_send = Vec::new();
                
                // Extract packets that are due to be sent
                if let Ok(mut queue) = queue.lock() {
                    let mut i = 0;
                    while i < queue.len() {
                        if queue[i].1 <= now {
                            packets_to_send.push(queue.remove(i));
                        } else {
                            i += 1;
                        }
                    }
                }
                
                // Send the packets
                for (packet, _) in packets_to_send {
                    if let Err(e) = sender.send(packet).await {
                        warn!("Failed to send scheduled packet: {}", e);
                    }
                }
                
                // Check if the channel is closed
                if sender.is_closed() {
                    debug!("Sender channel closed, stopping scheduler");
                    break;
                }
            }
        });
        
        self.send_task = Some(handle);
        debug!("Started RTP scheduler");
        Ok(())
    }
    
    /// Stop the scheduler
    pub async fn stop(&mut self) {
        if !self.running {
            return;
        }
        
        self.running = false;
        
        // Abort the sending task if it's running
        if let Some(handle) = self.send_task.take() {
            handle.abort();
            debug!("Stopped RTP scheduler");
        }
    }
    
    /// Get the number of packets currently in the queue
    pub fn queue_size(&self) -> usize {
        if let Ok(queue) = self.packet_queue.lock() {
            queue.len()
        } else {
            0
        }
    }
    
    /// Get the current sequence number (the next one to be used)
    pub fn get_sequence(&self) -> RtpSequenceNumber {
        self.sequence
    }
    
    /// Get the current timestamp (the next one to be used)
    pub fn get_timestamp(&self) -> RtpTimestamp {
        self.timestamp
    }
    
    /// Get statistics about the scheduler
    pub fn get_stats(&self) -> RtpSchedulerStats {
        RtpSchedulerStats {
            packets_scheduled: self.packets_scheduled,
            packets_sent: self.packets_sent,
            queue_size: self.queue_size(),
            running: self.running,
        }
    }
}

/// Statistics for the RTP scheduler
#[derive(Debug, Clone)]
pub struct RtpSchedulerStats {
    /// Number of packets scheduled
    pub packets_scheduled: u64,
    
    /// Number of packets actually sent
    pub packets_sent: u64,
    
    /// Current queue size
    pub queue_size: usize,
    
    /// Whether the scheduler is running
    pub running: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use crate::packet::{RtpHeader, RtpPacket};
    use crate::RtpSsrc;
    
    #[tokio::test]
    async fn test_scheduler_basic() {
        let (tx, mut rx) = mpsc::channel(32);
        
        // Create scheduler
        let mut scheduler = RtpScheduler::new(8000, 1000, 0);
        scheduler.set_interval(20, 160); // 20ms, 160 samples (8kHz * 20ms)
        scheduler.set_sender(tx);
        
        // Start the scheduler
        scheduler.start().unwrap();
        
        // Create test packet
        let header = RtpHeader::new(0, 0, 0, 0x12345678);
        let payload = Bytes::from_static(b"test");
        let packet = RtpPacket::new(header, payload);
        
        // Schedule the packet
        scheduler.schedule_packet(packet).unwrap();
        
        // Wait to receive the packet
        let received = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await;
        assert!(received.is_ok());
        
        let packet = received.unwrap().unwrap();
        assert_eq!(packet.header.sequence_number, 1000);
        assert_eq!(packet.header.timestamp, 0);
        
        // Stop the scheduler
        scheduler.stop().await;
    }
    
    #[tokio::test]
    async fn test_scheduler_timestamp_increment() {
        let (tx, mut rx) = mpsc::channel(32);
        
        // Create scheduler
        let mut scheduler = RtpScheduler::new(8000, 1000, 0);
        scheduler.set_interval(20, 160); // 20ms, 160 samples (8kHz * 20ms)
        scheduler.set_sender(tx);
        
        // Start the scheduler
        scheduler.start().unwrap();
        
        // Create and schedule multiple packets
        for _ in 0..3 {
            let header = RtpHeader::new(0, 0, 0, 0x12345678);
            let payload = Bytes::from_static(b"test");
            let packet = RtpPacket::new(header, payload);
            scheduler.schedule_packet(packet).unwrap();
        }
        
        // Receive first packet
        let packet1 = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await.unwrap().unwrap();
        assert_eq!(packet1.header.sequence_number, 1000);
        assert_eq!(packet1.header.timestamp, 0);
        
        // Receive second packet
        let packet2 = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await.unwrap().unwrap();
        assert_eq!(packet2.header.sequence_number, 1001);
        assert_eq!(packet2.header.timestamp, 160);
        
        // Receive third packet
        let packet3 = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await.unwrap().unwrap();
        assert_eq!(packet3.header.sequence_number, 1002);
        assert_eq!(packet3.header.timestamp, 320);
        
        // Stop the scheduler
        scheduler.stop().await;
    }
} 