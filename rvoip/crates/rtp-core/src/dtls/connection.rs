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
        
        // Spawn a task to handle the handshake process
        tokio::spawn(async move {
            // Create a new handshake state machine
            let mut handshake = HandshakeState::new(role, version, max_retransmissions);
            
            // Initialize the handshake
            let initial_messages = match handshake.start() {
                Ok(messages) => messages,
                Err(e) => {
                    let _ = handshake_complete_tx.send(Err(e)).await;
                    return;
                }
            };
            
            // Process and send initial messages (ClientHello for clients)
            for message in initial_messages {
                // Serialize the message with a header
                let msg_type = message.message_type();
                let msg_data = match message.serialize() {
                    Ok(data) => data,
                    Err(e) => {
                        let _ = handshake_complete_tx.send(Err(e)).await;
                        return;
                    }
                };
                
                // Create a handshake record
                let header = super::message::handshake::HandshakeHeader::new(
                    msg_type,
                    msg_data.len() as u32,
                    0, // message_seq
                    0, // fragment_offset
                    msg_data.len() as u32, // fragment_length
                );
                
                let header_data = match header.serialize() {
                    Ok(data) => data,
                    Err(e) => {
                        let _ = handshake_complete_tx.send(Err(e)).await;
                        return;
                    }
                };
                
                // Create a DTLS record
                let record = super::record::Record::new(
                    super::record::ContentType::Handshake,
                    version,
                    0, // epoch
                    0, // sequence_number
                    Bytes::from(vec![header_data.freeze(), msg_data].concat()),
                );
                
                // Serialize the record
                let record_data = match record.serialize() {
                    Ok(data) => data,
                    Err(e) => {
                        let _ = handshake_complete_tx.send(Err(e)).await;
                        return;
                    }
                };
                
                // Send the record
                let mut transport_guard = transport.lock().await;
                if let Err(e) = transport_guard.send(&record_data, remote_addr).await {
                    let _ = handshake_complete_tx.send(Err(e)).await;
                    return;
                }
                println!("Sent initial handshake message: {:?}", msg_type);
            }
            
            // Start handshake message exchange loop
            let mut handshake_complete = false;
            let mut sequence_number = 1; // Initial messages used sequence_number 0
            
            // Set timeout for handshake completion
            let handshake_timeout = tokio::time::sleep(std::time::Duration::from_secs(30));
            tokio::pin!(handshake_timeout);
            
            // Message exchange loop
            while !handshake_complete {
                tokio::select! {
                    // Check for timeout
                    _ = &mut handshake_timeout => {
                        println!("Handshake timed out!");
                        let _ = handshake_complete_tx.send(Err(
                            crate::error::Error::Timeout("Handshake timed out".to_string())
                        )).await;
                        return;
                    }
                    
                    // Wait for incoming message
                    recv_result = async {
                        println!("Waiting for handshake message...");
                        let mut transport_guard = transport.lock().await;
                        transport_guard.recv().await
                    } => {
                        match recv_result {
                            Some((packet, addr)) => {
                                println!("Received packet ({} bytes) from {}", packet.len(), addr);
                                
                                // Check that the packet is from the expected peer
                                if addr != remote_addr {
                                    println!("Ignoring packet from unexpected address: {}", addr);
                                    // Ignore packets from other sources
                                    continue;
                                }
                                
                                // Parse the record(s)
                                let records = match super::record::Record::parse_multiple(&packet) {
                                    Ok(records) => {
                                        println!("Successfully parsed {} DTLS records", records.len());
                                        records
                                    },
                                    Err(e) => {
                                        println!("Failed to parse DTLS record: {:?}", e);
                                        // Invalid record, ignore and continue
                                        continue;
                                    }
                                };
                                
                                // Process each record
                                for record in records {
                                    println!("Processing record of type: {:?}", record.header.content_type);
                                    
                                    match record.header.content_type {
                                        super::record::ContentType::Handshake => {
                                            // Parse the handshake message(s)
                                            let mut data = &record.data[..];
                                            
                                            while !data.is_empty() {
                                                // Parse handshake header
                                                let (header, header_size) = match super::message::handshake::HandshakeHeader::parse(data) {
                                                    Ok(result) => {
                                                        println!("Parsed handshake header: {:?}", result.0.msg_type);
                                                        result
                                                    },
                                                    Err(e) => {
                                                        println!("Failed to parse handshake header: {:?}", e);
                                                        break;
                                                    },
                                                };
                                                
                                                // Check we have enough data for the fragment
                                                if data.len() < header_size + header.fragment_length as usize {
                                                    println!("Not enough data for complete handshake message");
                                                    break;
                                                }
                                                
                                                // Extract message data
                                                let msg_data = &data[header_size..header_size + header.fragment_length as usize];
                                                
                                                // Parse the handshake message
                                                let message = match super::message::handshake::HandshakeMessage::parse(header.msg_type, msg_data) {
                                                    Ok(msg) => {
                                                        println!("Successfully parsed handshake message: {:?}", header.msg_type);
                                                        msg
                                                    },
                                                    Err(e) => {
                                                        println!("Failed to parse handshake message: {:?}", e);
                                                        // Skip this message and try the next one
                                                        data = &data[header_size + header.fragment_length as usize..];
                                                        continue;
                                                    }
                                                };
                                                
                                                // Process the message
                                                println!("Processing handshake message, current state: {:?}", handshake.step());
                                                match handshake.process_message(message) {
                                                    Ok(Some(response_messages)) => {
                                                        println!("Generated {} response messages", response_messages.len());
                                                        // Send any response messages
                                                        for response in response_messages {
                                                            println!("Sending response: {:?}", response.message_type());
                                                            // Serialize the message with a header
                                                            let msg_type = response.message_type();
                                                            let resp_data = match response.serialize() {
                                                                Ok(data) => data,
                                                                Err(e) => {
                                                                    println!("Failed to serialize response: {:?}", e);
                                                                    let _ = handshake_complete_tx.send(Err(e)).await;
                                                                    return;
                                                                }
                                                            };
                                                            
                                                            // Create a handshake record
                                                            let resp_header = super::message::handshake::HandshakeHeader::new(
                                                                msg_type,
                                                                resp_data.len() as u32,
                                                                sequence_number as u16, // message_seq
                                                                0, // fragment_offset
                                                                resp_data.len() as u32, // fragment_length
                                                            );
                                                            
                                                            sequence_number += 1;
                                                            
                                                            let resp_header_data = match resp_header.serialize() {
                                                                Ok(data) => data,
                                                                Err(e) => {
                                                                    println!("Failed to serialize header: {:?}", e);
                                                                    let _ = handshake_complete_tx.send(Err(e)).await;
                                                                    return;
                                                                }
                                                            };
                                                            
                                                            // Create a DTLS record
                                                            let resp_record = super::record::Record::new(
                                                                super::record::ContentType::Handshake,
                                                                version,
                                                                0, // epoch
                                                                sequence_number as u64, // sequence_number
                                                                Bytes::from(vec![resp_header_data.freeze(), resp_data].concat()),
                                                            );
                                                            
                                                            // Serialize the record
                                                            let resp_record_data = match resp_record.serialize() {
                                                                Ok(data) => data,
                                                                Err(e) => {
                                                                    println!("Failed to serialize record: {:?}", e);
                                                                    let _ = handshake_complete_tx.send(Err(e)).await;
                                                                    return;
                                                                }
                                                            };
                                                            
                                                            // Send the record
                                                            let mut transport_guard = transport.lock().await;
                                                            if let Err(e) = transport_guard.send(&resp_record_data, remote_addr).await {
                                                                println!("Failed to send response: {:?}", e);
                                                                let _ = handshake_complete_tx.send(Err(e)).await;
                                                                return;
                                                            }
                                                            println!("Sent response message: {:?}", msg_type);
                                                        }
                                                    }
                                                    Ok(None) => {
                                                        println!("No response needed for this message");
                                                    }
                                                    Err(e) => {
                                                        println!("Error processing message: {:?}", e);
                                                        let _ = handshake_complete_tx.send(Err(e)).await;
                                                        return;
                                                    }
                                                }
                                                
                                                // Move to next message
                                                data = &data[header_size + header.fragment_length as usize..];
                                            }
                                        }
                                        super::record::ContentType::ChangeCipherSpec => {
                                            println!("Received ChangeCipherSpec (not implemented yet)");
                                            // In real implementation, handle cipher spec changes
                                        }
                                        super::record::ContentType::Alert => {
                                            println!("Received Alert (not implemented yet)");
                                            // In real implementation, handle alerts
                                        }
                                        _ => {
                                            println!("Ignoring record of type: {:?}", record.header.content_type);
                                            // Ignore other record types during handshake
                                        }
                                    }
                                }
                                
                                // Check if handshake is complete
                                println!("Current handshake state: {:?}", handshake.step());
                                if handshake.step() == super::handshake::HandshakeStep::Complete {
                                    println!("Handshake complete!");
                                    handshake_complete = true;
                                }
                            }
                            None => {
                                println!("Transport closed or error during handshake");
                                // Transport closed or error
                                let _ = handshake_complete_tx.send(Err(
                                    crate::error::Error::Transport("Transport closed during handshake".to_string())
                                )).await;
                                return;
                            }
                        }
                    }
                }
                
                // If handshake is complete, exit the loop
                if handshake_complete {
                    println!("Breaking out of handshake loop");
                    break;
                }
            }
            
            // Handshake completed successfully, get the master secret
            let master_secret = match handshake.master_secret() {
                Some(secret) => Bytes::copy_from_slice(secret),
                None => {
                    let _ = handshake_complete_tx.send(Err(
                        crate::error::Error::InvalidState("No master secret available after handshake".to_string())
                    )).await;
                    return;
                }
            };
            
            // Get client and server random values
            let client_random = match handshake.client_random() {
                Some(random) => Bytes::copy_from_slice(random),
                None => {
                    let _ = handshake_complete_tx.send(Err(
                        crate::error::Error::InvalidState("No client random available after handshake".to_string())
                    )).await;
                    return;
                }
            };
            
            let server_random = match handshake.server_random() {
                Some(random) => Bytes::copy_from_slice(random),
                None => {
                    let _ = handshake_complete_tx.send(Err(
                        crate::error::Error::InvalidState("No server random available after handshake".to_string())
                    )).await;
                    return;
                }
            };
            
            // Derive the key material
            let mac_key_size = 20; // SHA-1 size
            let key_size = 16;    // AES-128 key size
            let iv_size = 16;     // AES-128 IV size
            
            // Use PRF to generate key material
            let key_block = match super::crypto::keys::prf_tls12(
                &master_secret,
                b"key expansion",
                &[&client_random[..], &server_random[..]].concat(),
                2 * mac_key_size + 2 * key_size + 2 * iv_size,
                super::crypto::cipher::HashAlgorithm::Sha256,
            ) {
                Ok(block) => block,
                Err(e) => {
                    let _ = handshake_complete_tx.send(Err(e)).await;
                    return;
                }
            };
            
            // Extract the key materials
            let mut offset = 0;
            
            let client_write_mac_key = Bytes::copy_from_slice(&key_block[offset..offset + mac_key_size]);
            offset += mac_key_size;
            
            let server_write_mac_key = Bytes::copy_from_slice(&key_block[offset..offset + mac_key_size]);
            offset += mac_key_size;
            
            let client_write_key = Bytes::copy_from_slice(&key_block[offset..offset + key_size]);
            offset += key_size;
            
            let server_write_key = Bytes::copy_from_slice(&key_block[offset..offset + key_size]);
            offset += key_size;
            
            let client_write_iv = Bytes::copy_from_slice(&key_block[offset..offset + iv_size]);
            offset += iv_size;
            
            let server_write_iv = Bytes::copy_from_slice(&key_block[offset..offset + iv_size]);
            
            // Create keying material with derived keys
            let keying_material = crate::dtls::crypto::keys::DtlsKeyingMaterial::new(
                master_secret,
                client_random,
                server_random,
                client_write_mac_key,
                server_write_mac_key,
                client_write_key,
                server_write_key,
                client_write_iv,
                server_write_iv,
            );
            
            // Create result with keying material and negotiated SRTP profile
            let srtp_profile = match handshake.srtp_profile() {
                Some(profile) => {
                    match profile {
                        0x0001 => Some(crate::dtls::message::extension::SrtpProtectionProfile::Aes128CmSha1_80),
                        0x0002 => Some(crate::dtls::message::extension::SrtpProtectionProfile::Aes128CmSha1_32),
                        0x0007 => Some(crate::dtls::message::extension::SrtpProtectionProfile::AeadAes128Gcm),
                        0x0008 => Some(crate::dtls::message::extension::SrtpProtectionProfile::AeadAes256Gcm),
                        _ => None,
                    }
                },
                None => {
                    // If no profile was negotiated but config had profiles, use the first one
                    if !srtp_profiles.is_empty() {
                        match srtp_profiles[0] {
                            crate::srtp::SRTP_AES128_CM_SHA1_80 => {
                                Some(crate::dtls::message::extension::SrtpProtectionProfile::Aes128CmSha1_80)
                            },
                            crate::srtp::SRTP_AES128_CM_SHA1_32 => {
                                Some(crate::dtls::message::extension::SrtpProtectionProfile::Aes128CmSha1_32)
                            },
                            _ => None,
                        }
                    } else {
                        None
                    }
                }
            };
            
            // Create connection result
            let conn_result = ConnectionResult {
                keying_material: Some(keying_material),
                srtp_profile,
            };
            
            // Signal handshake completion
            let _ = handshake_complete_tx.send(Ok(conn_result)).await;
        });
        
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
        // Parse the record(s)
        let records = Record::parse_multiple(data)?;
        
        // Process each record
        for record in records {
            match record.header.content_type {
                ContentType::Handshake => {
                    self.process_handshake_record(&record.data).await?;
                }
                ContentType::ChangeCipherSpec => {
                    self.process_change_cipher_spec_record(&record.data).await?;
                }
                ContentType::Alert => {
                    self.process_alert_record(&record.data).await?;
                }
                ContentType::ApplicationData => {
                    self.process_application_data_record(&record.data).await?;
                }
                ContentType::Invalid => {
                    return Err(crate::error::Error::InvalidPacket(
                        "Invalid content type in DTLS record".to_string()
                    ));
                }
            }
        }
        
        Ok(())
    }
    
    /// Process a handshake record
    async fn process_handshake_record(&mut self, data: &[u8]) -> Result<()> {
        // Make sure we have a handshake state machine
        if self.handshake.is_none() {
            return Err(crate::error::Error::InvalidState(
                "Cannot process handshake: no handshake state machine".to_string()
            ));
        }
        
        // Parse the handshake messages
        let mut offset = 0;
        
        while offset < data.len() {
            // Make sure we have enough data for a header
            if data.len() - offset < 12 {
                // Not enough data for a complete handshake header
                break;
            }
            
            // Parse the handshake header
            let (header, header_size) = super::message::handshake::HandshakeHeader::parse(&data[offset..])?;
            
            // Make sure we have enough data for the fragment
            if data.len() - offset < header_size + header.fragment_length as usize {
                // Not enough data for the complete message
                break;
            }
            
            // Extract the message data
            let msg_data = &data[offset + header_size..offset + header_size + header.fragment_length as usize];
            
            // Parse the handshake message
            let message = super::message::handshake::HandshakeMessage::parse(header.msg_type, msg_data)?;
            
            // Process the message in the handshake state
            let response_messages = match self.handshake.as_mut().unwrap().process_message(message) {
                Ok(Some(messages)) => messages,
                Ok(None) => Vec::new(),
                Err(e) => return Err(e),
            };
            
            // Send any response messages
            for response in response_messages {
                // Clone fields we need to avoid self borrows
                let version = self.config.version;
                let seq_num = self.sequence_number;
                let remote_addr = self.remote_addr.unwrap();
                
                // Serialize the message
                let msg_type = response.message_type();
                let msg_data = response.serialize()?;
                
                // Create a handshake header
                let header = super::message::handshake::HandshakeHeader::new(
                    msg_type,
                    msg_data.len() as u32,
                    seq_num as u16, // message_seq
                    0, // fragment_offset
                    msg_data.len() as u32, // fragment_length
                );
                
                let header_data = header.serialize()?;
                
                // Create a DTLS record
                let record = super::record::Record::new(
                    super::record::ContentType::Handshake,
                    version,
                    0, // epoch
                    seq_num, // sequence_number
                    Bytes::from(vec![header_data.freeze(), msg_data].concat()),
                );
                
                // Send the record
                self.send_record(record).await?;
                
                // Increment sequence number
                self.sequence_number += 1;
            }
            
            // Move to the next message
            offset += header_size + header.fragment_length as usize;
        }
        
        Ok(())
    }
    
    /// Process a ChangeCipherSpec record
    async fn process_change_cipher_spec_record(&mut self, data: &[u8]) -> Result<()> {
        // This would handle cipher state changes
        Err(crate::error::Error::NotImplemented("ChangeCipherSpec record processing not yet implemented".to_string()))
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
        
        // Serialize the record
        let data = record.serialize()?;
        
        println!("Sending DTLS record to {}: type={:?}, epoch={}, seq={}, len={}",
            remote_addr, record.header.content_type, record.header.epoch, 
            record.header.sequence_number, record.header.length);
        
        // Send the data
        let transport = transport.lock().await;
        transport.send(&data, remote_addr).await?;
        
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
} 