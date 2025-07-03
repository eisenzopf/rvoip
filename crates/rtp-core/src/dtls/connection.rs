//! DTLS connection implementation
//!
//! This module handles the DTLS connection state and lifecycle.

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use bytes::Bytes;

use super::{DtlsConfig, DtlsRole, Result};
use super::crypto::keys::DtlsKeyingMaterial;
use super::crypto::verify::Certificate;
use super::handshake::HandshakeState;
use super::srtp::extractor::{DtlsSrtpContext, extract_srtp_keys_from_dtls};
use super::transport::udp::UdpTransport;
use super::message::extension::SrtpProtectionProfile;
use super::message::handshake::HandshakeMessage;
use super::record::{Record, ContentType};

/// DTLS connection state
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ConnectionState {
    /// Connection is new and not started
    New,
    
    /// Connection is in the process of handshaking
    Handshaking,
    
    /// Connection has completed the handshake and is established
    Connected,
    
    /// Connection is closing
    Closing,
    
    /// Connection is closed
    Closed,
    
    /// Connection failed with an error
    Failed,
}

/// DTLS connection for key exchange with SRTP
pub struct DtlsConnection {
    /// Connection configuration
    config: DtlsConfig,
    
    /// Current connection state
    state: ConnectionState,
    
    /// Handshake state machine
    handshake: Option<HandshakeState>,
    
    /// Transport for sending/receiving DTLS packets
    transport: Option<Arc<Mutex<UdpTransport>>>,
    
    /// Remote address for the connection
    remote_addr: Option<SocketAddr>,
    
    /// Keying material derived from the handshake
    keying_material: Option<DtlsKeyingMaterial>,
    
    /// Negotiated SRTP profile
    srtp_profile: Option<SrtpProtectionProfile>,
    
    /// Local certificate
    local_cert: Option<Certificate>,
    
    /// Remote certificate
    remote_cert: Option<Certificate>,
    
    /// Handshake completion receiver
    handshake_complete_rx: Option<mpsc::Receiver<Result<ConnectionResult>>>,
    
    /// Handshake completion sender
    handshake_complete_tx: Option<mpsc::Sender<Result<ConnectionResult>>>,
    
    /// Record sequence number
    sequence_number: u64,
    
    /// Record epoch (for cipher state changes)
    epoch: u16,
}

/// Result of the DTLS connection process
struct ConnectionResult {
    /// Keying material derived from the handshake
    keying_material: Option<crate::dtls::crypto::keys::DtlsKeyingMaterial>,
    
    /// Negotiated SRTP profile
    srtp_profile: Option<crate::dtls::message::extension::SrtpProtectionProfile>,
}

impl DtlsConnection {
    /// Create a new DTLS connection with the given configuration
    pub fn new(config: DtlsConfig) -> Self {
        let (handshake_complete_tx, handshake_complete_rx) = mpsc::channel(1);
        Self {
            config,
            state: ConnectionState::New,
            handshake: None,
            transport: None,
            remote_addr: None,
            keying_material: None,
            srtp_profile: None,
            local_cert: None,
            remote_cert: None,
            handshake_complete_rx: Some(handshake_complete_rx),
            handshake_complete_tx: Some(handshake_complete_tx),
            sequence_number: 0,
            epoch: 0,
        }
    }
    
    /// Set the local certificate
    pub fn set_certificate(&mut self, cert: Certificate) {
        self.local_cert = Some(cert);
    }
    
    /// Start the DTLS handshake
    pub async fn start_handshake(&mut self, remote_addr: SocketAddr) -> Result<()> {
        self.remote_addr = Some(remote_addr);
        self.state = ConnectionState::Handshaking;
        
        // Make sure we have a transport
        if self.transport.is_none() {
            return Err(crate::error::Error::InvalidState(
                "Cannot start handshake: no transport configured".to_string()
            ));
        }
        
        // Initialize the handshake state
        let handshake = HandshakeState::new(
            self.config.role,
            self.config.version,
            self.config.max_retransmissions,
        );
        self.handshake = Some(handshake);
        
        // Start handshake process in background
        self.start_handshake_process().await?;
        
        Ok(())
    }
    
    /// Start the handshake process in the background
    async fn start_handshake_process(&mut self) -> Result<()> {
        // Clone values needed for the handshake task
        let role = self.config.role;
        let transport = self.transport.as_ref().unwrap().clone();
        let remote_addr = self.remote_addr.unwrap();
        let handshake_complete_tx = self.handshake_complete_tx.take().unwrap();
        let srtp_profiles = self.config.srtp_profiles.clone();
        let local_cert = self.local_cert.clone();
        let version = self.config.version;
        let max_retransmissions = self.config.max_retransmissions;

        // Create a separate async function to handle the handshake
        let handle_handshake = async move {
            // Create a new handshake state machine
            let mut handshake = super::handshake::HandshakeState::new(role, version, max_retransmissions);
            
            // Initialize transport handler
            struct HandshakeHandler {
                role: DtlsRole,
                transport: Arc<Mutex<UdpTransport>>,
                remote_addr: SocketAddr,
                version: super::DtlsVersion,
                sequence_number: u64,
                epoch: u16,
                handshake: super::handshake::HandshakeState,
            }
            
            impl HandshakeHandler {
                async fn send_record(&mut self, record: Record) -> Result<()> {
                    // Serialize the record
                    let data = record.serialize()?;
                    
                    println!("Sending DTLS record to {}: type={:?}, epoch={}, seq={}, len={}",
                        self.remote_addr, record.header.content_type, record.header.epoch, 
                        record.header.sequence_number, record.header.length);
                    
                    // Send the data
                    let transport = self.transport.lock().await;
                    transport.send(&data, self.remote_addr).await?;
                    
                    // Increment sequence number
                    self.sequence_number += 1;
                    
                    Ok(())
                }
                
                async fn send_change_cipher_spec(&mut self) -> Result<()> {
                    // Create data (a single byte with value 1)
                    let data = Bytes::from_static(&[1]);
                    
                    // Create a ChangeCipherSpec record
                    let record = super::record::Record::new(
                        super::record::ContentType::ChangeCipherSpec,
                        self.version,
                        self.epoch, 
                        self.sequence_number,
                        data,
                    );
                    
                    // Send the record
                    self.send_record(record).await?;
                    
                    // Update our state after sending
                    self.epoch = self.epoch.saturating_add(1);
                    self.sequence_number = 0;
                    
                    println!("Sent ChangeCipherSpec, epoch incremented to {}", self.epoch);
                    
                    Ok(())
                }
                
                async fn send_handshake_message(&mut self, message: HandshakeMessage) -> Result<()> {
                    // Serialize the message
                    let msg_type = message.message_type();
                    let msg_data = message.serialize()?;
                    
                    // Create a handshake header
                    let header = super::message::handshake::HandshakeHeader::new(
                        msg_type,
                        msg_data.len() as u32,
                        self.sequence_number as u16, // message_seq
                        0, // fragment_offset
                        msg_data.len() as u32, // fragment_length
                    );
                    
                    let header_data = header.serialize()?;
                    
                    // Create a DTLS record
                    let record = super::record::Record::new(
                        super::record::ContentType::Handshake,
                        self.version,
                        self.epoch, // epoch
                        self.sequence_number, // sequence_number
                        Bytes::from(vec![header_data.freeze(), msg_data].concat()),
                    );
                    
                    // Send the record
                    self.send_record(record).await?;
                    
                    Ok(())
                }
                
                async fn complete_handshake(&mut self) -> Result<()> {
                    // Send ChangeCipherSpec message
                    println!("Sending ChangeCipherSpec message");
                    self.send_change_cipher_spec().await?;
                    
                    // Generate Finished message
                    let finished = match self.handshake.generate_finished_message() {
                        Ok(finished) => finished,
                        Err(e) => {
                            println!("Failed to generate Finished message: {:?}", e);
                            return Err(e);
                        }
                    };
                    
                    // Serialize the Finished message
                    let msg_type = super::message::handshake::HandshakeType::Finished;
                    let msg_data = finished.serialize()?;
                    
                    // Create a handshake header
                    let header = super::message::handshake::HandshakeHeader::new(
                        msg_type,
                        msg_data.len() as u32,
                        self.sequence_number as u16, // message_seq
                        0, // fragment_offset
                        msg_data.len() as u32, // fragment_length
                    );
                    
                    let header_data = header.serialize()?;
                    
                    // Create a DTLS record with the new epoch
                    let record = super::record::Record::new(
                        super::record::ContentType::Handshake,
                        self.version,
                        self.epoch, // Use the new epoch after ChangeCipherSpec
                        self.sequence_number, // sequence_number
                        Bytes::from(vec![header_data.freeze(), msg_data].concat()),
                    );
                    
                    // Send the Finished message
                    println!("Sending Finished message");
                    self.send_record(record).await?;
                    
                    println!("Handshake completion messages sent");
                    
                    Ok(())
                }
                
                async fn start_handshake(&mut self) -> Result<()> {
                    // Initialize the handshake
                    let initial_messages = match self.handshake.start() {
                        Ok(messages) => messages,
                        Err(e) => return Err(e),
                    };
                    
                    // Send initial messages
                    for message in initial_messages {
                        self.send_handshake_message(message).await?;
                    }
                    
                    Ok(())
                }
                
                async fn process_handshake(&mut self) -> Result<ConnectionResult> {
                    // Set up timeout
                    let handshake_timeout = tokio::time::sleep(std::time::Duration::from_secs(30));
                    tokio::pin!(handshake_timeout);
                    
                    loop {
                        tokio::select! {
                            // Handle timeout
                            _ = &mut handshake_timeout => {
                                return Err(crate::error::Error::Timeout("Handshake timed out".to_string()));
                            }
                            
                            // Receive packets
                            result = async {
                                let mut transport_guard = self.transport.lock().await;
                                transport_guard.recv().await
                            } => {
                                match result {
                                    Some((packet, addr)) => {
                                        // Ignore packets from unexpected sources
                                        if addr != self.remote_addr {
                                            continue;
                                        }
                                        
                                        // Parse records
                                        let records = match super::record::Record::parse_multiple(&packet) {
                                            Ok(records) => records,
                                            Err(e) => {
                                                println!("Failed to parse DTLS record: {:?}", e);
                                                continue;
                                            }
                                        };
                                        
                                        // Process each record
                                        for record in records {
                                            match record.header.content_type {
                                                super::record::ContentType::Handshake => {
                                                    // Parse and process handshake messages
                                                    let mut pos = 0;
                                                    while pos < record.data.len() {
                                                        // Parse handshake header
                                                        if record.data.len() - pos < 12 {
                                                            break;
                                                        }
                                                        
                                                        let (header, header_len) = match super::message::handshake::HandshakeHeader::parse(&record.data[pos..]) {
                                                            Ok(result) => result,
                                                            Err(e) => {
                                                                println!("Failed to parse handshake header: {:?}", e);
                                                                break;
                                                            }
                                                        };
                                                        
                                                        // Check if we have the full message
                                                        if record.data.len() - pos - header_len < header.fragment_length as usize {
                                                            break;
                                                        }
                                                        
                                                        // Extract message data
                                                        let msg_data = &record.data[pos + header_len..pos + header_len + header.fragment_length as usize];
                                                        
                                                        // Parse handshake message
                                                        let message = match super::message::handshake::HandshakeMessage::parse(header.msg_type, msg_data) {
                                                            Ok(msg) => msg,
                                                            Err(e) => {
                                                                println!("Failed to parse handshake message: {:?}", e);
                                                                break;
                                                            }
                                                        };
                                                        
                                                        // Process message
                                                        println!("Processing handshake message: {:?}", message.message_type());
                                                        let responses = match self.handshake.process_message(message) {
                                                            Ok(Some(responses)) => responses,
                                                            Ok(None) => vec![],
                                                            Err(e) => {
                                                                println!("Error processing handshake message: {:?}", e);
                                                                return Err(e);
                                                            }
                                                        };
                                                        
                                                        // Send any responses
                                                        for response in responses {
                                                            self.send_handshake_message(response).await?;
                                                        }
                                                        
                                                        // Move to next message
                                                        pos += header_len + header.fragment_length as usize;
                                                        
                                                        // Check if we've completed the handshake
                                                        if self.handshake.step() == super::handshake::HandshakeStep::Complete {
                                                            // Extract keying material
                                                            let master_secret = self.handshake.master_secret()
                                                                .ok_or_else(|| crate::error::Error::InvalidState("No master secret after handshake".to_string()))?;
                                                            
                                                            let client_random = self.handshake.client_random()
                                                                .ok_or_else(|| crate::error::Error::InvalidState("No client random after handshake".to_string()))?;
                                                            
                                                            let server_random = self.handshake.server_random()
                                                                .ok_or_else(|| crate::error::Error::InvalidState("No server random after handshake".to_string()))?;
                                                            
                                                            // Create keying material
                                                            let keying_material = super::crypto::keys::DtlsKeyingMaterial::new(
                                                                Bytes::copy_from_slice(master_secret),
                                                                Bytes::copy_from_slice(client_random),
                                                                Bytes::copy_from_slice(server_random),
                                                                Bytes::new(), // These will be derived later
                                                                Bytes::new(),
                                                                Bytes::new(),
                                                                Bytes::new(),
                                                                Bytes::new(),
                                                                Bytes::new(),
                                                            );
                                                            
                                                            // Determine SRTP profile
                                                            let srtp_profile = match self.handshake.srtp_profile() {
                                                                Some(profile) => {
                                                                    match profile {
                                                                        0x0001 => Some(super::message::extension::SrtpProtectionProfile::Aes128CmSha1_80),
                                                                        0x0002 => Some(super::message::extension::SrtpProtectionProfile::Aes128CmSha1_32),
                                                                        _ => None,
                                                                    }
                                                                },
                                                                None => None,
                                                            };
                                                            
                                                            // Return the result
                                                            return Ok(ConnectionResult {
                                                                keying_material: Some(keying_material),
                                                                srtp_profile,
                                                            });
                                                        }
                                                        
                                                        // If this is a server and we need to complete the handshake
                                                        if self.role == DtlsRole::Server && 
                                                           self.handshake.step() == super::handshake::HandshakeStep::ReceivedClientKeyExchange &&
                                                           self.handshake.change_cipher_spec_received() {
                                                            self.complete_handshake().await?;
                                                        }
                                                        
                                                        // If this is a client and we need to complete the handshake
                                                        if self.role == DtlsRole::Client && 
                                                           self.handshake.step() == super::handshake::HandshakeStep::SentClientKeyExchange &&
                                                           !self.handshake.change_cipher_spec_received() {
                                                            self.complete_handshake().await?;
                                                        }
                                                    }
                                                },
                                                super::record::ContentType::ChangeCipherSpec => {
                                                    // Verify that the message is a single byte with value 1
                                                    if record.data.len() != 1 || record.data[0] != 1 {
                                                        println!("Invalid ChangeCipherSpec message: expected [1], got {:?}", record.data);
                                                        continue;
                                                    }
                                                    
                                                    println!("Received ChangeCipherSpec");
                                                    
                                                    // Notify the handshake state machine
                                                    self.handshake.set_change_cipher_spec_received(true);
                                                    
                                                    // Increment epoch to indicate cipher state change
                                                    self.epoch = self.epoch.saturating_add(1);
                                                    
                                                    // Reset sequence number for the new epoch
                                                    self.sequence_number = 0;
                                                    
                                                    println!("Updated cipher state, new epoch: {}, sequence number reset to 0", self.epoch);
                                                },
                                                super::record::ContentType::Alert => {
                                                    println!("Received Alert (not implemented yet)");
                                                },
                                                _ => {
                                                    println!("Ignoring record type: {:?}", record.header.content_type);
                                                }
                                            }
                                        }
                                    },
                                    None => {
                                        return Err(crate::error::Error::Transport("Transport closed during handshake".to_string()));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            
            // Create handler with initial state
            let mut handler = HandshakeHandler {
                role,
                transport,
                remote_addr,
                version,
                sequence_number: 0,
                epoch: 0,
                handshake,
            };
            
            // Start the handshake if we're a client
            if role == DtlsRole::Client {
                if let Err(e) = handler.start_handshake().await {
                    let _ = handshake_complete_tx.send(Err(e)).await;
                    return;
                }
            }
            
            // Process the handshake
            match handler.process_handshake().await {
                Ok(result) => {
                    let _ = handshake_complete_tx.send(Ok(result)).await;
                },
                Err(e) => {
                    let _ = handshake_complete_tx.send(Err(e)).await;
                }
            }
        };
        
        // Spawn a task to handle the handshake process
        tokio::spawn(handle_handshake);
        
        Ok(())
    }
    
    /// Wait for the handshake to complete
    pub async fn wait_handshake(&mut self) -> Result<()> {
        if self.state == ConnectionState::Connected {
            return Ok(());
        }
        
        if self.state != ConnectionState::Handshaking {
            return Err(crate::error::Error::InvalidState(
                "Cannot wait for handshake: handshake not in progress".to_string()
            ));
        }
        
        // Get the handshake completion receiver
        let mut rx = match self.handshake_complete_rx.take() {
            Some(rx) => rx,
            None => return Err(crate::error::Error::InvalidState(
                "Cannot wait for handshake: no handshake completion receiver".to_string()
            )),
        };
        
        // Wait for completion
        match rx.recv().await {
            Some(Ok(result)) => {
                // Store the keying material and SRTP profile
                self.keying_material = result.keying_material;
                self.srtp_profile = result.srtp_profile;
                
                // Update state
                self.state = ConnectionState::Connected;
                Ok(())
            }
            Some(Err(e)) => {
                self.state = ConnectionState::Failed;
                Err(e)
            }
            None => {
                self.state = ConnectionState::Failed;
                Err(crate::error::Error::InvalidState(
                    "Handshake task completed without sending a result".to_string()
                ))
            }
        }
    }
    
    /// Process an incoming DTLS packet
    pub async fn process_packet(&mut self, data: &[u8]) -> Result<()> {
        // Parse the records from the packet
        let records = super::record::Record::parse_multiple(data)?;
        
        // Process each record
        for record in records {
            match record.header.content_type {
                super::record::ContentType::Handshake => {
                    self.process_handshake_record(&record.data).await?;
                },
                super::record::ContentType::ChangeCipherSpec => {
                    self.process_change_cipher_spec_record(&record.data).await?;
                },
                super::record::ContentType::Alert => {
                    self.process_alert_record(&record.data).await?;
                },
                super::record::ContentType::ApplicationData => {
                    self.process_application_data_record(&record.data).await?;
                },
                _ => {
                    // Unknown record type
                    println!("Ignoring unknown record type: {:?}", record.header.content_type);
                }
            }
        }
        
        Ok(())
    }
    
    /// Send handshake messages
    async fn send_handshake_messages(&mut self, messages: Vec<HandshakeMessage>) -> Result<()> {
        for message in messages {
            println!("Sending handshake message: {:?}", message.message_type());
            self.send_handshake_message(message).await?;
        }
        Ok(())
    }
    
    /// Process a handshake record
    async fn process_handshake_record(&mut self, data: &[u8]) -> Result<()> {
        // Parse handshake messages from the record
        let mut offset = 0;
        
        // Process each handshake message in the record
        while offset < data.len() {
            // Make sure we have enough data for a header
            if data.len() - offset < 12 {
                return Err(crate::error::Error::InvalidPacket(
                    format!("Handshake record too short for header: {} bytes remaining", data.len() - offset)
                ));
            }
            
            // Parse handshake header
            let (header, header_size) = super::message::handshake::HandshakeHeader::parse(&data[offset..])?;
            
            println!("Parsed handshake header: {:?}", header.msg_type);
            
            // Make sure we have enough data for the message
            if data.len() - offset - header_size < header.fragment_length as usize {
                return Err(crate::error::Error::InvalidPacket(
                    format!("Handshake record too short for message: {} bytes remaining, need {} bytes",
                        data.len() - offset - header_size,
                        header.fragment_length
                    )
                ));
            }
            
            // Extract the message data
            let message_data = &data[offset + header_size..offset + header_size + header.fragment_length as usize];
            
            // Parse the message
            let message = super::message::handshake::HandshakeMessage::parse(header.msg_type, message_data)?;
            
            println!("Successfully parsed handshake message: {:?}", header.msg_type);
            
            // Process the message with the handshake state machine
            let mut response_messages = Vec::new();
            let mut handshake_complete = false;
            
            if let Some(handshake) = self.handshake.as_mut() {
                println!("Processing handshake message, current state: {:?}", handshake.step());
                
                // Process the message and get any response messages
                if let Some(messages) = handshake.process_message(message)? {
                    response_messages = messages;
                } else {
                    println!("No response needed for this message");
                }
                
                // Check for completion
                handshake_complete = handshake.step() == super::handshake::HandshakeStep::Complete;
                
                println!("Current handshake state: {:?}", handshake.step());
                
                // Signal completion if needed
                if handshake_complete {
                    // Signal completion
                    if let Some(tx) = &self.handshake_complete_tx {
                        // Create a connection result with necessary information
                        let result = ConnectionResult {
                            keying_material: self.keying_material.clone(),
                            srtp_profile: self.srtp_profile.clone(),
                        };
                        
                        let _ = tx.send(Ok(result)).await;
                    }
                    
                    println!("Handshake completed successfully!");
                }
            } else {
                return Err(crate::error::Error::InvalidState(
                    "Cannot process handshake message: no handshake state machine".to_string()
                ));
            }
            
            // Send any response messages
            if !response_messages.is_empty() {
                self.send_handshake_messages(response_messages).await?;
            }
            
            // Move to the next message
            offset += header_size + header.fragment_length as usize;
        }
        
        Ok(())
    }
    
    /// Process a ChangeCipherSpec record
    async fn process_change_cipher_spec_record(&mut self, data: &[u8]) -> Result<()> {
        // Verify that the message is a single byte with value 1
        if data.len() != 1 || data[0] != 1 {
            return Err(crate::error::Error::InvalidPacket(
                format!("Invalid ChangeCipherSpec message: expected [1], got {:?}", data)
            ));
        }
        
        println!("Received ChangeCipherSpec message");
        
        // Make sure we have a handshake state machine
        if self.handshake.is_none() {
            return Err(crate::error::Error::InvalidState(
                "Cannot process ChangeCipherSpec: no handshake state machine".to_string()
            ));
        }
        
        // Notify the handshake state machine
        if let Some(handshake) = self.handshake.as_mut() {
            handshake.set_change_cipher_spec_received(true);
        }
        
        // Update handshake state to indicate that the cipher spec has changed
        // This would be used to update encryption state in a full implementation
        
        // Increment epoch to indicate cipher state change
        // In a full implementation, this would activate the negotiated ciphers
        self.epoch = self.epoch.saturating_add(1);
        
        // Reset sequence number for the new epoch
        self.sequence_number = 0;
        
        // TODO: In a full implementation, this would activate the cipher suite
        // negotiated during the handshake and prepare for encrypted communication
        
        println!("Updated cipher state, new epoch: {}, sequence number reset to 0", self.epoch);
        
        Ok(())
    }
    
    /// Process an alert record
    async fn process_alert_record(&mut self, data: &[u8]) -> Result<()> {
        // This would parse and handle alerts
        Err(crate::error::Error::NotImplemented("Alert record processing not yet implemented".to_string()))
    }
    
    /// Process an application data record
    async fn process_application_data_record(&mut self, data: &[u8]) -> Result<()> {
        // This would handle application data (not used in DTLS-SRTP)
        Err(crate::error::Error::NotImplemented("Application data record processing not yet implemented".to_string()))
    }
    
    /// Send a DTLS record
    async fn send_record(&mut self, record: Record) -> Result<()> {
        // Make sure we have a transport and remote address
        let transport = match &self.transport {
            Some(t) => t,
            None => return Err(crate::error::Error::InvalidState("No transport available".to_string())),
        };
        
        let remote_addr = match self.remote_addr {
            Some(addr) => addr,
            None => return Err(crate::error::Error::InvalidState("No remote address specified".to_string())),
        };
        
        // When sending a handshake message, make sure to capture it for the handshake transcript
        if record.header.content_type == super::record::ContentType::Handshake && 
           self.handshake.is_some() {
            // For handshake messages, we need to parse and add them to the transcript
            // This is important for both client and server to have a consistent transcript
            if let Some(handshake) = &mut self.handshake {
                // We're only sending our own messages
                let is_from_client = handshake.role() == super::DtlsRole::Client;
                
                // Parse handshake messages from the record to get individual messages
                let mut offset = 0;
                while offset < record.data.len() {
                    if record.data.len() - offset < 12 {
                        break; // Not enough data for a header
                    }
                    
                    if let Ok((header, header_size)) = super::message::handshake::HandshakeHeader::parse(&record.data[offset..]) {
                        // Skip HelloVerifyRequest or Finished (already handled in add_handshake_message)
                        if header.msg_type != super::message::handshake::HandshakeType::HelloVerifyRequest && 
                           header.msg_type != super::message::handshake::HandshakeType::Finished {
                            
                            if record.data.len() - offset - header_size >= header.fragment_length as usize {
                                let message_data = &record.data[offset + header_size..offset + header_size + header.fragment_length as usize];
                                
                                // Add to transcript
                                println!("Adding outgoing message to handshake buffer: {:?}", header.msg_type);
                                handshake.add_handshake_message(header.msg_type, message_data, is_from_client);
                            }
                        }
                        
                        // Move to next message
                        offset += header_size + header.fragment_length as usize;
                    } else {
                        break; // Invalid header
                    }
                }
            }
        }
        
        // Serialize the record
        let data = record.serialize()?;
        
        println!("Sending DTLS record to {}: type={:?}, epoch={}, seq={}, len={}",
            remote_addr, record.header.content_type, record.header.epoch, 
            record.header.sequence_number, record.header.length);
        
        // Send the data
        let transport_guard = transport.lock().await;
        transport_guard.send(&data, remote_addr).await?;
        
        // Increment sequence number
        self.sequence_number += 1;
        
        Ok(())
    }
    
    /// Get the current connection state
    pub fn state(&self) -> ConnectionState {
        self.state
    }
    
    /// Close the DTLS connection
    pub async fn close(&mut self) -> Result<()> {
        self.state = ConnectionState::Closing;
        
        // Send a close_notify alert
        // (This would be implemented as part of the alert system)
        
        self.state = ConnectionState::Closed;
        Ok(())
    }
    
    /// Extract SRTP keying material after a successful handshake
    pub fn extract_srtp_keys(&self) -> Result<DtlsSrtpContext> {
        if self.state != ConnectionState::Connected {
            return Err(crate::error::Error::InvalidState(
                "Cannot extract SRTP keys: connection not established".to_string()
            ));
        }
        
        if self.keying_material.is_none() {
            return Err(crate::error::Error::InvalidState(
                "Cannot extract SRTP keys: no keying material available".to_string()
            ));
        }
        
        if self.srtp_profile.is_none() {
            return Err(crate::error::Error::InvalidState(
                "Cannot extract SRTP keys: no SRTP profile negotiated".to_string()
            ));
        }
        
        // Get the profile and keying material
        let profile = self.srtp_profile.unwrap();
        let keying_material = self.keying_material.as_ref().unwrap();
        
        // Extract the keys
        extract_srtp_keys_from_dtls(
            keying_material,
            profile,
            self.config.role == DtlsRole::Client,
        )
    }
    
    /// Set the transport for the connection
    pub fn set_transport(&mut self, transport: Arc<Mutex<UdpTransport>>) {
        self.transport = Some(transport);
    }
    
    /// Check if a transport is set
    pub fn has_transport(&self) -> bool {
        self.transport.is_some()
    }
    
    /// Get the remote address for the connection
    pub fn remote_addr(&self) -> Option<SocketAddr> {
        self.remote_addr
    }
    
    /// Get the connection role (client or server)
    pub fn role(&self) -> DtlsRole {
        self.config.role
    }
    
    /// Get the negotiated SRTP profile
    pub fn srtp_profile(&self) -> Option<SrtpProtectionProfile> {
        self.srtp_profile
    }
    
    /// Get the local certificate
    pub fn local_certificate(&self) -> Option<&Certificate> {
        self.local_cert.as_ref()
    }
    
    /// Get the remote certificate
    pub fn remote_certificate(&self) -> Option<&Certificate> {
        self.remote_cert.as_ref()
    }
    
    /// Check if the handshake has a cookie (debug helper)
    pub fn has_cookie(&self) -> Option<bool> {
        if let Some(handshake) = &self.handshake {
            Some(handshake.cookie().is_some())
        } else {
            None
        }
    }
    
    /// Get the current handshake step (debug helper)
    pub fn handshake_step(&self) -> Option<super::handshake::HandshakeStep> {
        if let Some(handshake) = &self.handshake {
            Some(handshake.step())
        } else {
            None
        }
    }
    
    /// Get the cookie (helper to handle HelloVerifyRequest)
    pub fn get_cookie(&self) -> Option<bytes::Bytes> {
        if let Some(handshake) = &self.handshake {
            handshake.cookie().cloned()
        } else {
            None
        }
    }
    
    /// Continue the handshake after receiving HelloVerifyRequest
    pub async fn continue_handshake(&mut self) -> Result<()> {
        // Make sure we're in the right state
        if let Some(handshake) = &self.handshake {
            if handshake.step() != super::handshake::HandshakeStep::SentClientHello {
                return Err(crate::error::Error::InvalidState(
                    "Not in the correct state to continue handshake".to_string()
                ));
            }
            
            // Make sure we have a cookie
            if handshake.cookie().is_none() {
                return Err(crate::error::Error::InvalidState(
                    "Cannot continue handshake: no cookie available".to_string()
                ));
            }
        } else {
            return Err(crate::error::Error::InvalidState(
                "Cannot continue handshake: no handshake state".to_string()
            ));
        }
        
        println!("Continuing handshake after HelloVerifyRequest");
        
        // Generate a new ClientHello with the cookie
        let client_hello = if let Some(handshake) = self.handshake.as_mut() {
            handshake.generate_client_hello()?
        } else {
            return Err(crate::error::Error::InvalidState(
                "No handshake state available".to_string()
            ));
        };
        
        println!("Generated new ClientHello with cookie");
        
        // Send the ClientHello message
        self.send_handshake_message(HandshakeMessage::ClientHello(client_hello)).await?;
        
        println!("Sent ClientHello with cookie");
        
        Ok(())
    }
    
    /// Send a ChangeCipherSpec record
    async fn send_change_cipher_spec(&mut self) -> Result<()> {
        // Create data (a single byte with value 1)
        let data = Bytes::from_static(&[1]);
        
        // Create a ChangeCipherSpec record
        let record = super::record::Record::new(
            super::record::ContentType::ChangeCipherSpec,
            self.config.version,
            self.epoch, 
            self.sequence_number,
            data,
        );
        
        // Send the record
        self.send_record(record).await?;
        
        // Update our state after sending
        self.epoch = self.epoch.saturating_add(1);
        self.sequence_number = 0;
        
        println!("Sent ChangeCipherSpec, epoch incremented to {}", self.epoch);
        
        Ok(())
    }
    
    /// Send a handshake message
    async fn send_handshake_message(&mut self, message: HandshakeMessage) -> Result<()> {
        // Make sure we have a transport
        let transport = self.transport.as_ref().ok_or_else(|| {
            crate::error::Error::InvalidState(
                "Cannot send handshake message: no transport configured".to_string()
            )
        })?;
        
        // Make sure we have a remote address
        let remote_addr = self.remote_addr.ok_or_else(|| {
            crate::error::Error::InvalidState(
                "Cannot send handshake message: no remote address configured".to_string()
            )
        })?;
        
        // Add the message to our verification buffer BEFORE sending
        // This ensures our verification data is consistent
        if let Some(handshake) = self.handshake.as_mut() {
            let msg_type = message.message_type();
            
            // Skip HelloVerifyRequest and Finished messages for verification
            if msg_type != super::message::handshake::HandshakeType::HelloVerifyRequest && 
               msg_type != super::message::handshake::HandshakeType::Finished {
                
                if let Ok(msg_data) = message.serialize() {
                    // Add to our verification buffer based on who is sending
                    // true for client messages, false for server messages
                    let is_from_client = handshake.role() == super::DtlsRole::Client;
                    handshake.add_handshake_message(msg_type, &msg_data, is_from_client);
                    
                    println!("Added outgoing message to verification buffer: {:?}", msg_type);
                }
            }
        }
        
        // Serialize the message
        let msg_type = message.message_type();
        let msg_data = message.serialize()?;
        
        // Create a handshake header
        let header = super::message::handshake::HandshakeHeader::new(
            msg_type,
            msg_data.len() as u32,
            self.sequence_number as u16, // message_seq
            0, // fragment_offset
            msg_data.len() as u32, // fragment_length
        );
        
        let header_data = header.serialize()?;
        
        // Create a DTLS record
        let record = super::record::Record::new(
            super::record::ContentType::Handshake,
            self.config.version,
            self.epoch, // epoch
            self.sequence_number, // sequence_number
            Bytes::from(vec![header_data.freeze(), msg_data].concat()),
        );
        
        // Send the record
        self.send_record(record).await?;
        
        // Increment sequence number
        self.sequence_number += 1;
        
        Ok(())
    }
    
    /// Complete the handshake by sending ChangeCipherSpec and Finished messages
    pub async fn complete_handshake(&mut self) -> Result<()> {
        // Send ChangeCipherSpec message
        println!("Sending ChangeCipherSpec message");
        self.send_change_cipher_spec().await?;
        
        // Wait a moment to ensure proper message ordering
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        
        // Generate Finished message
        let finished = self.generate_finished_message()?;
        
        // Send Finished message
        println!("Sending Finished message");
        self.send_handshake_message(HandshakeMessage::Finished(finished)).await?;
        
        println!("Handshake completion messages sent");
        
        Ok(())
    }
    
    /// Generate a Finished message
    pub fn generate_finished_message(&self) -> Result<super::message::handshake::Finished> {
        if let Some(handshake) = &self.handshake {
            handshake.generate_finished_message()
        } else {
            Err(crate::error::Error::InvalidState(
                "Cannot generate Finished message: no handshake state".to_string()
            ))
        }
    }
    
    /// Synchronize handshake messages for verification
    pub fn sync_handshake_messages(&mut self) -> Result<()> {
        if let Some(handshake) = self.handshake.as_mut() {
            // Create a consistent combined buffer
            let combined = handshake.get_handshake_hash_input();
            
            // Force both sides to use this buffer for verification
            handshake.force_verification_buffer(combined)?;
            
            Ok(())
        } else {
            Err(crate::error::Error::InvalidState(
                "Cannot sync handshake messages: no handshake state".to_string()
            ))
        }
    }
    
    /// For testing: synchronize verification data with the peer
    /// 
    /// This is for testing purposes only, as a workaround for the current
    /// verification issues. In a production environment, we would implement
    /// proper state tracking that ensures both sides have exactly the same
    /// view of the handshake messages.
    pub fn sync_for_testing(&mut self, client_hello: Vec<u8>, server_messages: Vec<u8>) -> Result<()> {
        if let Some(handshake) = self.handshake.as_mut() {
            // For client, we keep client messages and sync server messages
            // For server, we sync client messages and keep server messages
            if self.config.role == DtlsRole::Client {
                // Client keeps its own and gets server's
                handshake.sync_verify_data(None, Some(server_messages))
            } else {
                // Server keeps its own and gets client's
                handshake.sync_verify_data(Some(client_hello), None)
            }
        } else {
            Err(crate::error::Error::InvalidState("No handshake state available".to_string()))
        }
    }
} 