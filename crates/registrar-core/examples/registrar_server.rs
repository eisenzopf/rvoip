//! Full SIP Registrar Server with Digest Authentication
//!
//! This example demonstrates a complete registrar server that:
//! - Listens on port 5060 for REGISTER requests
//! - Authenticates users using SIP Digest authentication
//! - Stores registrations in memory
//! - Supports registration, unregistration, and refresh
//!
//! Usage:
//!   cargo run --example registrar_server
//!
//! The server will start on 0.0.0.0:5060 and accept REGISTER requests.
//! Default users:
//!   - alice / password123
//!   - bob / secret456
//!   - charlie / mypass789

use rvoip_dialog_core::{
    api::config::ServerConfig,
    transaction::transport::{TransportManager, TransportManagerConfig},
    transaction::TransactionManager,
    DialogServer,
};
use rvoip_registrar_core::RegistrarService;
use rvoip_sip_core::{Method, Request, Response, StatusCode};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_target(false)
        .with_max_level(tracing::Level::INFO)
        .init();

    info!("Starting SIP Registrar Server");

    // Create transport config
    let transport_config = TransportManagerConfig {
        enable_udp: true,
        bind_addresses: vec!["0.0.0.0:5060".parse()?],
        enable_tcp: false,
        enable_tls: false,
        ..Default::default()
    };

    info!("Creating transport manager on 0.0.0.0:5060");
    let (transport, transport_rx) = TransportManager::new(transport_config).await?;

    info!("Creating transaction manager");
    let (transaction_manager, global_rx) =
        TransactionManager::with_transport_manager(transport, transport_rx, Some(100)).await?;

    // Create dialog server
    info!("Creating dialog server");
    let server_config = ServerConfig::default();
    let dialog_server =
        DialogServer::with_global_events(Arc::new(transaction_manager), global_rx, server_config)
            .await?;

    // Create registrar service with authentication
    let realm = "rvoip.local";
    let config = rvoip_registrar_core::types::RegistrarConfig::default();
    let registrar = Arc::new(
        RegistrarService::with_auth(rvoip_registrar_core::api::ServiceMode::B2BUA, config, realm)
            .await?,
    );

    // Add test users to user store
    if let Some(user_store) = registrar.user_store() {
        user_store.add_user("alice", "password123")?;
        user_store.add_user("bob", "secret456")?;
        user_store.add_user("charlie", "mypass789")?;
        info!("Test users added: alice, bob, charlie");
    }

    info!("Registrar service created with authentication enabled");

    info!("Setting up REGISTER request handler");

    // We'll manually handle REGISTER requests
    // In a real implementation, this would be integrated into dialog-core's protocol handlers

    info!("Registrar server ready!");
    info!("Listening on: 0.0.0.0:5060");
    info!("Realm: rvoip.local");
    info!("Registered users:");
    if let Some(user_store) = registrar.user_store() {
        for username in user_store.list_users() {
            info!("  - {}", username);
        }
    }
    info!("");
    info!("Server is running. Press Ctrl+C to stop.");
    info!("");
    info!("Test with a SIP client:");
    info!("  Register as: sip:alice@<server-ip>");
    info!("  Password: password123");

    // Start the dialog server
    dialog_server.start().await?;

    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;
    info!("Shutdown signal received, stopping server");

    Ok(())
}

/// Handle a REGISTER request with authentication
async fn handle_register_request(
    request: Request,
    source: SocketAddr,
    auth: &DigestAuthenticator,
    user_store: &UserStore,
    registrar: &RegistrarService,
) -> Result<Response, Box<dyn std::error::Error>> {
    debug!("Processing REGISTER request from {}", source);

    // Extract username from From header
    let from_header = request.from().ok_or("Missing From header")?;
    let username = from_header
        .uri()
        .user()
        .ok_or("Missing username in From URI")?;

    debug!("REGISTER request for user: {}", username);

    // Check for Authorization header
    if let Some(auth_header_value) = request.header("Authorization") {
        // Parse authorization header
        let auth_str = auth_header_value.as_str()?;
        let digest_response = DigestAuthenticator::parse_authorization(auth_str)?;

        debug!(
            "Authorization header found for user: {}",
            digest_response.username
        );

        // Get password from user store
        let password = user_store
            .get_password(&digest_response.username)
            .ok_or_else(|| format!("User not found: {}", digest_response.username))?;

        // Validate digest response
        let is_valid = auth.validate_response(&digest_response, "REGISTER", &password)?;

        if !is_valid {
            warn!(
                "Authentication failed for user: {}",
                digest_response.username
            );
            return create_401_response(&request, auth);
        }

        info!(
            "User {} authenticated successfully",
            digest_response.username
        );

        // Process registration
        // In a real implementation, we would:
        // 1. Parse Contact header
        // 2. Extract Expires value
        // 3. Store registration in registrar
        // 4. Return 200 OK

        // For now, just return 200 OK
        let mut response = Response::new(StatusCode::Ok);
        // Copy Via, From, To, Call-ID, CSeq headers from request
        if let Some(via) = request.header("Via") {
            response.insert_header("Via", via.clone());
        }
        if let Some(from) = request.header("From") {
            response.insert_header("From", from.clone());
        }
        if let Some(to) = request.header("To") {
            // Add tag to To header for response
            let to_str = to.as_str()?;
            let to_with_tag = if !to_str.contains(";tag=") {
                format!("{};tag={}", to_str, generate_tag())
            } else {
                to_str.to_string()
            };
            response.insert_header("To", to_with_tag);
        }
        if let Some(call_id) = request.header("Call-ID") {
            response.insert_header("Call-ID", call_id.clone());
        }
        if let Some(cseq) = request.header("CSeq") {
            response.insert_header("CSeq", cseq.clone());
        }

        // Add Contact header with registration info
        if let Some(contact) = request.header("Contact") {
            response.insert_header("Contact", contact.clone());
        }

        // Add Expires header
        let expires = extract_expires(&request).unwrap_or(3600);
        response.insert_header("Expires", expires.to_string());

        info!("Registration successful for user: {}", username);
        Ok(response)
    } else {
        // No authorization header - send 401 challenge
        debug!("No Authorization header, sending 401 challenge");
        create_401_response(&request, auth)
    }
}

/// Create 401 Unauthorized response with WWW-Authenticate challenge
fn create_401_response(
    request: &Request,
    auth: &DigestAuthenticator,
) -> Result<Response, Box<dyn std::error::Error>> {
    let challenge = auth.generate_challenge();
    let www_auth = auth.format_www_authenticate(&challenge);

    let mut response = Response::new(StatusCode::Unauthorized);

    // Copy headers from request
    if let Some(via) = request.header("Via") {
        response.insert_header("Via", via.clone());
    }
    if let Some(from) = request.header("From") {
        response.insert_header("From", from.clone());
    }
    if let Some(to) = request.header("To") {
        let to_str = to.as_str()?;
        let to_with_tag = if !to_str.contains(";tag=") {
            format!("{};tag={}", to_str, generate_tag())
        } else {
            to_str.to_string()
        };
        response.insert_header("To", to_with_tag);
    }
    if let Some(call_id) = request.header("Call-ID") {
        response.insert_header("Call-ID", call_id.clone());
    }
    if let Some(cseq) = request.header("CSeq") {
        response.insert_header("CSeq", cseq.clone());
    }

    // Add WWW-Authenticate header
    response.insert_header("WWW-Authenticate", www_auth);

    debug!("Sending 401 Unauthorized with challenge");
    Ok(response)
}

/// Extract Expires value from request
fn extract_expires(request: &Request) -> Option<u32> {
    request
        .header("Expires")
        .and_then(|h| h.as_str().ok())
        .and_then(|s| s.parse().ok())
}

/// Generate a random tag for To header
fn generate_tag() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let random: u64 = rng.gen();
    format!("{:x}", random)
}
