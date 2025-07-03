/// This is a minimal test for raw UDP sockets
/// 
/// It directly tests UDP socket functionality without the RTP layer.

use std::net::{SocketAddr, IpAddr, Ipv4Addr};
use std::time::Duration;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Raw UDP Socket Test");
    println!("==================\n");

    // Create UDP sockets
    let server_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 20000);
    let client_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 10000);
    
    println!("Creating server socket on {}", server_addr);
    let server_socket = Arc::new(UdpSocket::bind(server_addr).await?);
    
    println!("Creating client socket on {}", client_addr);
    let client_socket = UdpSocket::bind(client_addr).await?;
    
    // Spawn receiver task
    let server_socket_clone = server_socket.clone();
    let receiver_task = tokio::spawn(async move {
        println!("Receiver task started");
        let mut buf = vec![0u8; 1024];
        
        loop {
            match server_socket_clone.recv_from(&mut buf).await {
                Ok((len, src)) => {
                    println!("Received {} bytes from {}", len, src);
                    println!("Data: {:?}", &buf[..len]);
                },
                Err(e) => {
                    println!("Error receiving: {}", e);
                    break;
                }
            }
        }
    });
    
    // Send test packets
    println!("\nSending 3 test UDP packets...");
    for i in 1..=3 {
        let test_data = vec![i as u8; 20]; // Simple test data
        client_socket.send_to(&test_data, server_addr).await?;
        println!("Sent packet #{} ({} bytes) to {}", i, test_data.len(), server_addr);
        
        // Wait a bit to see the packet arrive
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    
    // Wait a bit to ensure all packets are processed
    println!("\nWaiting for final packets...");
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Clean up
    println!("\nTest completed");
    receiver_task.abort();
    
    Ok(())
} 