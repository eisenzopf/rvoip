//! Receive command - Wait for and handle incoming calls

use std::time::Duration;
use tracing::{info, warn};
use crate::{Result, Config, SipClient};

/// Execute receive command
pub async fn execute(
    auto_answer: bool,
    max_duration: u64,
    config: &Config,
) -> Result<()> {
    info!("📞 Waiting for incoming calls");
    info!("   Auto-answer: {}", auto_answer);
    info!("   Max duration: {}s", max_duration);

    // Create and start client
    let mut client = SipClient::new(config.clone()).await?;
    
    // Register to receive calls
    client.register().await?;
    info!("✅ Registered with SIP server, ready to receive calls");
    info!("📞 Waiting for incoming calls (press Ctrl+C to exit)...");

    // Wait for incoming calls
    loop {
        tokio::select! {
            incoming = client.next_incoming_call() => {
                if let Some(incoming) = incoming {
                    info!("📞 Incoming call from {}", incoming.caller_id());
                    
                    if auto_answer {
                        info!("✅ Auto-answering call...");
                        let call = incoming.answer().await?;
                        
                        info!("🔄 Call active for up to {}s...", max_duration);
                        tokio::time::sleep(Duration::from_secs(max_duration)).await;
                        
                        call.hangup().await?;
                        info!("📴 Call ended");
                    } else {
                        // Manual answer prompt (simplified for now)
                        info!("❓ Answer call? (auto-rejecting for now)");
                        incoming.reject().await?;
                        info!("❌ Call rejected");
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                info!("🛑 Shutting down...");
                break;
            }
        }
    }

    info!("✅ Receive mode ended");
    Ok(())
} 