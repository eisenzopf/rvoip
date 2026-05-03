//! TLS SIP server — accepts `sips:` calls over a TLS listener.
//!
//! Demonstrates the Sprint 1 A1 TLS client/server path: set
//! `Config::tls_cert_path` + `tls_key_path` and session-core auto-binds
//! a TLS listener at `sip_port + 1` (RFC 3261 5060→5061 convention).
//!
//! The cert + key come from `TLS_CERT_PATH` / `TLS_KEY_PATH`; the
//! run.sh harness generates a one-off CA and a server cert signed by
//! it (with SAN=127.0.0.1) so both the insecure and secure client
//! passes can validate the same cert chain. The server itself doesn't
//! validate any inbound TLS cert in this one-way TLS setup, so no
//! insecure-skip-verify knob is needed here.
//!
//! Run standalone:  cargo run -p rvoip-session-core --example streampeer_tls_server --features dev-insecure-tls
//! Or with client:  ./examples/streampeer/tls/run.sh

#[cfg(not(feature = "dev-insecure-tls"))]
compile_error!(
    "streampeer/tls example requires --features dev-insecure-tls (self-signed dev cert)"
);

use async_trait::async_trait;
use rvoip_session_core::{
    CallHandler, CallHandlerDecision, CallId, CallbackPeer, Config, EndReason, IncomingCall,
    SessionHandle,
};

struct TlsLogger;

#[async_trait]
impl CallHandler for TlsLogger {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        println!("[SERVER] Incoming TLS call: {} -> {}", call.from, call.to);
        CallHandlerDecision::Accept
    }

    async fn on_call_established(&self, handle: SessionHandle) {
        println!("[SERVER] Call {} established over TLS", handle.id());
    }

    async fn on_call_ended(&self, call_id: CallId, reason: EndReason) {
        println!("[SERVER] Call {} ended: {:?}", call_id, reason);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()),
        )
        .init();

    let cert_path = std::env::var("TLS_CERT_PATH")
        .expect("TLS_CERT_PATH must be set (the run.sh script does this)");
    let key_path = std::env::var("TLS_KEY_PATH")
        .expect("TLS_KEY_PATH must be set (the run.sh script does this)");

    let mut config = Config::local("tls_server", 5060).tls_reachable_contact(
        "127.0.0.1:5061".parse()?,
        cert_path,
        key_path,
    );
    config.local_uri = "sips:tls_server@127.0.0.1:5061;transport=tls".to_string();
    config.contact_uri = Some("sips:tls_server@127.0.0.1:5061;transport=tls".to_string());
    // Server side does no TLS validation in this one-way-TLS demo —
    // it only presents its own cert. The client decides whether to
    // validate it (the `streampeer_tls_client` example exercises both
    // modes).

    let peer = CallbackPeer::new(TlsLogger, config).await?;

    println!("Listening on sip:127.0.0.1:5060 + sips:127.0.0.1:5061 (TLS)…");
    println!("Press Ctrl+C to stop.");

    tokio::select! {
        res = peer.run() => res?,
        _ = tokio::signal::ctrl_c() => println!("\nShutting down."),
    }

    std::process::exit(0);
}
