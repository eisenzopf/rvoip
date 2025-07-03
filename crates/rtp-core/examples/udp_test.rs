use std::net::{SocketAddr, IpAddr, Ipv4Addr};
use tokio::net::UdpSocket;
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up the same addresses as in the DTLS test
    let server_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 20000);
    let client_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 20001);
    
    println!("UDP Basic Test");
    println!("=============");
    
    // Create the sockets
    println!("Creating UDP sockets...");
    let server_socket = Arc::new(UdpSocket::bind(server_addr).await?);
    let client_socket = Arc::new(UdpSocket::bind(client_addr).await?);
    println!("Server listening on {}", server_addr);
    println!("Client listening on {}", client_addr);
    
    // Start server receiver task
    let server_socket_clone = server_socket.clone();
    let server_task = tokio::spawn(async move {
        println!("Server waiting for packets...");
        let mut buf = vec![0u8; 1500];
        
        // Add a timeout to the receiver
        match tokio::time::timeout(
            Duration::from_secs(5),
            server_socket_clone.recv_from(&mut buf)
        ).await {
            Ok(result) => match result {
                Ok((size, addr)) => {
                    let message = String::from_utf8_lossy(&buf[..size]);
                    println!("Server received {} bytes from {}: {}", size, addr, message);
                    
                    // Send response back
                    let response = "Hello from server!";
                    if let Err(e) = server_socket_clone.send_to(response.as_bytes(), addr).await {
                        println!("Server failed to send response: {}", e);
                    } else {
                        println!("Server sent response to {}", addr);
                    }
                },
                Err(e) => println!("Server receive error: {}", e),
            },
            Err(_) => println!("Server timed out waiting for message"),
        }
    });
    
    // Start client receiver task
    let client_socket_clone = client_socket.clone();
    let client_task = tokio::spawn(async move {
        println!("Client waiting for response...");
        let mut buf = vec![0u8; 1500];
        
        // Add a timeout to the receiver
        match tokio::time::timeout(
            Duration::from_secs(5),
            client_socket_clone.recv_from(&mut buf)
        ).await {
            Ok(result) => match result {
                Ok((size, addr)) => {
                    let message = String::from_utf8_lossy(&buf[..size]);
                    println!("Client received {} bytes from {}: {}", size, addr, message);
                },
                Err(e) => println!("Client receive error: {}", e),
            },
            Err(_) => println!("Client timed out waiting for response"),
        }
    });
    
    // Wait a moment to ensure receiver tasks are running
    println!("Waiting for receiver tasks to start...");
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Send message from client to server
    println!("Client sending message to server...");
    let message = "Hello from client!";
    match client_socket.send_to(message.as_bytes(), server_addr).await {
        Ok(size) => println!("Client sent {} bytes to {}", size, server_addr),
        Err(e) => println!("Client failed to send: {}", e),
    }
    
    // Wait for tasks to complete
    println!("Waiting for tasks to complete...");
    let _ = server_task.await;
    let _ = client_task.await;
    
    println!("UDP test completed!");
    Ok(())
} 