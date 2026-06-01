//! TLS SIP server — accepts `sips:` calls over a TLS listener.
//!
//! Set `Config::tls_*` (via `tls_reachable_contact`) and rvoip-sip auto-binds a
//! TLS listener at `sip_port + 1` (RFC 3261's 5060→5061 convention). The cert +
//! key come from `TLS_CERT_PATH` / `TLS_KEY_PATH`, which `run_demo.sh` generates
//! as a one-off CA + server cert (SAN=127.0.0.1). This is one-way TLS: the
//! server only presents its cert; the client decides whether to validate it.

use async_trait::async_trait;
use rvoip_sip::{
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
        println!("[SERVER] ✅ Call {} established over TLS", handle.id());
    }

    async fn on_call_ended(&self, call_id: CallId, reason: EndReason) {
        println!("[SERVER] Call {call_id} ended: {reason:?}");
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_sip_dialog=error".into()),
        )
        .init();

    let cert_path =
        std::env::var("TLS_CERT_PATH").expect("TLS_CERT_PATH must be set (run_demo.sh does this)");
    let key_path =
        std::env::var("TLS_KEY_PATH").expect("TLS_KEY_PATH must be set (run_demo.sh does this)");

    let mut config = Config::local("tls_server", 5060).tls_reachable_contact(
        "127.0.0.1:5061".parse()?,
        cert_path,
        key_path,
    );
    config.local_uri = "sips:tls_server@127.0.0.1:5061;transport=tls".to_string();
    config.contact_uri = Some("sips:tls_server@127.0.0.1:5061;transport=tls".to_string());

    let peer = CallbackPeer::new(TlsLogger, config).await?;

    println!("Listening on sip:127.0.0.1:5060 + sips:127.0.0.1:5061 (TLS)…");
    println!("Press Ctrl+C to stop.");

    tokio::select! {
        res = peer.run() => res?,
        _ = tokio::signal::ctrl_c() => println!("\nShutting down."),
    }

    std::process::exit(0);
}
