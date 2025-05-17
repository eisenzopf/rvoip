use std::sync::Arc;
use std::net::SocketAddr;
use tokio::sync::mpsc;
use tokio::time::sleep;
use std::time::Duration;
use std::str::FromStr;
use async_trait::async_trait;
use anyhow::{Result, Context};

// Import SIP types
use rvoip_sip_core::{
    Uri, Message, Method, StatusCode, 
    Request, Response, HeaderName, TypedHeader
};

// Import transport types
use rvoip_sip_transport::{Transport, TransportEvent};
use rvoip_sip_transport::transport::TransportType;
use rvoip_transaction_core::{
    TransactionManager, 
    TransactionEvent, 
    transport::{TransportCapabilities, NetworkInfoForSdp, TransportCapabilitiesExt}
};

// Import session-core helpers for transport information
use rvoip_session_core::helpers::{
    get_transport_capabilities,
    get_transport_info,
    get_network_info_for_sdp,
    get_best_transport_for_uri,
    get_websocket_status,
    create_sdp_offer_with_transport_info
};

// Import media types
use rvoip_session_core::media::AudioCodecType;

/// Advanced transport implementation that supports multiple transport types
#[derive(Debug, Clone)]
struct MultiTransport {
    event_tx: mpsc::Sender<TransportEvent>,
    local_addr: SocketAddr,
    supports_ws: bool,
    supports_tcp: bool,
}

impl MultiTransport {
    fn new(
        event_tx: mpsc::Sender<TransportEvent>, 
        local_addr: SocketAddr,
        supports_ws: bool,
        supports_tcp: bool,
    ) -> Self {
        Self { 
            event_tx, 
            local_addr,
            supports_ws,
            supports_tcp,
        }
    }
    
    fn generate_transport_info(&self) -> String {
        let mut info = String::new();
        info.push_str(&format!("Local address: {}\n", self.local_addr));
        info.push_str(&format!("Supports UDP: true\n"));
        info.push_str(&format!("Supports TCP: {}\n", self.supports_tcp));
        info.push_str(&format!("Supports WebSocket: {}\n", self.supports_ws));
        info
    }
}

#[async_trait::async_trait]
impl Transport for MultiTransport {
    async fn send_message(&self, message: Message, destination: SocketAddr) 
        -> std::result::Result<(), rvoip_sip_transport::error::Error> {
        println!("Transport: Sending {} to {}", 
            if message.is_request() { "request" } else { "response" }, 
            destination);
        
        // Simulate sending the message
        Ok(())
    }
    
    fn local_addr(&self) -> std::result::Result<SocketAddr, rvoip_sip_transport::error::Error> {
        Ok(self.local_addr)
    }
    
    async fn close(&self) -> std::result::Result<(), rvoip_sip_transport::error::Error> {
        println!("Transport: Closing connection");
        Ok(())
    }
    
    fn is_closed(&self) -> bool {
        false
    }
    
    // Override default implementations from Transport trait to match MultiTransport capabilities
    fn supports_udp(&self) -> bool {
        true // Always supports UDP
    }
    
    fn supports_tcp(&self) -> bool {
        self.supports_tcp
    }
    
    fn supports_ws(&self) -> bool {
        self.supports_ws
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Setup logging
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(tracing::Level::DEBUG)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .context("setting default subscriber failed")?;
    
    println!("=== Transport Information Example ===");
    
    // Create transport channels
    let (transport_tx, transport_rx) = mpsc::channel(100);
    
    // Define our local address
    let local_addr: SocketAddr = "192.168.1.100:5060".parse()?;
    
    // Create a multi-transport (simulate having multiple transport options)
    let transport = Arc::new(MultiTransport::new(
        transport_tx,
        local_addr,
        true,  // Supports WebSocket
        true,  // Supports TCP
    ));
    
    // Display basic transport information
    println!("\n-- Basic Transport Information --");
    println!("{}", transport.generate_transport_info());
    
    // Create transaction manager
    let (transaction_manager, _events_rx) = TransactionManager::new(
        transport.clone(),
        transport_rx,
        Some(10)
    ).await.map_err(|e| anyhow::anyhow!("Failed to create transaction manager: {}", e))?;
    let transaction_manager = Arc::new(transaction_manager);
    
    // Demonstrate getting transport capabilities
    println!("\n-- Transport Capabilities --");
    let capabilities = get_transport_capabilities(&transaction_manager);
    print_transport_capabilities(&capabilities);
    
    // Demonstrate getting network information for SDP
    println!("\n-- Network Information for SDP --");
    let network_info = get_network_info_for_sdp(&transaction_manager);
    print_network_info(&network_info);
    
    // Demonstrate getting transport info for specific types
    println!("\n-- Transport Type Details --");
    print_transport_type_info(&transaction_manager, TransportType::Udp);
    print_transport_type_info(&transaction_manager, TransportType::Tcp);
    print_transport_type_info(&transaction_manager, TransportType::Ws);
    
    // Demonstrate determining best transport for URIs
    println!("\n-- Best Transport for URIs --");
    print_best_transport(&transaction_manager, "sip:user@example.com");
    print_best_transport(&transaction_manager, "sips:secure@example.com");
    print_best_transport(&transaction_manager, "ws://ws.example.com");
    
    // Demonstrate WebSocket status
    println!("\n-- WebSocket Status --");
    if let Some(ws_status) = get_websocket_status(&transaction_manager) {
        println!("WS connections: {}", ws_status.ws_connections);
        println!("WSS connections: {}", ws_status.wss_connections);
        println!("Has active connection: {}", ws_status.has_active_connection);
    } else {
        println!("WebSocket transport not supported");
    }
    
    // Demonstrate creating an SDP using transport information
    println!("\n-- SDP Generation with Transport Info --");
    let codecs = vec![AudioCodecType::PCMU, AudioCodecType::PCMA];
    let sdp = create_sdp_offer_with_transport_info(
        &transaction_manager,
        &codecs,
        Some("Transport Example Call")
    );
    
    println!("Generated SDP with correct network information:");
    println!("{}", sdp);
    
    println!("\n=== Example Complete ===");
    
    Ok(())
}

// Helper function to print transport capabilities
fn print_transport_capabilities(capabilities: &TransportCapabilities) {
    println!("Available transports:");
    println!("UDP: {}", capabilities.supports_udp);
    println!("TCP: {}", capabilities.supports_tcp);
    println!("TLS: {}", capabilities.supports_tls);
    println!("WS: {}", capabilities.supports_ws);
    println!("WSS: {}", capabilities.supports_wss);
    
    if let Some(addr) = &capabilities.local_addr {
        println!("Local address: {}", addr);
    }
    
    println!("Default transport: {}", capabilities.default_transport);
}

// Helper function to print network information
fn print_network_info(network_info: &NetworkInfoForSdp) {
    println!("Local IP address: {}", network_info.local_ip);
    println!("RTP port range: {}-{}", 
        network_info.rtp_port_range.0,
        network_info.rtp_port_range.1);
}

// Helper function to print transport type information
fn print_transport_type_info(transaction_manager: &Arc<TransactionManager>, transport_type: TransportType) {
    println!("Information for {}: ", transport_type);
    
    if let Some(info) = get_transport_info(transaction_manager, transport_type) {
        println!("  Is connected: {}", info.is_connected);
        println!("  Connection count: {}", info.connection_count);
        if let Some(addr) = &info.local_addr {
            println!("  Local address: {}", addr);
        } else {
            println!("  Local address: Not available");
        }
    } else {
        println!("  Not supported");
    }
}

// Helper function to print best transport for a URI
fn print_best_transport(transaction_manager: &Arc<TransactionManager>, uri_str: &str) {
    match Uri::from_str(uri_str) {
        Ok(uri) => {
            let best_transport = get_best_transport_for_uri(transaction_manager, &uri);
            println!("Best transport for {}: {}", uri, best_transport);
        },
        Err(e) => {
            println!("Invalid URI {}: {}", uri_str, e);
        }
    }
} 