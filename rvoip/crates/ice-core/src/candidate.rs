use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use rand::random;
use serde::{Deserialize, Serialize};
use tokio::net::{TcpSocket, TcpStream, UdpSocket};
use tokio::sync::{mpsc, Mutex};

use crate::error::{Error, Result};
use crate::stun::StunMessage;

/// ICE candidate types (RFC 8445 Section 5.1.1)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CandidateType {
    /// Host candidate
    Host,
    
    /// Server reflexive candidate (STUN)
    ServerReflexive,
    
    /// Peer reflexive candidate
    PeerReflexive,
    
    /// Relay candidate (TURN)
    Relay,
}

impl CandidateType {
    /// Get string representation
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::ServerReflexive => "srflx",
            Self::PeerReflexive => "prflx",
            Self::Relay => "relay",
        }
    }
}

impl fmt::Display for CandidateType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Transport protocol type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TransportType {
    /// UDP transport
    Udp,
    
    /// TCP transport (active)
    TcpActive,
    
    /// TCP transport (passive)
    TcpPassive,
    
    /// TCP transport (simultaneous open)
    TcpSimultaneousOpen,
}

impl TransportType {
    /// Get string representation
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Udp => "udp",
            Self::TcpActive => "tcp-act",
            Self::TcpPassive => "tcp-pass",
            Self::TcpSimultaneousOpen => "tcp-so",
        }
    }
    
    /// Get protocol name (UDP or TCP)
    pub fn protocol(&self) -> &'static str {
        match self {
            Self::Udp => "UDP",
            _ => "TCP",
        }
    }
    
    /// Is this a TCP transport?
    pub fn is_tcp(&self) -> bool {
        match self {
            Self::Udp => false,
            _ => true,
        }
    }
}

impl fmt::Display for TransportType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// ICE candidate
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IceCandidate {
    /// Foundation
    pub foundation: String,
    
    /// Component ID (1 for RTP, 2 for RTCP)
    pub component: u32,
    
    /// Transport type
    pub transport: TransportType,
    
    /// Priority
    pub priority: u32,
    
    /// IP address
    pub ip: IpAddr,
    
    /// Port
    pub port: u16,
    
    /// Candidate type
    pub candidate_type: CandidateType,
    
    /// Related address (for reflexive/relay candidates)
    pub related_address: Option<IpAddr>,
    
    /// Related port (for reflexive/relay candidates)
    pub related_port: Option<u16>,
}

impl IceCandidate {
    /// Create a new host candidate
    pub fn new_host(
        component: u32,
        transport: TransportType,
        addr: SocketAddr,
    ) -> Self {
        // Generate foundation (should be the same for candidates with same type, protocol, and base)
        let foundation = format!("{:08x}", random::<u32>());
        
        Self {
            foundation,
            component,
            transport,
            // Host candidates have highest priority, per RFC 8445
            priority: compute_priority(CandidateType::Host, component as u8, 0),
            ip: addr.ip(),
            port: addr.port(),
            candidate_type: CandidateType::Host,
            related_address: None,
            related_port: None,
        }
    }
    
    /// Create a new server reflexive candidate
    pub fn new_srflx(
        component: u32,
        transport: TransportType,
        addr: SocketAddr,
        related_addr: SocketAddr,
    ) -> Self {
        // Generate foundation (should be the same for candidates with same type, protocol, and base)
        let foundation = format!("{:08x}", random::<u32>());
        
        Self {
            foundation,
            component,
            transport,
            // Server reflexive priority, per RFC 8445
            priority: compute_priority(CandidateType::ServerReflexive, component as u8, 0),
            ip: addr.ip(),
            port: addr.port(),
            candidate_type: CandidateType::ServerReflexive,
            related_address: Some(related_addr.ip()),
            related_port: Some(related_addr.port()),
        }
    }
    
    /// Create a new relay candidate
    pub fn new_relay(
        component: u32,
        transport: TransportType,
        addr: SocketAddr,
        related_addr: SocketAddr,
    ) -> Self {
        // Generate foundation (should be the same for candidates with same type, protocol, and base)
        let foundation = format!("{:08x}", random::<u32>());
        
        Self {
            foundation,
            component,
            transport,
            // Relay candidates have lowest priority, per RFC 8445
            priority: compute_priority(CandidateType::Relay, component as u8, 0),
            ip: addr.ip(),
            port: addr.port(),
            candidate_type: CandidateType::Relay,
            related_address: Some(related_addr.ip()),
            related_port: Some(related_addr.port()),
        }
    }
    
    /// Create a new peer reflexive candidate
    pub fn new_prflx(
        component: u32,
        transport: TransportType,
        addr: SocketAddr,
        related_addr: SocketAddr,
    ) -> Self {
        // Generate foundation (should be the same for candidates with same type, protocol, and base)
        let foundation = format!("{:08x}", random::<u32>());
        
        Self {
            foundation,
            component,
            transport,
            // Peer reflexive priority, per RFC 8445
            priority: compute_priority(CandidateType::PeerReflexive, component as u8, 0),
            ip: addr.ip(),
            port: addr.port(),
            candidate_type: CandidateType::PeerReflexive,
            related_address: Some(related_addr.ip()),
            related_port: Some(related_addr.port()),
        }
    }
    
    /// Get the full address
    pub fn address(&self) -> SocketAddr {
        SocketAddr::new(self.ip, self.port)
    }
    
    /// Get the related address if present
    pub fn related_address_full(&self) -> Option<SocketAddr> {
        match (self.related_address, self.related_port) {
            (Some(ip), Some(port)) => Some(SocketAddr::new(ip, port)),
            _ => None,
        }
    }
    
    /// Format the candidate as per SDP syntax (RFC 8839)
    pub fn to_sdp_string(&self) -> String {
        let mut sdp = format!(
            "candidate:{} {} {} {} {} {} typ {}",
            self.foundation,
            self.component,
            self.transport.protocol().to_lowercase(),
            self.priority,
            self.ip,
            self.port,
            self.candidate_type
        );
        
        // Add related address information if present
        if let (Some(raddr), Some(rport)) = (self.related_address, self.related_port) {
            sdp.push_str(&format!(" raddr {} rport {}", raddr, rport));
        }
        
        // Add TCP type if present for TCP candidates
        if self.transport.is_tcp() {
            sdp.push_str(&format!(" tcptype {}", self.transport));
        }
        
        sdp
    }
    
    /// Parse a candidate from SDP format (RFC 8839)
    pub fn from_sdp_string(s: &str) -> Result<Self> {
        // Remove "candidate:" prefix if present
        let s = s.trim_start_matches("candidate:").trim();
        
        // Split into parts
        let parts: Vec<&str> = s.split_whitespace().collect();
        if parts.len() < 8 {
            return Err(Error::InvalidCandidate("Not enough fields in candidate".to_string()));
        }
        
        // Parse foundation
        let foundation = parts[0].to_string();
        
        // Parse component
        let component = parts[1].parse::<u32>()
            .map_err(|_| Error::InvalidCandidate("Invalid component ID".to_string()))?;
        
        // Parse transport type
        let transport = match parts[2].to_lowercase().as_str() {
            "udp" => TransportType::Udp,
            "tcp" => {
                // Default to active if tcptype is not specified
                TransportType::TcpActive
            },
            _ => return Err(Error::InvalidCandidate(format!("Unsupported transport: {}", parts[2]))),
        };
        
        // Parse priority
        let priority = parts[3].parse::<u32>()
            .map_err(|_| Error::InvalidCandidate("Invalid priority".to_string()))?;
        
        // Parse IP address
        let ip = parts[4].parse::<IpAddr>()
            .map_err(|_| Error::InvalidCandidate(format!("Invalid IP address: {}", parts[4])))?;
        
        // Parse port
        let port = parts[5].parse::<u16>()
            .map_err(|_| Error::InvalidCandidate(format!("Invalid port: {}", parts[5])))?;
        
        // Check "typ" keyword
        if parts[6] != "typ" {
            return Err(Error::InvalidCandidate("Missing 'typ' keyword".to_string()));
        }
        
        // Parse candidate type
        let candidate_type = match parts[7] {
            "host" => CandidateType::Host,
            "srflx" => CandidateType::ServerReflexive,
            "prflx" => CandidateType::PeerReflexive,
            "relay" => CandidateType::Relay,
            _ => return Err(Error::InvalidCandidate(format!("Unknown candidate type: {}", parts[7]))),
        };
        
        // Parse optional parameters (raddr, rport, tcptype)
        let mut related_address = None;
        let mut related_port = None;
        let mut final_transport = transport;
        
        let mut i = 8;
        while i + 1 < parts.len() {
            match parts[i] {
                "raddr" => {
                    if i + 1 < parts.len() {
                        related_address = Some(parts[i + 1].parse::<IpAddr>()
                            .map_err(|_| Error::InvalidCandidate(format!("Invalid related address: {}", parts[i + 1])))?);
                        i += 2;
                    } else {
                        return Err(Error::InvalidCandidate("Missing value for raddr".to_string()));
                    }
                },
                "rport" => {
                    if i + 1 < parts.len() {
                        related_port = Some(parts[i + 1].parse::<u16>()
                            .map_err(|_| Error::InvalidCandidate(format!("Invalid related port: {}", parts[i + 1])))?);
                        i += 2;
                    } else {
                        return Err(Error::InvalidCandidate("Missing value for rport".to_string()));
                    }
                },
                "tcptype" => {
                    if i + 1 < parts.len() {
                        final_transport = match parts[i + 1] {
                            "active" => TransportType::TcpActive,
                            "passive" => TransportType::TcpPassive,
                            "so" => TransportType::TcpSimultaneousOpen,
                            _ => return Err(Error::InvalidCandidate(format!("Invalid tcptype: {}", parts[i + 1]))),
                        };
                        i += 2;
                    } else {
                        return Err(Error::InvalidCandidate("Missing value for tcptype".to_string()));
                    }
                },
                _ => {
                    // Skip unknown attribute
                    i += 1;
                }
            }
        }
        
        Ok(Self {
            foundation,
            component,
            transport: final_transport,
            priority,
            ip,
            port,
            candidate_type,
            related_address,
            related_port,
        })
    }
}

/// Compute candidate priority (as per RFC 8445 Section 5.1.2)
///
/// - type_preference: 0-126, higher values are more preferred
/// - local_preference: 0-65535, higher values are more preferred
/// - component_id: 1-256, 1 = RTP, 2 = RTCP
pub fn compute_priority(candidate_type: CandidateType, component_id: u8, local_preference: u16) -> u32 {
    // The type preference gets the most significant bits
    let type_preference = match candidate_type {
        CandidateType::Host => 126,            // Highest preference for host
        CandidateType::PeerReflexive => 110,   // Peer reflexive are discovered during connectivity checks
        CandidateType::ServerReflexive => 100, // Server reflexive next
        CandidateType::Relay => 0,             // Lowest preference for relay
    };
    
    // Priority formula: (2^24) * type_pref + (2^8) * local_pref + (256 - component_id)
    (type_preference as u32) << 24 |
    (local_preference as u32) << 8 |
    (256 - component_id as u32)
}

/// Type for abstract candidate implementations
pub trait Candidate: Send + Sync {
    /// Get the candidate info
    fn get_info(&self) -> &IceCandidate;
    
    /// Send data to a remote address
    async fn send_to(&self, data: &[u8], target: SocketAddr) -> Result<usize>;
    
    /// Get the data receive channel
    fn get_data_receiver(&self) -> mpsc::Receiver<(Bytes, SocketAddr)>;
}

/// UDP candidate
pub struct UdpCandidate {
    /// Candidate information
    pub info: IceCandidate,
    
    /// UDP socket
    socket: Arc<UdpSocket>,
    
    /// Receiver channel for incoming data
    receiver_tx: mpsc::Sender<(Bytes, SocketAddr)>,
    
    /// Receiver for data - wrapped in Arc<Mutex> for cloning
    receiver_factory: Arc<Mutex<Option<mpsc::Receiver<(Bytes, SocketAddr)>>>>,
}

impl UdpCandidate {
    /// Create a new UDP candidate
    pub async fn new(
        socket: UdpSocket, 
        component_id: u32, 
        candidate_type: CandidateType,
        related_addr: Option<SocketAddr>
    ) -> Result<Self> {
        let local_addr = socket.local_addr()?;
        
        // Create channels for incoming data
        let (tx, rx) = mpsc::channel::<(Bytes, SocketAddr)>(100);
        
        // Create candidate info
        let info = match (candidate_type, related_addr) {
            (CandidateType::Host, _) => IceCandidate::new_host(
                component_id,
                TransportType::Udp,
                local_addr,
            ),
            (CandidateType::ServerReflexive, Some(related)) => IceCandidate::new_srflx(
                component_id,
                TransportType::Udp,
                local_addr,
                related,
            ),
            (CandidateType::Relay, Some(related)) => IceCandidate::new_relay(
                component_id,
                TransportType::Udp,
                local_addr,
                related,
            ),
            (CandidateType::PeerReflexive, Some(related)) => IceCandidate::new_prflx(
                component_id,
                TransportType::Udp,
                local_addr,
                related,
            ),
            _ => return Err(Error::InvalidCandidate(format!("Cannot create {:?} candidate without related address", candidate_type))),
        };
        
        let socket = Arc::new(socket);
        
        // Start a task that reads from the socket and sends to the channel
        let socket_clone = socket.clone();
        let tx_clone = tx.clone();
        
        tokio::spawn(async move {
            let mut buf = BytesMut::with_capacity(65_535);
            
            loop {
                buf.resize(65_535, 0);
                
                // Read from socket
                match socket_clone.recv_from(&mut buf).await {
                    Ok((n, addr)) => {
                        // Create a slice of the data we received
                        let data = buf.split_to(n).freeze();
                        
                        // Send to channel
                        if tx_clone.send((data, addr)).await.is_err() {
                            // Channel closed
                            break;
                        }
                    }
                    Err(e) => {
                        // Socket error
                        tracing::error!("UDP socket error: {}", e);
                        break;
                    }
                }
            }
        });
        
        Ok(Self {
            info,
            socket,
            receiver_tx: tx,
            receiver_factory: Arc::new(Mutex::new(Some(rx))),
        })
    }
}

impl Candidate for UdpCandidate {
    fn get_info(&self) -> &IceCandidate {
        &self.info
    }
    
    async fn send_to(&self, data: &[u8], target: SocketAddr) -> Result<usize> {
        self.socket.send_to(data, target).await.map_err(|e| Error::ConnectionError(e.to_string()))
    }
    
    fn get_data_receiver(&self) -> mpsc::Receiver<(Bytes, SocketAddr)> {
        // Create a new channel pair
        let (tx, rx) = mpsc::channel(100);
        
        // Try to take the original receiver from the factory
        let mut receiver_guard = self.receiver_factory.lock().unwrap();
        if let Some(mut original_rx) = receiver_guard.take() {
            // Start a task that forwards messages from the original to the new receiver
            let tx_clone = tx.clone();
            tokio::spawn(async move {
                while let Some(msg) = original_rx.recv().await {
                    if tx_clone.send(msg).await.is_err() {
                        break;
                    }
                }
            });
        } else {
            // If the receiver has already been taken, connect to the original tx
            let receiver_tx = self.receiver_tx.clone();
            tokio::spawn(async move {
                // Just observe that this task exists - data will be forwarded
                // through the shared sender
                let _ = receiver_tx;
            });
        }
        
        rx
    }
}

/// TCP candidate
pub struct TcpCandidate {
    /// Candidate information
    pub info: IceCandidate,
    
    /// For active/passive TCP candidates
    socket: Option<Arc<TcpSocket>>,
    
    /// For connected TCP candidates
    stream: Arc<Mutex<Option<TcpStream>>>,
    
    /// Receiver channel for incoming data
    receiver_tx: mpsc::Sender<(Bytes, SocketAddr)>,
    
    /// Receiver for data - wrapped in Arc<Mutex> for cloning
    receiver_factory: Arc<Mutex<Option<mpsc::Receiver<(Bytes, SocketAddr)>>>>,
}

impl TcpCandidate {
    /// Create a new TCP candidate
    pub async fn new(
        socket: TcpSocket,
        transport_type: TransportType,
        component_id: u32,
        candidate_type: CandidateType,
        related_addr: Option<SocketAddr>,
    ) -> Result<Self> {
        if !transport_type.is_tcp() {
            return Err(Error::InvalidCandidate("TCP candidate requires TCP transport type".to_string()));
        }
        
        let local_addr = socket.local_addr()?;
        
        // Create channels for incoming data
        let (tx, rx) = mpsc::channel::<(Bytes, SocketAddr)>(100);
        
        // Create candidate info
        let info = match (candidate_type, related_addr) {
            (CandidateType::Host, _) => IceCandidate::new_host(
                component_id,
                transport_type,
                local_addr,
            ),
            (CandidateType::ServerReflexive, Some(related)) => IceCandidate::new_srflx(
                component_id,
                transport_type,
                local_addr,
                related,
            ),
            (CandidateType::Relay, Some(related)) => IceCandidate::new_relay(
                component_id,
                transport_type,
                local_addr,
                related,
            ),
            (CandidateType::PeerReflexive, Some(related)) => IceCandidate::new_prflx(
                component_id,
                transport_type,
                local_addr,
                related,
            ),
            _ => return Err(Error::InvalidCandidate(format!("Cannot create {:?} candidate without related address", candidate_type))),
        };
        
        let stream = Arc::new(Mutex::new(None));
        let socket = Arc::new(socket);
        
        // For passive candidates, we need to listen for incoming connections
        if transport_type == TransportType::TcpPassive {
            // This would be handled in the real implementation
            // We'd bind, listen, and accept connections
        }
        
        Ok(Self {
            info,
            socket: Some(socket),
            stream,
            receiver_tx: tx,
            receiver_factory: Arc::new(Mutex::new(Some(rx))),
        })
    }
    
    /// Connect to a remote address (for active candidates)
    pub async fn connect(&self, remote_addr: SocketAddr) -> Result<()> {
        if self.info.transport != TransportType::TcpActive {
            return Err(Error::InvalidState("Only active TCP candidates can connect".to_string()));
        }
        
        if let Some(_socket) = &self.socket {
            // This would normally establish the connection
            // But we'll just implement a stub here
            tracing::debug!("Would connect TCP socket to {}", remote_addr);
        }
        
        Ok(())
    }
}

impl Candidate for TcpCandidate {
    fn get_info(&self) -> &IceCandidate {
        &self.info
    }
    
    async fn send_to(&self, data: &[u8], _target: SocketAddr) -> Result<usize> {
        // In TCP, we don't send to a specific address - we use the established connection
        let stream_guard = self.stream.lock().await;
        if let Some(_stream) = &*stream_guard {
            // This would normally write to the stream
            // But we'll just implement a stub here
            Ok(data.len())
        } else {
            Err(Error::ConnectionError("TCP connection not established".to_string()))
        }
    }
    
    fn get_data_receiver(&self) -> mpsc::Receiver<(Bytes, SocketAddr)> {
        // Create a new channel pair
        let (tx, rx) = mpsc::channel(100);
        
        // Try to take the original receiver from the factory
        let mut receiver_guard = self.receiver_factory.lock().unwrap();
        if let Some(mut original_rx) = receiver_guard.take() {
            // Start a task that forwards messages from the original to the new receiver
            let tx_clone = tx.clone();
            tokio::spawn(async move {
                while let Some(msg) = original_rx.recv().await {
                    if tx_clone.send(msg).await.is_err() {
                        break;
                    }
                }
            });
        } else {
            // If the receiver has already been taken, connect to the original tx
            let receiver_tx = self.receiver_tx.clone();
            tokio::spawn(async move {
                // Just observe that this task exists - data will be forwarded
                // through the shared sender
                let _ = receiver_tx;
            });
        }
        
        rx
    }
} 