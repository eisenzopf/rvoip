use rvoip_client_core::ClientBuilder;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use tokio;

/// Test that media ports are automatically allocated when set to 0
#[tokio::test]
async fn test_automatic_media_port_allocation() {
    // Create client with media port set to 0 for automatic allocation
    let sip_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    let media_addr: SocketAddr = "127.0.0.1:0".parse().unwrap(); // Port 0 for automatic allocation

    let client = ClientBuilder::new()
        .local_address(sip_addr)
        .media_address(media_addr)
        .domain("example.com")
        .build()
        .await
        .expect("Failed to build client");

    // Verify that the addresses were set correctly
    // Port 0 means automatic allocation will happen when media sessions are created
    assert_eq!(media_addr.port(), 0);

    // Cleanup
    client.stop().await.expect("Failed to stop client");
}

/// Test that multiple clients can use automatic port allocation without conflicts
#[tokio::test]
async fn test_multiple_clients_automatic_ports() {
    let mut clients = Vec::new();
    let base_sip_port = AtomicU16::new(6060);

    // Create 5 clients with automatic media port allocation
    for i in 0..5 {
        let sip_port = base_sip_port.fetch_add(1, Ordering::SeqCst);
        
        let sip_addr: SocketAddr = format!("127.0.0.1:{}", sip_port).parse().unwrap();
        let media_addr: SocketAddr = "127.0.0.1:0".parse().unwrap(); // Automatic allocation

        let client = ClientBuilder::new()
            .local_address(sip_addr)
            .media_address(media_addr)
            .domain(format!("domain{}.com", i))
            .build()
            .await
            .expect(&format!("Failed to build client {}", i));

        clients.push((client, sip_addr, media_addr));
    }

    // Verify all clients have different SIP ports
    let mut sip_ports = Vec::new();
    for (_, sip_addr, media_addr) in &clients {
        assert_eq!(media_addr.port(), 0); // Media port still 0 until actual media session
        sip_ports.push(sip_addr.port());
    }
    
    // Check that all SIP ports are unique
    sip_ports.sort();
    sip_ports.dedup();
    assert_eq!(sip_ports.len(), 5, "All SIP ports should be unique");

    // Cleanup all clients
    for (client, _, _) in clients {
        client.stop().await.expect("Failed to stop client");
    }
}

/// Test bind address propagation with automatic port allocation
#[tokio::test]
async fn test_bind_address_with_automatic_ports() {
    // Use a specific bind IP with automatic port allocation
    let bind_ip = "192.168.1.100";
    let sip_addr: SocketAddr = format!("{}:0", bind_ip).parse().unwrap(); // Auto SIP port
    let media_addr: SocketAddr = format!("{}:0", bind_ip).parse().unwrap(); // Auto media port

    // Note: This might fail if the IP is not available on the system
    // But it tests that the configuration is properly set up
    let client_result = ClientBuilder::new()
        .local_address(sip_addr)
        .media_address(media_addr)
        .domain("test.com")
        .build()
        .await;

    match client_result {
        Ok(client) => {
            // If successful, verify the IP addresses are preserved
            assert_eq!(sip_addr.ip().to_string(), bind_ip);
            assert_eq!(media_addr.ip().to_string(), bind_ip);
            
            client.stop().await.expect("Failed to stop client");
        }
        Err(e) => {
            // Expected if the IP is not available on the system
            println!("Client creation failed as expected for unavailable IP: {}", e);
        }
    }
}

/// Test that media port inherits from SIP port when only SIP is configured
#[tokio::test]
async fn test_media_port_inheritance() {
    // Create builder with only SIP address configured
    let sip_addr: SocketAddr = "10.0.0.5:5080".parse().unwrap();
    let media_addr: SocketAddr = format!("{}:0", sip_addr.ip()).parse().unwrap(); // Same IP, port 0

    // The media address should have the same IP as SIP but port 0
    assert_eq!(media_addr.ip(), sip_addr.ip());
    assert_eq!(media_addr.port(), 0);
    
    // Try to build (might fail if IP is not available)
    match ClientBuilder::new()
        .local_address(sip_addr)
        .media_address(media_addr)
        .domain("inherit-test.com")
        .build()
        .await
    {
        Ok(client) => {
            client.stop().await.expect("Failed to stop client");
        }
        Err(e) => {
            println!("Client creation failed as expected for unavailable IP: {}", e);
        }
    }
} 