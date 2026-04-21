//! NOTIFY send — Bob.
//!
//! Bob auto-accepts an incoming call (via `StreamPeer::accept_call`)
//! and listens on the event stream for `Event::NotifyReceived` with the
//! expected fields. Exits 0 on match, 1 on timeout or mismatch.

use rvoip_session_core::{Config, Event, StreamPeer};
use tokio::time::{timeout, Duration};

fn env_port(key: &str, default: u16) -> u16 {
    std::env::var(key).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()),
        )
        .init();

    let bob_port = env_port("BOB_PORT", 35092);
    let config = Config::local("bob", bob_port);
    let mut bob = StreamPeer::with_config(config).await?;
    let mut events = bob.control().subscribe_events().await?;

    println!("[BOB] Listening on {}…", bob_port);

    let incoming = timeout(Duration::from_secs(8), bob.wait_for_incoming())
        .await
        .map_err(|_| "never saw incoming call")??;
    println!("[BOB] Incoming call from {} — accepting", incoming.from);
    incoming.accept().await?;

    let outcome = timeout(Duration::from_secs(15), async {
        loop {
            match events.next().await {
                Some(Event::NotifyReceived {
                    event_package,
                    subscription_state,
                    content_type: _,
                    body,
                    ..
                }) => {
                    println!(
                        "[BOB] Got NotifyReceived(event={}, sub_state={:?}, body_len={})",
                        event_package,
                        subscription_state,
                        body.as_deref().map(str::len).unwrap_or(0)
                    );
                    if event_package != "dialog" {
                        return Err(format!(
                            "unexpected event_package: {}",
                            event_package
                        ));
                    }
                    let sub = subscription_state.unwrap_or_default();
                    if !sub.contains("active") {
                        return Err(format!(
                            "expected Subscription-State containing 'active', got: {}",
                            sub
                        ));
                    }
                    return Ok(());
                }
                Some(Event::CallEnded { .. }) => {
                    return Err("call ended before NOTIFY arrived".into());
                }
                Some(_) => continue,
                None => return Err("event stream closed".into()),
            }
        }
    })
    .await;

    match outcome {
        Ok(Ok(())) => {
            println!("[BOB] Observed expected NotifyReceived — exiting 0.");
            std::process::exit(0);
        }
        Ok(Err(e)) => {
            eprintln!("[BOB] assertion failed: {}", e);
            std::process::exit(1);
        }
        Err(_) => {
            eprintln!("[BOB] timed out waiting for NotifyReceived");
            std::process::exit(1);
        }
    }
}
