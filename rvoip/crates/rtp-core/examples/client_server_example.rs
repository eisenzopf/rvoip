use std::time::Duration;
use tokio::time::sleep;

use rtp_core::api::client::{MediaTransportClient, ClientConfigBuilder, ClientFactory};
use rtp_core::api::server::{MediaTransportServer, ServerConfigBuilder, ServerFactory};
use rtp_core::api::common::frame::{MediaFrame, MediaFrameType};
use rtp_core::api::common::events::{MediaTransportEvent, MediaEventCallback};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a server config
    let server_config = ServerConfigBuilder::new()
        .local_address("127.0.0.1:9000".parse().unwrap())
        .default_payload_type(8) // G.711 A-law
        .clock_rate(8000)
        .build();
    
    // Create a client config
    let client_config = ClientConfigBuilder::new()
        .remote_address("127.0.0.1:9000".parse().unwrap())
        .default_payload_type(8) // G.711 A-law
        .clock_rate(8000)
        .build();
    
    // Create server and client
    let server = ServerFactory::create_server(server_config).await?;
    let client = ClientFactory::create_client(client_config).await?;
    
    // Register event callbacks
    let client_event_callback: MediaEventCallback = Box::new(|event| {
        println!("Client received event: {:?}", event);
    });
    
    let server_event_callback: MediaEventCallback = Box::new(|event| {
        println!("Server received event: {:?}", event);
    });
    
    client.on_event(client_event_callback)?;
    server.on_event(server_event_callback)?;
    
    // Register for client connections on server
    server.on_client_connected(Box::new(|client_info| {
        println!("New client connected: {} from {}", client_info.id, client_info.address);
    }))?;
    
    // Start server
    server.start().await?;
    println!("Server started on 127.0.0.1:9000");
    
    // Connect client
    client.connect().await?;
    println!("Client connected to server");
    
    // Wait a bit for the connection to establish
    sleep(Duration::from_millis(500)).await;
    
    // Send a frame from client to server
    println!("Sending a test frame from client to server...");
    let client_frame = MediaFrame {
        frame_type: MediaFrameType::Audio,
        data: vec![1, 2, 3, 4, 5], // Dummy audio data
        timestamp: 1000,
        sequence: 1,
        marker: true,
        payload_type: 8,
        ssrc: 12345,
    };
    
    client.send_frame(client_frame).await?;
    
    // Wait for the frame to be processed
    sleep(Duration::from_millis(500)).await;
    
    // Receive a frame on the server
    println!("Waiting for frame on server...");
    match server.receive_frame().await {
        Ok((client_id, frame)) => {
            println!("Server received frame from client {}: {:?}", client_id, frame);
            
            // Send a response frame back to the client
            println!("Sending response from server to client...");
            let server_frame = MediaFrame {
                frame_type: MediaFrameType::Audio,
                data: vec![6, 7, 8, 9, 10], // Dummy response audio data
                timestamp: 2000,
                sequence: 1,
                marker: true,
                payload_type: 8,
                ssrc: 54321,
            };
            
            server.send_frame_to(&client_id, server_frame).await?;
        },
        Err(e) => {
            println!("Error receiving frame: {}", e);
        }
    }
    
    // Wait for a bit to let the response arrive
    sleep(Duration::from_millis(500)).await;
    
    // Try to receive a frame on the client
    match client.receive_frame(Duration::from_millis(1000)).await {
        Ok(Some(frame)) => {
            println!("Client received response frame: {:?}", frame);
        },
        Ok(None) => {
            println!("No response frame received within timeout");
        },
        Err(e) => {
            println!("Error receiving frame: {}", e);
        }
    }
    
    // Clean up
    println!("Disconnecting client...");
    client.disconnect().await?;
    
    println!("Stopping server...");
    server.stop().await?;
    
    println!("Example completed successfully");
    Ok(())
} 