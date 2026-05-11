//! SIP-only B2BUA example — uses `rvoip_sip::server::*` exclusively. No
//! `rvoip-core` involvement.
//!
//! This is the consumer pattern for SIP carrier / SIP-only call-center
//! backends: the proven `api::UnifiedCoordinator` for raw call control plus
//! the new `server::*` helpers for B2BUA orchestration (bridge / contact
//! resolver / transfer / b2bua).
//!
//! Run with:
//!
//! ```bash
//! cargo run -p rvoip-sip --example sip_b2bua -- \
//!     --bind 127.0.0.1:5070 \
//!     --upstream sip:upstream@example.com:5060
//! ```
//!
//! On every incoming INVITE the example calls `SipB2bua::handle_inbound` to
//! accept the inbound, originate an outbound leg to `--upstream`, and bridge
//! the two through `media-core`. Drop the returned `BridgeHandle` to tear
//! the bridge down.

use rvoip_sip::api::events::Event;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::server::b2bua::SipB2bua;
use rvoip_sip::server::contact_resolver::{
    ContactRequest, ContactResolver, StaticContactResolver,
};
use std::net::SocketAddr;
use std::sync::Arc;

#[derive(Debug)]
struct Args {
    bind: SocketAddr,
    upstream_aor: String,
    from_uri: String,
}

impl Args {
    fn parse() -> Self {
        let mut bind = "127.0.0.1:5070".parse().unwrap();
        let mut upstream_aor = "sip:upstream@127.0.0.1:5080".to_string();
        let mut from_uri = "sip:b2bua@127.0.0.1:5070".to_string();
        let mut iter = std::env::args().skip(1);
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--bind" => bind = iter.next().unwrap().parse().unwrap(),
                "--upstream" => upstream_aor = iter.next().unwrap(),
                "--from" => from_uri = iter.next().unwrap(),
                "--help" | "-h" => {
                    eprintln!(
                        "usage: sip_b2bua [--bind ADDR] [--upstream URI] [--from URI]"
                    );
                    std::process::exit(0);
                }
                other => {
                    eprintln!("unknown arg: {other}");
                    std::process::exit(2);
                }
            }
        }
        Self {
            bind,
            upstream_aor,
            from_uri,
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info,rvoip_sip_dialog=warn".into()),
        )
        .init();

    let args = Args::parse();
    println!(
        "[B2BUA] bind={} upstream={} from={}",
        args.bind, args.upstream_aor, args.from_uri
    );

    let coordinator = UnifiedCoordinator::new(Config::on(
        "sip-b2bua",
        args.bind.ip(),
        args.bind.port(),
    ))
    .await?;

    // Resolve the upstream URI once at startup. In a real deployment a
    // RegistrarContactResolver would be wired here so the upstream contact
    // is re-resolved per inbound call.
    let resolver = StaticContactResolver;
    let upstream = resolver
        .resolve_contact(&ContactRequest::Static {
            uri: args.upstream_aor.clone(),
        })
        .await?;

    let b2bua = SipB2bua::new(coordinator.clone());
    let mut events = coordinator.events().await?;

    println!("[B2BUA] ready — waiting for inbound INVITEs (Ctrl-C to quit)");

    while let Some(event) = events.next().await {
        match event {
            Event::IncomingCall {
                call_id, from, to, ..
            } => {
                println!("[B2BUA] inbound call from {from} to {to} (session={call_id})");
                let upstream_uri = upstream.uri.clone();
                let from_uri = args.from_uri.clone();
                let b2bua = b2bua.clone();
                tokio::spawn(async move {
                    match b2bua
                        .handle_inbound(&from_uri, &call_id, &upstream_uri)
                        .await
                    {
                        Ok(_handle) => {
                            println!("[B2BUA] bridged session {call_id} → {upstream_uri}");
                            // Hold the BridgeHandle until either leg drops.
                            // In a real B2BUA you'd watch for CallEnded on
                            // both legs and tear down explicitly.
                            tokio::time::sleep(std::time::Duration::from_secs(60 * 60)).await;
                        }
                        Err(err) => {
                            eprintln!("[B2BUA] bridge failed for session {call_id}: {err}");
                        }
                    }
                });
            }
            Event::CallEnded { call_id, reason } => {
                println!("[B2BUA] session {call_id} ended: {reason}");
            }
            Event::CallFailed {
                call_id,
                status_code,
                reason,
            } => {
                println!("[B2BUA] session {call_id} failed: {status_code} {reason}");
            }
            _ => {}
        }
    }

    Ok(())
}

// Marker: `Arc<UnifiedCoordinator>` is what `SipB2bua::new` takes. Keeping
// the import explicit so readers can grep for it.
#[allow(dead_code)]
fn _types_used(coord: Arc<UnifiedCoordinator>) -> SipB2bua {
    SipB2bua::new(coord)
}
