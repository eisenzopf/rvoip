use anyhow::{Context, Result};
use bytes::Bytes;
use clap::Parser;
use rtp_core::RtpSession;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, error, debug, Level};
use tracing_subscriber::FmtSubscriber;

// Include RTCP example module
mod rtcp_example;

/// Simple RTP loopback test application
///
/// This example creates both a sender and receiver RTP session and demonstrates
/// sending packets between them.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Local address for the sender
    #[arg(short = 's', long, default_value = "127.0.0.1:10000")]
    sender_addr: SocketAddr,

    /// Local address for the receiver
    #[arg(short = 'r', long, default_value = "127.0.0.1:10001")]
    receiver_addr: SocketAddr,

    /// Number of packets to send
    #[arg(short, long, default_value = "10")]
    count: u32,

    /// Interval between packets in milliseconds
    #[arg(short, long, default_value = "100")]
    interval: u64,

    /// Payload type
    #[arg(short, long, default_value = "0")]
    payload_type: u8,
    
    /// Run RTCP example instead of basic loopback
    #[arg(long)]
    rtcp: bool,
    
    /// Enable verbose debug logging
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging with appropriate level
    let args = Args::parse();
    let subscriber = FmtSubscriber::builder()
        .with_max_level(if args.verbose { Level::DEBUG } else { Level::INFO })
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .expect("Failed to set global default subscriber");
    
    if args.rtcp {
        // Run RTCP example
        info!("Running RTCP example");
        return rtcp_example::run_rtcp_example().await;
    }
    
    // Run basic loopback example
    info!("Starting RTP loopback test");
    
    // Create sender RTP session
    let sender_config = rtp_core::session::RtpSessionConfig {
        local_addr: args.sender_addr,
        remote_addr: Some(args.receiver_addr),
        payload_type: args.payload_type,
        enable_jitter_buffer: false, // No need for jitter buffer in this example
        ..Default::default()
    };
    
    let mut sender = RtpSession::new(sender_config)
        .await
        .context("Failed to create sender RTP session")?;
    
    info!("Sender created and bound to {}", args.sender_addr);
    let sender_actual_addr = sender.local_addr()?;
    info!("Sender actual bound address: {}", sender_actual_addr);
    
    // Create receiver RTP session
    let receiver_config = rtp_core::session::RtpSessionConfig {
        local_addr: args.receiver_addr,
        remote_addr: Some(sender_actual_addr), // Use the actual bound address
        payload_type: args.payload_type,
        ..Default::default()
    };
    
    let mut receiver = RtpSession::new(receiver_config)
        .await
        .context("Failed to create receiver RTP session")?;
    
    info!("Receiver created and bound to {}", args.receiver_addr);
    let receiver_actual_addr = receiver.local_addr()?;
    info!("Receiver actual bound address: {}", receiver_actual_addr);
    
    // Update sender's remote address to the actual receiver address
    sender.set_remote_addr(receiver_actual_addr);
    info!("Updated sender to send to: {}", receiver_actual_addr);
    
    // Give sockets time to properly set up
    debug!("Waiting for sockets to stabilize...");
    sleep(Duration::from_millis(250)).await;
    
    // Start receiver task
    let verbose = args.verbose;
    let count = args.count;
    let receiver_handle = tokio::spawn(async move {
        info!("Receiver task started, waiting for packets...");
        
        // Keep track of received packets
        let mut received_count = 0;
        
        while received_count < count {
            match receiver.receive_packet().await {
                Ok(packet) => {
                    received_count += 1;
                    
                    // Convert payload to string
                    let payload_str = String::from_utf8_lossy(&packet.payload);
                    
                    info!(
                        "Received packet {}/{}: seq={}, ts={}, len={}, payload={}",
                        received_count,
                        count,
                        packet.header.sequence_number,
                        packet.header.timestamp,
                        packet.payload.len(),
                        payload_str
                    );
                }
                Err(e) => {
                    error!("Error receiving packet: {}", e);
                    break;
                }
            }
        }
        
        info!("Receiver task completed, received {} packets", received_count);
        Ok::<_, anyhow::Error>(received_count)
    });
    
    // Give receiver time to start
    sleep(Duration::from_millis(250)).await;
    
    // Send test packets
    info!("Starting to send {} packets", args.count);
    
    for i in 0..args.count {
        // Create payload with packet number
        let payload_data = format!("Test packet {}", i);
        
        if verbose {
            debug!("Creating payload: '{}' ({} bytes)", payload_data, payload_data.len());
        }
        
        let payload = Bytes::from(payload_data);
        
        // Use packet number as timestamp for simplicity
        let timestamp = i * 160; // Assuming G.711 with 20ms packets (8000 Hz * 0.02s = 160 samples)
        
        // Send packet
        match sender.send_packet(timestamp, payload, i == 0).await {
            Ok(_) => {
                info!("Sent packet {}/{}: ts={}", i+1, args.count, timestamp);
            }
            Err(e) => {
                error!("Failed to send packet {}: {}", i, e);
                break;
            }
        }
        
        // Wait before sending the next packet
        sleep(Duration::from_millis(args.interval)).await;
    }
    
    // Wait for receiver to process all packets
    match receiver_handle.await {
        Ok(result) => {
            match result {
                Ok(received_count) => {
                    // Print stats
                    let sender_stats = sender.get_stats();
                    info!("Sender stats: sent={} packets", sender_stats.packets_sent);
                    
                    info!("Test completed successfully: sent={}, received={} packets", 
                          args.count, received_count);
                }
                Err(e) => {
                    error!("Receiver task returned an error: {}", e);
                }
            }
        }
        Err(e) => {
            error!("Failed to join receiver task: {}", e);
        }
    }
    
    Ok(())
} 