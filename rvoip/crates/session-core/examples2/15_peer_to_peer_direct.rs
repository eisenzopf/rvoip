//! # 15 - Peer-to-Peer Direct Call
//! 
//! Simple peer-to-peer calling between two SIP clients without a central server.
//! Perfect for direct communication, gaming, and decentralized VoIP applications.

use rvoip_session_core::api::simple::*;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashMap;
use std::net::SocketAddr;
use tokio;

/// Peer-to-peer SIP client that can make direct calls
struct P2PClient {
    session_manager: SessionManager,
    local_address: SocketAddr,
    local_uri: String,
    active_calls: Arc<Mutex<HashMap<String, ActiveCall>>>,
    peers: Arc<Mutex<HashMap<String, PeerInfo>>>,
}

#[derive(Debug, Clone)]
struct PeerInfo {
    uri: String,
    address: SocketAddr,
    display_name: String,
    last_seen: chrono::DateTime<chrono::Utc>,
    online: bool,
}

impl P2PClient {
    async fn new(local_address: SocketAddr, display_name: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let local_uri = format!("sip:{}@{}", display_name.replace(" ", ""), local_address.ip());
        
        let mut config = SessionConfig::default();
        config.set_local_address(local_address);
        config.set_p2p_mode(true); // No registration required
        
        let session_manager = SessionManager::new(config).await?;

        Ok(Self {
            session_manager,
            local_address,
            local_uri,
            active_calls: Arc::new(Mutex::new(HashMap::new())),
            peers: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    async fn start(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("🚀 Starting P2P client at {}", self.local_address);
        println!("📱 Local URI: {}", self.local_uri);

        // Set up incoming call handler
        self.setup_call_handler().await?;

        // Start listening for incoming connections
        self.session_manager.start_p2p_listener(self.local_address).await?;

        // Start peer discovery
        self.start_peer_discovery().await;

        println!("✅ P2P client ready for direct calls");
        Ok(())
    }

    async fn setup_call_handler(&self) -> Result<(), Box<dyn std::error::Error>> {
        let active_calls = self.active_calls.clone();
        let peers = self.peers.clone();

        self.session_manager.set_incoming_call_handler(move |incoming_call| {
            let active_calls = active_calls.clone();
            let peers = peers.clone();

            async move {
                let caller = incoming_call.from();
                let caller_address = incoming_call.source_address();
                
                println!("📞 Incoming P2P call from {} ({})", caller, caller_address);

                // Update peer info
                {
                    let mut peers = peers.lock().await;
                    peers.insert(caller.to_string(), PeerInfo {
                        uri: caller.to_string(),
                        address: caller_address,
                        display_name: extract_display_name(caller),
                        last_seen: chrono::Utc::now(),
                        online: true,
                    });
                }

                // Auto-accept P2P calls (or could prompt user)
                println!("✅ Auto-accepting P2P call");
                CallAction::Answer
            }
        }).await?;

        Ok(())
    }

    async fn make_direct_call(&self, peer_address: SocketAddr, peer_name: &str) -> Result<String, Box<dyn std::error::Error>> {
        let peer_uri = format!("sip:{}@{}", peer_name.replace(" ", ""), peer_address.ip());
        
        println!("📞 Making direct call to {} at {}", peer_name, peer_address);

        let call = self.session_manager
            .make_direct_call(&self.local_uri, &peer_uri, peer_address)
            .await?;

        let call_id = call.id().to_string();

        // Store the active call
        {
            let mut active_calls = self.active_calls.lock().await;
            active_calls.insert(call_id.clone(), call.clone());
        }

        // Set up call event handlers
        self.setup_call_events(&call).await;

        // Update peer info
        {
            let mut peers = self.peers.lock().await;
            peers.insert(peer_uri.clone(), PeerInfo {
                uri: peer_uri,
                address: peer_address,
                display_name: peer_name.to_string(),
                last_seen: chrono::Utc::now(),
                online: true,
            });
        }

        Ok(call_id)
    }

    async fn setup_call_events(&self, call: &ActiveCall) {
        let active_calls = self.active_calls.clone();
        let call_id = call.id().to_string();

        call.on_answered(|call| async move {
            println!("✅ P2P call connected with {}", call.remote_party());
        }).await;

        call.on_ended(move |call, reason| {
            let active_calls = active_calls.clone();
            let call_id = call_id.clone();
            async move {
                println!("📴 P2P call ended with {}: {}", call.remote_party(), reason);
                let mut active_calls = active_calls.lock().await;
                active_calls.remove(&call_id);
            }
        }).await;

        call.on_rejected(|call, reason| async move {
            println!("🚫 P2P call rejected by {}: {}", call.remote_party(), reason);
        }).await;
    }

    async fn start_peer_discovery(&self) {
        // Simple UDP broadcast for peer discovery
        let local_address = self.local_address;
        let local_uri = self.local_uri.clone();
        let peers = self.peers.clone();

        tokio::spawn(async move {
            loop {
                // Broadcast presence every 30 seconds
                Self::broadcast_presence(&local_uri, local_address).await;

                // Clean up old peers
                {
                    let mut peers = peers.lock().await;
                    let cutoff = chrono::Utc::now() - chrono::Duration::minutes(2);
                    peers.retain(|_, peer| peer.last_seen > cutoff);
                }

                tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
            }
        });

        // Listen for peer announcements
        let peers = self.peers.clone();
        tokio::spawn(async move {
            Self::listen_for_peers(peers).await;
        });
    }

    async fn broadcast_presence(local_uri: &str, local_address: SocketAddr) {
        let broadcast_port = local_address.port() + 1000; // Use different port for discovery
        let message = format!("P2P_ANNOUNCE:{}:{}", local_uri, local_address);
        
        if let Ok(socket) = tokio::net::UdpSocket::bind("0.0.0.0:0").await {
            socket.set_broadcast(true).ok();
            
            // Broadcast to local subnet
            let broadcast_addr = match local_address {
                SocketAddr::V4(addr) => {
                    let ip = addr.ip().octets();
                    let broadcast_ip = std::net::Ipv4Addr::new(ip[0], ip[1], ip[2], 255);
                    SocketAddr::V4(std::net::SocketAddrV4::new(broadcast_ip, broadcast_port))
                }
                SocketAddr::V6(_) => return, // IPv6 broadcast not implemented
            };

            socket.send_to(message.as_bytes(), broadcast_addr).await.ok();
        }
    }

    async fn listen_for_peers(peers: Arc<Mutex<HashMap<String, PeerInfo>>>) {
        let socket = match tokio::net::UdpSocket::bind("0.0.0.0:6060").await { // Discovery port
            Ok(socket) => socket,
            Err(_) => return,
        };

        let mut buf = [0; 1024];
        
        loop {
            if let Ok((size, addr)) = socket.recv_from(&mut buf).await {
                if let Ok(message) = std::str::from_utf8(&buf[..size]) {
                    if message.starts_with("P2P_ANNOUNCE:") {
                        if let Some(data) = message.strip_prefix("P2P_ANNOUNCE:") {
                            let parts: Vec<&str> = data.split(':').collect();
                            if parts.len() >= 2 {
                                let peer_uri = parts[0];
                                if let Ok(peer_address) = parts[1..].join(":").parse::<SocketAddr>() {
                                    let mut peers = peers.lock().await;
                                    peers.insert(peer_uri.to_string(), PeerInfo {
                                        uri: peer_uri.to_string(),
                                        address: peer_address,
                                        display_name: extract_display_name(peer_uri),
                                        last_seen: chrono::Utc::now(),
                                        online: true,
                                    });
                                    println!("🔍 Discovered peer: {} at {}", peer_uri, peer_address);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    async fn list_discovered_peers(&self) {
        let peers = self.peers.lock().await;
        println!("\n👥 Discovered Peers:");
        if peers.is_empty() {
            println!("   No peers discovered yet");
            return;
        }

        for (i, peer) in peers.values().enumerate() {
            let status = if peer.online { "🟢" } else { "🔴" };
            println!("{}. {} {} - {} (last seen: {})", 
                i + 1,
                status,
                peer.display_name,
                peer.address,
                peer.last_seen.format("%H:%M:%S")
            );
        }
    }

    async fn call_peer_by_index(&self, index: usize) -> Result<String, Box<dyn std::error::Error>> {
        let peers = self.peers.lock().await;
        let peer = peers.values().nth(index)
            .ok_or("Invalid peer index")?;
        
        let peer_address = peer.address;
        let peer_name = peer.display_name.clone();
        drop(peers);

        self.make_direct_call(peer_address, &peer_name).await
    }

    async fn interactive_mode(&self) -> Result<(), Box<dyn std::error::Error>> {
        use std::io;

        println!("\n🤝 P2P Client - Interactive Mode");
        println!("💡 Commands: peers, call <address:port>, call <peer_number>, quit");
        println!("💡 Example: call 192.168.1.100:5060 or call 1 (for first discovered peer)");

        loop {
            println!("\n> ");
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            let input = input.trim();
            let parts: Vec<&str> = input.split_whitespace().collect();

            if parts.is_empty() {
                continue;
            }

            match parts[0] {
                "peers" => {
                    self.list_discovered_peers().await;
                },
                "call" => {
                    if parts.len() > 1 {
                        let target = parts[1];
                        
                        // Check if it's a peer index
                        if let Ok(index) = target.parse::<usize>() {
                            if index > 0 {
                                match self.call_peer_by_index(index - 1).await {
                                    Ok(call_id) => println!("📞 Call initiated: {}", call_id),
                                    Err(e) => println!("❌ Call failed: {}", e),
                                }
                            }
                        } else if let Ok(address) = target.parse::<SocketAddr>() {
                            // Direct address call
                            match self.make_direct_call(address, "Unknown").await {
                                Ok(call_id) => println!("📞 Call initiated: {}", call_id),
                                Err(e) => println!("❌ Call failed: {}", e),
                            }
                        } else {
                            println!("❌ Invalid address or peer number");
                        }
                    } else {
                        println!("Usage: call <address:port> or call <peer_number>");
                    }
                },
                "quit" => break,
                _ => println!("Unknown command: {}", parts[0]),
            }
        }

        Ok(())
    }
}

fn extract_display_name(uri: &str) -> String {
    // Extract display name from SIP URI
    if let Some(start) = uri.find("sip:") {
        if let Some(end) = uri[start + 4..].find('@') {
            uri[start + 4..start + 4 + end].to_string()
        } else {
            "Unknown".to_string()
        }
    } else {
        "Unknown".to_string()
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Starting P2P Direct Call Client");

    let args: Vec<String> = std::env::args().collect();
    let local_address: SocketAddr = args.get(1)
        .and_then(|addr| addr.parse().ok())
        .unwrap_or_else(|| "0.0.0.0:5060".parse().unwrap());
    
    let display_name = args.get(2)
        .cloned()
        .unwrap_or_else(|| "P2P User".to_string());

    let client = P2PClient::new(local_address, &display_name).await?;
    
    // Start the P2P client
    client.start().await?;

    // Give some time for peer discovery
    println!("🔍 Discovering peers...");
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // Start interactive mode
    client.interactive_mode().await?;

    println!("👋 P2P client shutting down");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_p2p_client_creation() {
        let address = "127.0.0.1:5060".parse().unwrap();
        let client = P2PClient::new(address, "Test User").await;
        assert!(client.is_ok());
    }

    #[test]
    fn test_display_name_extraction() {
        assert_eq!(extract_display_name("sip:alice@192.168.1.100"), "alice");
        assert_eq!(extract_display_name("sip:bob@example.com"), "bob");
    }
} 