//! Call command - Make outgoing SIP calls

use std::time::Duration;
use tracing::{info, warn};
use crate::{Result, Config, SipClient};

/// Execute call command
pub async fn execute(
    target: &str,
    duration: u64,
    auto_hangup: bool,
    config: &Config,
) -> Result<()> {
    info!("📞 Making outgoing call");
    info!("   Target: {}", target);
    info!("   Duration: {}s", if duration == 0 { "unlimited".to_string() } else { duration.to_string() });
    info!("   Auto-hangup: {}", auto_hangup);

    // Create and start client
    let client = SipClient::new(config.clone()).await?;
    
    // Register first (required for outgoing calls)
    client.register().await?;
    info!("✅ Registered with SIP server");

    // Make the call
    info!("📞 Calling {}...", target);
    let call = client.call(target).await?;
    
    // Wait for answer
    info!("⏳ Waiting for answer...");
    call.wait_for_answer().await?;
    info!("✅ Call connected!");

    // Handle call duration
    if duration > 0 {
        info!("🔄 Call active for {}s...", duration);
        tokio::time::sleep(Duration::from_secs(duration)).await;
        
        if auto_hangup {
            call.hangup().await?;
            info!("📴 Call hung up automatically");
        }
    } else {
        info!("🔄 Call active (press Ctrl+C to hang up)...");
        
        // Wait for Ctrl+C
        tokio::signal::ctrl_c().await.expect("Failed to listen for Ctrl+C");
        info!("📴 Hanging up call...");
        call.hangup().await?;
        info!("✅ Call ended");
    }

    Ok(())
} 