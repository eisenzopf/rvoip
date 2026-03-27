//! Standalone web console server example.
//!
//! Starts a single binary that exposes:
//!   - HTTP/React UI on :3000
//!   - SIP over UDP  on :5060  (traditional SIP devices / trunks)
//!   - SIP over WebSocket on :8080  (browser softphone — sip-demo.html)
//!
//! Usage:
//!   cargo run -p rvoip-web-console --example web_console_server
//!
//! Requires PostgreSQL 18 running on localhost:5432 (see README).
//! Start with Podman:
//!   podman run -d --name rvoip-postgres \
//!     -e POSTGRES_USER=rvoip -e POSTGRES_PASSWORD=rvoip_dev -e POSTGRES_DB=rvoip \
//!     -p 5432:5432 -v rvoip-pgdata:/var/lib/postgresql/data \
//!     docker.io/library/postgres:18-alpine

use std::sync::Arc;
use rvoip_call_engine::{CallCenterConfig, CallCenterEngine};
use rvoip_call_engine::monitoring::CallCenterEvents;
use rvoip_registrar_core::RegistrarService;
use rvoip_web_console::{WebConsoleServer, server::WebConsoleConfig};
use rvoip_web_console::sip_providers::{DbAuthProvider, DbProxyRouter};

use users_core::SqliteUserStore;
use users_core::jwt::{JwtConfig, JwtIssuer};
use users_core::config::PasswordConfig;
use users_core::AuthenticationService;

const DEFAULT_DATABASE_URL: &str = "postgres://rvoip:rvoip_dev@localhost:5432/rvoip";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "info,rvoip_web_console=debug,rvoip_call_engine=info,rvoip_session_core=info".to_string()),
        )
        .init();

    // -----------------------------------------------------------------------
    // SIP + Call-center engine
    // -----------------------------------------------------------------------
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| DEFAULT_DATABASE_URL.to_string());

    tracing::info!("Connecting to PostgreSQL: {}", database_url);

    // Enable dual transport so the server accepts both:
    //   - Traditional SIP clients (UDP :5060)
    //   - Browser softphones such as sip-demo.html (WebSocket :8080)
    let local_ip = local_ip_or_loopback();
    let mut config = CallCenterConfig::default();
    config.general.enable_websocket = true;
    config.general.local_signaling_addr = "0.0.0.0:5060".parse()?;
    config.general.local_ip = local_ip.clone();

    tracing::info!("SIP UDP  listening on 0.0.0.0:5060  (local IP: {})", local_ip);
    tracing::info!("SIP WS   listening on 0.0.0.0:8080");

    let engine = CallCenterEngine::new(config, Some(database_url.clone())).await?;

    // Start the call-center event loop: processes REGISTER, INVITE, BYE, etc.
    // Without this the SIP stack is listening but call routing is dormant.
    engine.clone().start_event_monitoring().await?;
    tracing::info!("✅ Call-center event monitoring started");

    // -----------------------------------------------------------------------
    // DB-backed SIP auth + proxy routing (dynamic — no restart needed)
    // -----------------------------------------------------------------------
    if let Some(db) = engine.database_manager() {
        // Ensure schema exists.
        if let Err(e) = DbAuthProvider::init_table(db).await {
            tracing::warn!("sip_credentials table init failed: {e}");
        }
        if let Err(e) = DbProxyRouter::init_schema(db).await {
            tracing::warn!("sip_trunks routing_prefix column init failed: {e}");
        }

        let realm = std::env::var("SIP_REALM").unwrap_or_else(|_| "rvoip".to_string());
        let auth_provider = DbAuthProvider::new(db.clone(), realm);
        let proxy_router  = DbProxyRouter::new(db.clone());

        if let Err(e) = engine.set_auth_provider(auth_provider) {
            tracing::warn!("set_auth_provider failed: {e}");
        }
        if let Err(e) = engine.set_proxy_router(proxy_router) {
            tracing::warn!("set_proxy_router failed: {e}");
        }
        tracing::info!("✅ DB-backed SIP auth + proxy routing enabled");
    } else {
        tracing::warn!("No database — SIP auth/routing disabled (all calls accepted without credentials)");
    }

    // -----------------------------------------------------------------------
    // Registrar (tracks SIP registrations — used by the web console UI)
    // -----------------------------------------------------------------------
    let registrar = Arc::new(RegistrarService::new_b2bua().await?);

    // -----------------------------------------------------------------------
    // Real-time event bus (feeds the WebSocket event stream in the UI)
    // -----------------------------------------------------------------------
    let events = Arc::new(CallCenterEvents::new());

    // -----------------------------------------------------------------------
    // JWT / user authentication
    // -----------------------------------------------------------------------
    let jwt_secret = std::env::var("RVOIP_JWT_SECRET")
        .unwrap_or_else(|_| "rvoip-dev-secret-change-me-in-production".to_string());

    let jwt_config = JwtConfig {
        algorithm: "HS256".to_string(),
        signing_key: Some(jwt_secret.clone()),
        ..JwtConfig::default()
    };

    let jwt_issuer = JwtIssuer::new(jwt_config.clone())?;
    let password_config = PasswordConfig::default();

    let user_store = Arc::new(SqliteUserStore::new(&database_url).await?);
    let mut auth_service = AuthenticationService::new(
        user_store.clone(),
        jwt_issuer,
        user_store.clone(),
        password_config,
    )?;
    auth_service.set_pool(user_store.pool().clone());
    let auth_service = Arc::new(auth_service);
    let decoding_key = Arc::new(jsonwebtoken::DecodingKey::from_secret(jwt_secret.as_bytes()));

    // -----------------------------------------------------------------------
    // Web console HTTP server
    // -----------------------------------------------------------------------
    let console_config = WebConsoleConfig {
        // Listen on all interfaces so the UI is reachable from LAN, not just localhost.
        listen_addr: "0.0.0.0:3000".parse()?,
        ..Default::default()
    };

    tracing::info!("Web console listening on http://0.0.0.0:3000");
    tracing::info!("Open http://{}:3000 in your browser", local_ip);
    tracing::info!("SIP softphone demo: http://{}:3000/sip-demo.html", local_ip);

    let server = WebConsoleServer::new(engine, console_config)
        .with_registrar(registrar)
        .with_events(events)
        .with_auth(auth_service, decoding_key, jwt_config);

    server.serve().await
}

/// Attempt to detect the machine's LAN IP address.
/// Falls back to `127.0.0.1` if no non-loopback IPv4 address is found.
fn local_ip_or_loopback() -> String {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
    // Trick: connect a UDP socket to an external address (no traffic is sent)
    // to let the OS pick the outbound interface, then read the local address.
    if let Ok(socket) = UdpSocket::bind("0.0.0.0:0") {
        if socket.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = socket.local_addr() {
                if let IpAddr::V4(ip) = addr.ip() {
                    if !ip.is_loopback() {
                        return ip.to_string();
                    }
                }
            }
        }
    }
    "127.0.0.1".to_string()
}
