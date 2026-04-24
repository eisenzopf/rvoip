//! TLS SIP server — accepts `sips:` calls over a TLS listener.
//!
//! Demonstrates the Sprint 1 A1 TLS client/server path: set
//! `Config::tls_cert_path` + `tls_key_path` and session-core auto-binds
//! a TLS listener at `sip_port + 1` (RFC 3261 5060→5061 convention).
//!
//! The cert path and key path come from the env vars `TLS_CERT_PATH`
//! and `TLS_KEY_PATH`; the run.sh harness generates a shared
//! self-signed cert with `rcgen` and points both peers at it. Because
//! the cert isn't in any system trust store we also set
//! `tls_insecure_skip_verify = true` — this is a dev-only knob and
//! MUST NOT be used against real carriers. For production, omit it and
//! rely on `rustls-native-certs` + `webpki-roots`.
//!
//! Run standalone:  cargo run -p rvoip-session-core --example streampeer_tls_server
//! Or with client:  ./examples/streampeer/tls/run.sh

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

    let mut config = Config::local("tls_server", 5060);
    config.tls_cert_path = Some(cert_path.into());
    config.tls_key_path = Some(key_path.into());
    // Dev-only: accept the matching self-signed cert the client
    // presents. Production code MUST leave this false and rely on the
    // system trust store.
    config.tls_insecure_skip_verify = true;

    let peer = CallbackPeer::new(TlsLogger, config).await?;

    println!("Listening on sip:127.0.0.1:5060 + sips:127.0.0.1:5061 (TLS)…");
    println!("Press Ctrl+C to stop.");

    tokio::select! {
        res = peer.run() => res?,
        _ = tokio::signal::ctrl_c() => println!("\nShutting down."),
    }

    std::process::exit(0);
}
