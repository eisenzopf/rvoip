use std::time::Duration;
use tokio::time::sleep;
use tokio::sync::mpsc;
use tokio::signal;
use std::sync::Arc;

use rtp_core::api::client::{MediaTransportClient, ClientConfigBuilder, ClientFactory};
use rtp_core::api::server::{MediaTransportServer, ServerConfigBuilder, ServerFactory, ClientInfo};
use rtp_core::api::common::frame::{MediaFrame, MediaFrameType};
use rtp_core::api::common::events::{MediaTransportEvent, MediaEventCallback};
use rtp_core::api::common::buffer::{MediaBuffer, MediaBufferConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    
    println!("=== Secure Media Streaming Example ===");
    println!("This example demonstrates a secure media streaming server with multiple clients");
    
    // Create a server with DTLS-SRTP security
    let server_config = ServerConfigBuilder::new()
        .local_address("127.0.0.1:9000".parse().unwrap())
        .security_mode(rtp_core::api::common::config::SecurityMode::DtlsSrtp)
        .build()?;
    
    let server = ServerFactory::create_server(server_config).await?;
    
    // Print server security info
    let server_security = server.get_security_info().await?;
    println!("Server certificate fingerprint: {}", 
        server_security.fingerprint.as_ref().unwrap_or(&"None".to_string()));
    println!("Server fingerprint algorithm: {}", 
        server_security.fingerprint_algorithm.as_ref().unwrap_or(&"None".to_string()));
    println!("Server SRTP profiles: {:?}", server_security.srtp_profiles);
    
    // Track connected clients
    let connected_clients = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let connected_clients_clone = connected_clients.clone();
    
    // Register for client connections on server
    server.on_client_connected(Box::new(move |client_info| {
        let clients = connected_clients_clone.clone();
        tokio::spawn(async move {
            let mut clients_guard = clients.lock().await;
            clients_guard.push(client_info.clone());
            println!("Client connected: {} from {}", client_info.id, client_info.address);
        });
    }))?;
    
    // Start server
    server.start().await?;
    println!("Server started on 127.0.0.1:9000");
    
    // Create a media buffer for test frames
    let buffer_config = MediaBufferConfig {
        initial_jitter_delay_ms: 50,
        max_frames: 100,
        adaptive: true,
        clock_rate: 8000,
        ..Default::default()
    };
    
    let buffer = MediaBuffer::new(buffer_config)?;
    
    // Create a channel for sending test frames
    let (frame_tx, mut frame_rx) = mpsc::channel::<MediaFrame>(100);
    
    // Generate test audio frames (simulated)
    tokio::spawn(async move {
        let mut sequence = 0;
        let mut timestamp = 0;
        
        loop {
            // Create a dummy audio frame (simulated)
            let frame = MediaFrame {
                frame_type: MediaFrameType::Audio,
                data: vec![1, 2, 3, 4, 5, 6, 7, 8], // simulated audio data
                timestamp,
                sequence,
                marker: false,
                payload_type: 8, // G.711 A-law
                ssrc: 12345,
            };
            
            // Send frame to channel
            if frame_tx.send(frame).await.is_err() {
                break;
            }
            
            // Update sequence and timestamp
            sequence = sequence.wrapping_add(1);
            timestamp += 160; // 20ms @ 8kHz
            
            // Wait for next frame interval
            sleep(Duration::from_millis(20)).await;
        }
    });
    
    // Create clients
    let client_count = 2;
    let mut clients = Vec::new();
    
    for i in 0..client_count {
        // Create client with DTLS-SRTP security
        let client_config = ClientConfigBuilder::new()
            .remote_address("127.0.0.1:9000".parse().unwrap())
            .security_mode(rtp_core::api::common::config::SecurityMode::DtlsSrtp)
            .build()?;
        
        let client = ClientFactory::create_client(client_config).await?;
        
        // Set up client event callbacks
        let client_id = i + 1;
        let client_event_callback: MediaEventCallback = Box::new(move |event| {
            match event {
                MediaTransportEvent::FrameReceived(frame) => {
                    println!("Client {} received frame: seq={}, ts={}", 
                             client_id, frame.sequence, frame.timestamp);
                },
                MediaTransportEvent::Error(err) => {
                    println!("Client {} error: {}", client_id, err);
                },
                MediaTransportEvent::StateChanged(state) => {
                    println!("Client {} state changed: {:?}", client_id, state);
                },
                _ => {}
            }
        });
        
        client.on_event(client_event_callback)?;
        
        // Get client security info
        let client_security = client.get_security_info().await?;
        println!("Client {} fingerprint: {}", 
            client_id, client_security.fingerprint.as_ref().unwrap_or(&"None".to_string()));
        
        // Connect client (will perform DTLS handshake)
        client.connect().await?;
        println!("Client {} connected", client_id);
        
        clients.push(client);
    }
    
    // Wait for all clients to connect before starting to stream
    sleep(Duration::from_millis(1000)).await;
    
    // Server broadcasting task
    let server_clone = server.clone();
    tokio::spawn(async move {
        while let Some(frame) = frame_rx.recv().await {
            // Broadcast frame to all clients
            if let Err(e) = server_clone.broadcast_frame(frame.clone()).await {
                eprintln!("Error broadcasting frame: {}", e);
            }
            
            // Print stats occasionally
            if frame.sequence % 50 == 0 {
                if let Ok(stats) = server_clone.get_stats().await {
                    println!("Server stats: sent={}, received={}, quality={:?}", 
                             stats.packets_sent, stats.packets_received, stats.quality_level);
                }
            }
        }
    });
    
    // Wait for Ctrl+C
    println!("Press Ctrl+C to stop");
    match signal::ctrl_c().await {
        Ok(()) => {},
        Err(err) => {
            eprintln!("Unable to listen for shutdown signal: {}", err);
        },
    }
    
    // Shutdown
    println!("Shutting down...");
    
    // Disconnect clients
    for (i, client) in clients.iter().enumerate() {
        println!("Disconnecting client {}", i + 1);
        if let Err(e) = client.disconnect().await {
            eprintln!("Error disconnecting client {}: {}", i + 1, e);
        }
    }
    
    // Stop server
    println!("Stopping server");
    if let Err(e) = server.stop().await {
        eprintln!("Error stopping server: {}", e);
    }
    
    println!("Shutdown complete");
    Ok(())
} 