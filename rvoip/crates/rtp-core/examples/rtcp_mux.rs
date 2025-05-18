use bytes::Bytes;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration, timeout};
use rvoip_rtp_core::{
    packet::{RtpPacket, rtcp::{RtcpPacket, RtcpSenderReport, RtcpGoodbye, RtcpApplicationDefined, NtpTimestamp}},
    transport::{RtpTransport, RtpTransportConfig, UdpRtpTransport},
    traits::RtpEvent,
};

#[tokio::main]
async fn main() {
    // Enable tracing for better debug output
    tracing_subscriber::fmt::init();
    
    println!("=== RFC 5761 RTCP Multiplexing Example ===");
    println!("This example demonstrates RTP/RTCP multiplexing on a single port");
    
    // Create a master timeout for the entire example
    if let Err(_) = timeout(Duration::from_secs(10), run_example()).await {
        println!("ERROR: Example timed out after 10 seconds. Exiting.");
    }
}

async fn run_example() {
    // Create two endpoints with different configurations
    let addr1 = "127.0.0.1:0".parse().unwrap();
    let addr2 = "127.0.0.1:0".parse().unwrap();
    
    println!("Setting up endpoint 1 with RTCP-MUX enabled...");
    let config1 = RtpTransportConfig {
        local_rtp_addr: addr1,
        local_rtcp_addr: None, // Not needed with RTCP-MUX
        symmetric_rtp: true,
        rtcp_mux: true, // Enable RTCP multiplexing
    };
    
    println!("Setting up endpoint 2 with RTCP-MUX enabled...");
    let config2 = RtpTransportConfig {
        local_rtp_addr: addr2,
        local_rtcp_addr: None, // Not needed with RTCP-MUX
        symmetric_rtp: true,
        rtcp_mux: true, // Enable RTCP multiplexing
    };
    
    // Create the transports
    let transport1 = Arc::new(UdpRtpTransport::new(config1).await.unwrap());
    let transport2 = Arc::new(UdpRtpTransport::new(config2).await.unwrap());
    
    let addr1 = transport1.local_rtp_addr().unwrap();
    let addr2 = transport2.local_rtp_addr().unwrap();
    println!("Endpoint 1 bound to: {}", addr1);
    println!("Endpoint 2 bound to: {}", addr2);
    
    println!("Notice that with RTCP-MUX, the RTP and RTCP addresses are the same:");
    println!("Endpoint 1 RTP: {}, RTCP: {}", 
             transport1.local_rtp_addr().unwrap(),
             transport1.local_rtcp_addr().unwrap());
             
    // Create a subscription for transport events
    let mut events1 = transport1.subscribe();
    let mut events2 = transport2.subscribe();
    
    // Create a channel to signal test completion
    let (tx, mut rx) = mpsc::channel::<bool>(1);
    let tx_clone = tx.clone();
    
    // Clone the transport for the tokio tasks
    let transport1_clone = transport1.clone();
    let transport2_clone = transport2.clone();
    
    // Start a task to listen for events on transport1
    let transport1_handle = tokio::spawn(async move {
        println!("Endpoint 1 waiting for packets...");
        let mut rtp_count = 0;
        let mut rtcp_count = 0;
        
        loop {
            match events1.recv().await {
                Ok(RtpEvent::MediaReceived { payload_type, timestamp, marker, payload, source }) => {
                    rtp_count += 1;
                    println!("Endpoint 1 received RTP packet: PT={}, TS={}, Marker={}, Size={}, From={}",
                            payload_type, timestamp, marker, payload.len(), source);
                            
                    if rtp_count >= 3 {
                        // Send some RTCP packets back
                        println!("Endpoint 1 sending RTCP packets to endpoint 2...");
                        
                        // Create a simple RTCP SR packet
                        let mut sr = RtcpSenderReport::new(0x12345678);
                        sr.ntp_timestamp = NtpTimestamp::now();
                        sr.rtp_timestamp = timestamp;
                        sr.sender_packet_count = 1;
                        sr.sender_octet_count = payload.len() as u32;
                        
                        // Create an RTCP BYE packet
                        let bye = RtcpGoodbye::new_with_reason(0x12345678, "Example test complete".to_string());
                        
                        // Send the RTCP packets using RTCP-MUX
                        let rtcp_packet = RtcpPacket::SenderReport(sr);
                        let serialized_sr = rtcp_packet.serialize().expect("Failed to serialize RTCP SR");
                        println!("Endpoint 1 sending SR packet, first bytes: {:?}", &serialized_sr[..4]);
                        transport1_clone.send_rtcp(&rtcp_packet, addr2).await.expect("Failed to send RTCP SR");
                        
                        let rtcp_packet = RtcpPacket::Goodbye(bye);
                        let serialized_bye = rtcp_packet.serialize().expect("Failed to serialize RTCP BYE");
                        println!("Endpoint 1 sending BYE packet, first bytes: {:?}", &serialized_bye[..4]);
                        transport1_clone.send_rtcp(&rtcp_packet, addr2).await.expect("Failed to send RTCP BYE");
                        
                        println!("Endpoint 1 sent RTCP packets to endpoint 2");
                        break;
                    }
                },
                Ok(RtpEvent::RtcpReceived { data, source }) => {
                    rtcp_count += 1;
                    println!("Endpoint 1 received RTCP packet: Size={}, From={}", data.len(), source);
                    
                    // Try to parse the RTCP packet
                    match RtcpPacket::parse(&data) {
                        Ok(rtcp) => {
                            println!("  RTCP packet parsed successfully: {:?}", rtcp);
                        },
                        Err(e) => {
                            println!("  Failed to parse RTCP packet: {}", e);
                        }
                    }
                },
                Ok(RtpEvent::Error(e)) => {
                    println!("Endpoint 1 error: {}", e);
                },
                Err(e) => {
                    println!("Endpoint 1 channel error: {}", e);
                    break;
                }
            }
        }
        
        // Signal test completion
        println!("Endpoint 1 signaling completion");
        let _ = tx.send(true).await;
    });
    
    // Start a task to listen for events on transport2
    let transport2_handle = tokio::spawn(async move {
        println!("Endpoint 2 waiting for packets...");
        let mut rtp_count = 0;
        let mut rtcp_count = 0;
        
        loop {
            match events2.recv().await {
                Ok(RtpEvent::MediaReceived { payload_type, timestamp, marker, payload, source }) => {
                    rtp_count += 1;
                    println!("Endpoint 2 received RTP packet: PT={}, TS={}, Marker={}, Size={}, From={}",
                            payload_type, timestamp, marker, payload.len(), source);
                },
                Ok(RtpEvent::RtcpReceived { data, source }) => {
                    rtcp_count += 1;
                    println!("Endpoint 2 received RTCP packet: Size={}, From={}", data.len(), source);
                    
                    // Try to parse the RTCP packet
                    match RtcpPacket::parse(&data) {
                        Ok(rtcp) => {
                            println!("  RTCP packet parsed successfully: {:?}", rtcp);
                            
                            // If we received a BYE packet, send one back
                            if let RtcpPacket::Goodbye(_) = rtcp {
                                println!("Endpoint 2 received BYE, sending APP packet back...");
                                
                                // Create an RTCP APP packet
                                let mut app = RtcpApplicationDefined::new(0x87654321, *b"TEST");
                                app.set_data(Bytes::from_static(b"RFC5761 test complete"));
                                
                                let rtcp_packet = RtcpPacket::ApplicationDefined(app);
                                transport2_clone.send_rtcp(&rtcp_packet, addr1).await.expect("Failed to send RTCP APP");
                                
                                println!("Endpoint 2 sent RTCP APP packet to endpoint 1");
                                break;
                            }
                        },
                        Err(e) => {
                            println!("  Failed to parse RTCP packet: {}", e);
                        }
                    }
                },
                Ok(RtpEvent::Error(e)) => {
                    println!("Endpoint 2 error: {}", e);
                },
                Err(e) => {
                    println!("Endpoint 2 channel error: {}", e);
                    break;
                }
            }
        }
        
        // Signal test completion
        println!("Endpoint 2 signaling completion");
        let _ = tx_clone.send(true).await;
    });
    
    // Send RTP packets from transport2 to transport1
    println!("Sending RTP packets from endpoint 2 to endpoint 1...");
    for i in 0..3 {
        // Create a dummy RTP packet
        let packet = RtpPacket::new(
            rvoip_rtp_core::packet::RtpHeader::new(
                96, // Payload type
                i as u16, // Sequence number
                1000 + i * 160, // Timestamp
                0x87654321, // SSRC
            ),
            Bytes::from(format!("RTP packet {}", i)),
        );
        
        // Send the packet
        transport2.send_rtp(&packet, addr1).await.expect("Failed to send RTP packet");
        println!("Sent RTP packet {} from endpoint 2 to endpoint 1", i);
        
        // Wait a bit between packets
        sleep(Duration::from_millis(100)).await;
    }
    
    // Wait for the test to complete with a timeout
    println!("Waiting for test completion signals (with 5 second timeout)...");
    let timeout_result = timeout(Duration::from_secs(5), async {
        // Wait for first completion signal
        if let Some(true) = rx.recv().await {
            println!("Received first completion signal");
        }
        
        // Wait for second completion signal
        if let Some(true) = rx.recv().await {
            println!("Received second completion signal");
        }
    }).await;
    
    // Check if we timed out
    match timeout_result {
        Ok(_) => println!("Test complete! Both endpoints communicated successfully using RFC 5761 RTCP-MUX."),
        Err(_) => println!("WARNING: Test timed out waiting for completion signals. Some part of the test may have failed."),
    }
    
    // Abort any remaining tasks to ensure clean shutdown
    transport1_handle.abort();
    transport2_handle.abort();
} 