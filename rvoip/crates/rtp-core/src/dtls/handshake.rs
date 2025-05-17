//! DTLS handshake implementation
//!
//! This module handles the DTLS handshake protocol according to RFC 6347.

use bytes::Bytes;
use rand::Rng;

use super::message::handshake::{
    HandshakeMessage, ClientHello, ServerHello, 
    Certificate, ServerKeyExchange, CertificateRequest,
    ClientKeyExchange, CertificateVerify, Finished,
    HelloVerifyRequest
};
use super::message::extension::{Extension, UseSrtpExtension, SrtpProtectionProfile};
use super::{DtlsVersion, DtlsRole, Result};
use super::crypto::cipher::CipherSuiteId;
use super::crypto::keys::{calculate_master_secret, generate_ecdhe_pre_master_secret};

/// Handshake state for DTLS connections
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum HandshakeStep {
    /// Initial state
    Start,
    
    /// Sent ClientHello, waiting for ServerHello
    SentClientHello,
    
    /// Received ClientHello, sending ServerHello
    ReceivedClientHello,
    
    /// Sent HelloVerifyRequest, waiting for ClientHello with cookie
    SentHelloVerifyRequest,
    
    /// Received HelloVerifyRequest, sending ClientHello with cookie
    ReceivedHelloVerifyRequest,
    
    /// Sent ServerHello, waiting for client response
    SentServerHello,
    
    /// Received ServerHello, sending client response
    ReceivedServerHello,
    
    /// Sent client response (Certificate, ClientKeyExchange, etc.), waiting for Finished
    SentClientKeyExchange,
    
    /// Received client response, sending Finished
    ReceivedClientKeyExchange,
    
    /// Sent Finished, waiting for Finished (server)
    SentServerFinished,
    
    /// Sent Finished, waiting for Finished (client)
    SentClientFinished,
    
    /// Handshake complete
    Complete,
    
    /// Handshake failed
    Failed,
}

/// Handshake state machine for DTLS connections
pub struct HandshakeState {
    /// Current handshake step
    step: HandshakeStep,
    
    /// Connection role (client or server)
    role: DtlsRole,
    
    /// DTLS protocol version
    version: DtlsVersion,
    
    /// Client random bytes
    client_random: Option<[u8; 32]>,
    
    /// Server random bytes
    server_random: Option<[u8; 32]>,
    
    /// Pre-master secret
    pre_master_secret: Option<Vec<u8>>,
    
    /// Master secret
    master_secret: Option<Vec<u8>>,
    
    /// Client certificate
    client_certificate: Option<Vec<u8>>,
    
    /// Server certificate
    server_certificate: Option<Vec<u8>>,
    
    /// Negotiated cipher suite
    cipher_suite: Option<u16>,
    
    /// Negotiated compression method
    compression_method: Option<u8>,
    
    /// Handshake message sequence number
    message_seq: u16,
    
    /// Flight timer for retransmission
    retransmission_count: usize,
    
    /// Maximum number of retransmissions
    max_retransmissions: usize,
    
    /// Negotiated SRTP profile
    srtp_profile: Option<u16>,
    
    /// Cookie for DTLS HelloVerifyRequest
    cookie: Option<Bytes>,
    
    /// Session ID
    session_id: Option<Bytes>,
    
    /// Available SRTP profiles
    available_srtp_profiles: Vec<SrtpProtectionProfile>,
    
    /// Local ECDHE private key (P-256)
    local_ecdhe_private_key: Option<p256::SecretKey>,
    
    /// Local ECDHE public key (P-256)
    local_ecdhe_public_key: Option<p256::PublicKey>,
    
    /// Remote ECDHE public key (P-256)
    remote_ecdhe_public_key: Option<Bytes>,
}

impl HandshakeState {
    /// Create a new handshake state machine
    pub fn new(role: DtlsRole, version: DtlsVersion, max_retransmissions: usize) -> Self {
        Self {
            step: HandshakeStep::Start,
            role,
            version,
            client_random: None,
            server_random: None,
            pre_master_secret: None,
            master_secret: None,
            client_certificate: None,
            server_certificate: None,
            cipher_suite: None,
            compression_method: None,
            message_seq: 0,
            retransmission_count: 0,
            max_retransmissions: max_retransmissions,
            srtp_profile: None,
            cookie: None,
            session_id: None,
            available_srtp_profiles: vec![SrtpProtectionProfile::Aes128CmSha1_80],
            local_ecdhe_private_key: None,
            local_ecdhe_public_key: None,
            remote_ecdhe_public_key: None,
        }
    }
    
    /// Get the current handshake step
    pub fn step(&self) -> HandshakeStep {
        self.step
    }
    
    /// Process a handshake message
    pub fn process_message(&mut self, message: HandshakeMessage) -> Result<Option<Vec<HandshakeMessage>>> {
        match self.role {
            DtlsRole::Client => self.process_message_client(message),
            DtlsRole::Server => self.process_message_server(message),
        }
    }

    /// Process a handshake message as a client
    fn process_message_client(&mut self, message: HandshakeMessage) -> Result<Option<Vec<HandshakeMessage>>> {
        match self.step {
            HandshakeStep::SentClientHello => {
                match message {
                    HandshakeMessage::HelloVerifyRequest(hello_verify) => {
                        // Save the cookie
                        self.cookie = Some(hello_verify.cookie.clone());
                        
                        // Update state
                        self.step = HandshakeStep::ReceivedHelloVerifyRequest;
                        
                        // Generate a new ClientHello with the cookie
                        let client_hello = self.generate_client_hello_with_cookie()?;
                        
                        // Update state
                        self.step = HandshakeStep::SentClientHello;
                        
                        Ok(Some(vec![HandshakeMessage::ClientHello(client_hello)]))
                    },
                    HandshakeMessage::ServerHello(server_hello) => {
                        // Save server random
                        self.server_random = Some(server_hello.random);
                        
                        // Save negotiated parameters
                        self.cipher_suite = Some(server_hello.cipher_suite);
                        self.compression_method = Some(server_hello.compression_method);
                        self.session_id = Some(server_hello.session_id.clone());
                        
                        // Check for SRTP extension
                        for ext in &server_hello.extensions {
                            if let Extension::UseSrtp(srtp_ext) = ext {
                                if !srtp_ext.profiles.is_empty() {
                                    // Use the first profile
                                    self.srtp_profile = Some(srtp_ext.profiles[0].into());
                                }
                            }
                        }
                        
                        // Update state
                        self.step = HandshakeStep::ReceivedServerHello;
                        
                        // Must wait for ServerKeyExchange before proceeding
                        Ok(None)
                    },
                    _ => {
                        // Unexpected message
                        self.step = HandshakeStep::Failed;
                        Err(crate::error::Error::DtlsHandshakeError(
                            format!("Unexpected message in state {:?}: {:?}", self.step, message.message_type())
                        ))
                    }
                }
            },
            HandshakeStep::ReceivedServerHello => {
                match message {
                    HandshakeMessage::ServerKeyExchange(server_key_exchange) => {
                        println!("Client received ServerKeyExchange message");
                        
                        // Store server's public key
                        self.remote_ecdhe_public_key = Some(server_key_exchange.public_key.clone());
                        
                        // Generate our own ECDHE key pair
                        let (private_key, public_key) = super::crypto::keys::generate_ecdh_keypair()?;
                        
                        // Store our private key
                        self.local_ecdhe_private_key = Some(private_key);
                        self.local_ecdhe_public_key = Some(public_key);
                        
                        // Encode our public key for transmission
                        let encoded_public_key = super::crypto::keys::encode_public_key(&public_key)?;
                        
                        // Create ClientKeyExchange message with our public key
                        let client_key_exchange = super::message::handshake::ClientKeyExchange::new_ecdhe(encoded_public_key);
                        
                        // Derive the shared secret using ECDHE
                        if let (Some(ref private_key), Some(ref server_public_key)) = 
                            (&self.local_ecdhe_private_key, &self.remote_ecdhe_public_key) {
                            
                            // Encode private key to bytes
                            let private_key_bytes = super::crypto::keys::encode_private_key(private_key)?;
                            
                            // Generate the pre-master secret
                            let pre_master_secret = super::crypto::keys::generate_ecdhe_pre_master_secret(
                                server_public_key,
                                &private_key_bytes,
                            )?;
                            
                            // Save the pre-master secret
                            self.pre_master_secret = Some(pre_master_secret.to_vec());
                            
                            // Calculate master secret
                            if let (Some(client_random), Some(server_random)) = (&self.client_random, &self.server_random) {
                                let master_secret = calculate_master_secret(
                                    &self.pre_master_secret.as_ref().unwrap(),
                                    client_random,
                                    server_random,
                                )?;
                                
                                // Store the master secret
                                self.master_secret = Some(master_secret.to_vec());
                            } else {
                                return Err(crate::error::Error::InvalidState(
                                    "Missing client or server random for master secret calculation".to_string()
                                ));
                            }
                        } else {
                            return Err(crate::error::Error::InvalidState(
                                "Missing keys for ECDHE key exchange".to_string()
                            ));
                        }
                        
                        // Update state
                        self.step = HandshakeStep::SentClientKeyExchange;
                        
                        // In a simplified implementation, mark handshake as complete
                        // In reality, we'd wait for server's Finished message
                        self.step = HandshakeStep::Complete;
                        
                        // Return the ClientKeyExchange message to be sent
                        Ok(Some(vec![HandshakeMessage::ClientKeyExchange(client_key_exchange)]))
                    },
                    _ => {
                        // Unexpected message
                        self.step = HandshakeStep::Failed;
                        Err(crate::error::Error::DtlsHandshakeError(
                            format!("Unexpected message in state {:?}: {:?}", self.step, message.message_type())
                        ))
                    }
                }
            },
            HandshakeStep::ReceivedHelloVerifyRequest => {
                // This state is transient; we should have moved to SentClientHello
                self.step = HandshakeStep::Failed;
                Err(crate::error::Error::DtlsHandshakeError(
                    format!("Unexpected state: {:?}", self.step)
                ))
            },
            HandshakeStep::SentClientKeyExchange => {
                match message {
                    HandshakeMessage::Finished(_) => {
                        // Server has sent Finished message; the handshake is now complete
                        self.step = HandshakeStep::Complete;
                        Ok(None)
                    },
                    _ => {
                        // Unexpected message
                        self.step = HandshakeStep::Failed;
                        Err(crate::error::Error::DtlsHandshakeError(
                            format!("Unexpected message in state {:?}: {:?}", self.step, message.message_type())
                        ))
                    }
                }
            },
            _ => {
                // Unexpected state
                self.step = HandshakeStep::Failed;
                Err(crate::error::Error::DtlsHandshakeError(
                    format!("Unexpected state: {:?}", self.step)
                ))
            }
        }
    }

    /// Process a handshake message as a server
    fn process_message_server(&mut self, message: HandshakeMessage) -> Result<Option<Vec<HandshakeMessage>>> {
        match self.step {
            HandshakeStep::Start => {
                match message {
                    HandshakeMessage::ClientHello(client_hello) => {
                        // Save client random
                        self.client_random = Some(client_hello.random);
                        
                        // Check for cookie
                        if client_hello.cookie.is_empty() {
                            // No cookie - send HelloVerifyRequest
                            self.step = HandshakeStep::ReceivedClientHello;
                            
                            // Generate a cookie (in a real implementation, this would be cryptographically secure)
                            let mut rng = rand::thread_rng();
                            let mut cookie = vec![0u8; 16];
                            rng.fill(&mut cookie[..]);
                            
                            let cookie = Bytes::from(cookie);
                            self.cookie = Some(cookie.clone());
                            
                            // Create HelloVerifyRequest
                            let hello_verify = HelloVerifyRequest::new(
                                self.version,
                                cookie,
                            );
                            
                            // Update state
                            self.step = HandshakeStep::SentHelloVerifyRequest;
                            
                            Ok(Some(vec![HandshakeMessage::HelloVerifyRequest(hello_verify)]))
                        } else {
                            // Cookie present - validate it
                            // (In a real implementation, we'd verify the cookie)
                            
                            // Generate a session ID
                            let mut rng = rand::thread_rng();
                            let mut session_id = vec![0u8; 32];
                            rng.fill(&mut session_id[..]);
                            
                            let session_id = Bytes::from(session_id);
                            self.session_id = Some(session_id.clone());
                            
                            // Select cipher suite (choose the first one we support)
                            let selected_cipher = client_hello.cipher_suites.iter()
                                .find(|&&suite| self.is_supported_cipher_suite(suite))
                                .copied();
                            
                            if let Some(cipher) = selected_cipher {
                                self.cipher_suite = Some(cipher);
                            } else {
                                // No supported cipher suite
                                self.step = HandshakeStep::Failed;
                                return Err(crate::error::Error::DtlsHandshakeError(
                                    "No supported cipher suite".to_string()
                                ));
                            }
                            
                            // Select compression method (always 0 - no compression)
                            self.compression_method = Some(0);
                            
                            // Check for SRTP extension
                            let mut use_srtp_extension = None;
                            
                            for ext in &client_hello.extensions {
                                if let Extension::UseSrtp(srtp_ext) = ext {
                                    // Find the first supported profile
                                    for profile in &srtp_ext.profiles {
                                        if self.available_srtp_profiles.contains(profile) {
                                            self.srtp_profile = Some((*profile).into());
                                            
                                            // Create a new UseSrtp extension with just this profile
                                            use_srtp_extension = Some(UseSrtpExtension::with_profiles(
                                                vec![*profile]
                                            ));
                                            
                                            break;
                                        }
                                    }
                                }
                            }
                            
                            // Create ServerHello
                            let mut extensions = Vec::new();
                            
                            if let Some(srtp_ext) = use_srtp_extension {
                                extensions.push(Extension::UseSrtp(srtp_ext));
                            }
                            
                            let server_hello = ServerHello::new(
                                self.version,
                                session_id,
                                self.cipher_suite.unwrap(),
                                self.compression_method.unwrap(),
                                extensions,
                            );
                            
                            // Save server random
                            self.server_random = Some(server_hello.random);
                            
                            // Generate ECDHE key pair
                            let (private_key, public_key) = super::crypto::keys::generate_ecdh_keypair()?;
                            
                            // Store the private key for later
                            self.local_ecdhe_private_key = Some(private_key);
                            self.local_ecdhe_public_key = Some(public_key);
                            
                            // Encode the public key for transmission
                            let encoded_public_key = super::crypto::keys::encode_public_key(&public_key)?;
                            
                            // Create ServerKeyExchange message with our public key
                            let server_key_exchange = super::message::handshake::ServerKeyExchange::new_ecdhe(encoded_public_key);
                            
                            // Update state
                            self.step = HandshakeStep::SentServerHello;
                            
                            // Send ServerHello and ServerKeyExchange
                            Ok(Some(vec![
                                HandshakeMessage::ServerHello(server_hello),
                                HandshakeMessage::ServerKeyExchange(server_key_exchange),
                            ]))
                        }
                    },
                    _ => {
                        // Unexpected message
                        self.step = HandshakeStep::Failed;
                        Err(crate::error::Error::DtlsHandshakeError(
                            format!("Unexpected message in state {:?}: {:?}", self.step, message.message_type())
                        ))
                    }
                }
            },
            HandshakeStep::SentHelloVerifyRequest => {
                match message {
                    HandshakeMessage::ClientHello(client_hello) => {
                        // Verify cookie
                        if let Some(ref our_cookie) = self.cookie {
                            if &client_hello.cookie != our_cookie {
                                // Invalid cookie
                                self.step = HandshakeStep::Failed;
                                return Err(crate::error::Error::DtlsHandshakeError(
                                    "Invalid cookie".to_string()
                                ));
                            }
                        }
                        
                        // Save client random
                        self.client_random = Some(client_hello.random);
                        
                        // Generate a session ID
                        let mut rng = rand::thread_rng();
                        let mut session_id = vec![0u8; 32];
                        rng.fill(&mut session_id[..]);
                        
                        let session_id = Bytes::from(session_id);
                        self.session_id = Some(session_id.clone());
                        
                        // Select cipher suite (choose the first one we support)
                        let selected_cipher = client_hello.cipher_suites.iter()
                            .find(|&&suite| self.is_supported_cipher_suite(suite))
                            .copied();
                        
                        if let Some(cipher) = selected_cipher {
                            self.cipher_suite = Some(cipher);
                        } else {
                            // No supported cipher suite
                            self.step = HandshakeStep::Failed;
                            return Err(crate::error::Error::DtlsHandshakeError(
                                "No supported cipher suite".to_string()
                            ));
                        }
                        
                        // Select compression method (always 0 - no compression)
                        self.compression_method = Some(0);
                        
                        // Check for SRTP extension
                        let mut use_srtp_extension = None;
                        
                        for ext in &client_hello.extensions {
                            if let Extension::UseSrtp(srtp_ext) = ext {
                                // Find the first supported profile
                                for profile in &srtp_ext.profiles {
                                    if self.available_srtp_profiles.contains(profile) {
                                        self.srtp_profile = Some((*profile).into());
                                        
                                        // Create a new UseSrtp extension with just this profile
                                        use_srtp_extension = Some(UseSrtpExtension::with_profiles(
                                            vec![*profile]
                                        ));
                                        
                                        break;
                                    }
                                }
                            }
                        }
                        
                        // Create ServerHello
                        let mut extensions = Vec::new();
                        
                        if let Some(srtp_ext) = use_srtp_extension {
                            extensions.push(Extension::UseSrtp(srtp_ext));
                        }
                        
                        let server_hello = ServerHello::new(
                            self.version,
                            session_id,
                            self.cipher_suite.unwrap(),
                            self.compression_method.unwrap(),
                            extensions,
                        );
                        
                        // Save server random
                        self.server_random = Some(server_hello.random);
                        
                        // Generate ECDHE key pair
                        let (private_key, public_key) = super::crypto::keys::generate_ecdh_keypair()?;
                        
                        // Store the private key for later
                        self.local_ecdhe_private_key = Some(private_key);
                        self.local_ecdhe_public_key = Some(public_key);
                        
                        // Encode the public key for transmission
                        let encoded_public_key = super::crypto::keys::encode_public_key(&public_key)?;
                        
                        // Create ServerKeyExchange message with our public key
                        let server_key_exchange = super::message::handshake::ServerKeyExchange::new_ecdhe(encoded_public_key);
                        
                        // Update state
                        self.step = HandshakeStep::SentServerHello;
                        
                        // Send ServerHello and ServerKeyExchange
                        Ok(Some(vec![
                            HandshakeMessage::ServerHello(server_hello),
                            HandshakeMessage::ServerKeyExchange(server_key_exchange),
                        ]))
                    },
                    _ => {
                        // Unexpected message
                        self.step = HandshakeStep::Failed;
                        Err(crate::error::Error::DtlsHandshakeError(
                            format!("Unexpected message in state {:?}: {:?}", self.step, message.message_type())
                        ))
                    }
                }
            },
            HandshakeStep::SentServerHello => {
                match message {
                    HandshakeMessage::ClientKeyExchange(client_key_exchange) => {
                        println!("Server received ClientKeyExchange, length: {}", client_key_exchange.exchange_data.len());
                        
                        // Store the client's public key
                        self.remote_ecdhe_public_key = Some(client_key_exchange.exchange_data.clone());
                        
                        // Derive the shared secret using ECDHE
                        if let (Some(ref private_key), Some(ref client_public_key)) = 
                            (&self.local_ecdhe_private_key, &self.remote_ecdhe_public_key) {
                            
                            // Encode private key to bytes
                            let private_key_bytes = super::crypto::keys::encode_private_key(private_key)?;
                            
                            // Generate the pre-master secret
                            let pre_master_secret = super::crypto::keys::generate_ecdhe_pre_master_secret(
                                client_public_key,
                                &private_key_bytes,
                            )?;
                            
                            // Save the pre-master secret
                            self.pre_master_secret = Some(pre_master_secret.to_vec());
                            
                            // Calculate master secret
                            if let (Some(client_random), Some(server_random)) = (&self.client_random, &self.server_random) {
                                let master_secret = calculate_master_secret(
                                    &self.pre_master_secret.as_ref().unwrap(),
                                    client_random,
                                    server_random,
                                )?;
                                
                                // Store the master secret
                                self.master_secret = Some(master_secret.to_vec());
                            } else {
                                return Err(crate::error::Error::InvalidState(
                                    "Missing client or server random for master secret calculation".to_string()
                                ));
                            }
                        } else {
                            return Err(crate::error::Error::InvalidState(
                                "Missing keys for ECDHE key exchange".to_string()
                            ));
                        }
                        
                        // Update state
                        self.step = HandshakeStep::ReceivedClientKeyExchange;
                        
                        // In a simplified implementation, mark handshake as complete
                        // In reality, we'd send a ChangeCipherSpec and Finished message
                        self.step = HandshakeStep::Complete;
                        
                        // No response needed - in a full implementation, we'd send Finished
                        Ok(None)
                    },
                    _ => {
                        // Unexpected message
                        self.step = HandshakeStep::Failed;
                        Err(crate::error::Error::DtlsHandshakeError(
                            format!("Unexpected message in state {:?}: {:?}", self.step, message.message_type())
                        ))
                    }
                }
            },
            _ => {
                // Unexpected state
                self.step = HandshakeStep::Failed;
                Err(crate::error::Error::DtlsHandshakeError(
                    format!("Unexpected state: {:?}", self.step)
                ))
            }
        }
    }
    
    /// Check if a cipher suite is supported
    fn is_supported_cipher_suite(&self, cipher_suite: u16) -> bool {
        // For now, support a small set of common suites
        matches!(
            cipher_suite,
            0xC02B | // TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256
            0xC02F | // TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256
            0xC009 | // TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA
            0xC013 | // TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA
            0x002F   // TLS_RSA_WITH_AES_128_CBC_SHA
        )
    }
    
    /// Generate a ClientHello message with a cookie
    fn generate_client_hello_with_cookie(&mut self) -> Result<ClientHello> {
        // In a real implementation, we'd use the same values as the original ClientHello
        // but add the cookie. For now, we'll create a new one.
        
        let cipher_suites = vec![
            0xC02B, // TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256
            0xC02F, // TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256
            0xC009, // TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA
            0xC013, // TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA
            0x002F, // TLS_RSA_WITH_AES_128_CBC_SHA
        ];
        
        // No compression
        let compression_methods = vec![0];
        
        // Add SRTP extension
        let srtp_extension = UseSrtpExtension::with_profiles(
            vec![SrtpProtectionProfile::Aes128CmSha1_80]
        );
        
        let extensions = vec![
            Extension::UseSrtp(srtp_extension),
        ];
        
        let hello = ClientHello::new(
            self.version,
            Bytes::new(), // Empty session ID
            self.cookie.clone().unwrap_or_else(Bytes::new), // Add the cookie
            cipher_suites,
            compression_methods,
            extensions,
        );
        
        // Save the client random
        if self.client_random.is_none() {
            self.client_random = Some(hello.random);
        }
        
        Ok(hello)
    }
    
    /// Start the handshake process
    pub fn start(&mut self) -> Result<Vec<HandshakeMessage>> {
        match self.role {
            DtlsRole::Client => {
                println!("Starting handshake as CLIENT");
                // Generate ClientHello
                let client_hello = ClientHello::with_defaults(self.version);
                
                // Save the client random
                self.client_random = Some(client_hello.random);
                
                // Update state
                self.step = HandshakeStep::SentClientHello;
                
                Ok(vec![HandshakeMessage::ClientHello(client_hello)])
            }
            DtlsRole::Server => {
                println!("Starting handshake as SERVER");
                // Server waits for ClientHello
                self.step = HandshakeStep::Start;
                Ok(Vec::new())
            }
        }
    }
    
    /// Reset the handshake state
    pub fn reset(&mut self) {
        self.step = HandshakeStep::Start;
        self.client_random = None;
        self.server_random = None;
        self.pre_master_secret = None;
        self.master_secret = None;
        self.client_certificate = None;
        self.server_certificate = None;
        self.cipher_suite = None;
        self.compression_method = None;
        self.message_seq = 0;
        self.retransmission_count = 0;
        self.srtp_profile = None;
        self.cookie = None;
        self.session_id = None;
        self.local_ecdhe_private_key = None;
        self.local_ecdhe_public_key = None;
        self.remote_ecdhe_public_key = None;
    }
    
    /// Get the master secret
    pub fn master_secret(&self) -> Option<&[u8]> {
        self.master_secret.as_deref()
    }
    
    /// Get the client random
    pub fn client_random(&self) -> Option<&[u8; 32]> {
        self.client_random.as_ref()
    }
    
    /// Get the server random
    pub fn server_random(&self) -> Option<&[u8; 32]> {
        self.server_random.as_ref()
    }
    
    /// Get the negotiated SRTP profile
    pub fn srtp_profile(&self) -> Option<u16> {
        self.srtp_profile
    }
} 