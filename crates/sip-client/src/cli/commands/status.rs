//! Status command - Show client status and statistics

use std::time::Duration;
use tracing::{info, warn};
use crate::{Result, Config, SipClient};

/// Execute status command
pub async fn execute(
    detailed: bool,
    refresh: u64,
    config: &Config,
) -> Result<()> {
    info!("📊 Checking SIP client status");
    
    if refresh > 0 {
        info!("🔄 Refreshing every {}s (press Ctrl+C to exit)", refresh);
    }

    loop {
        // Create client to check status
        let client = SipClient::new(config.clone()).await?;
        let status = client.status().await?;

        // Display status
        println!("\n═══ RVOIP SIP Client Status ═══");
        println!("🚀 Running: {}", if status.is_running { "✅ Yes" } else { "❌ No" });
        println!("📝 Registered: {}", if status.is_registered { "✅ Yes" } else { "❌ No" });
        println!("📞 Total calls: {}", status.total_calls);
        println!("🔊 Active calls: {}", status.active_calls);
        println!("🌐 Local address: {}", status.local_address);

        if detailed {
            println!("\n--- Detailed Information ---");
            
            // Configuration details
            if let Some(creds) = &config.credentials {
                println!("👤 Username: {}", creds.username);
                println!("🌍 Domain: {}", creds.domain);
            }
            
            println!("🎧 User Agent: {}", config.user_agent);
            println!("📱 Max calls: {}", config.max_concurrent_calls);
            println!("🎵 Preferred codecs: {}", config.preferred_codecs().join(", "));
            
            // Media settings
            println!("🎤 Mic volume: {:.1}%", config.media.audio.microphone_volume * 100.0);
            println!("🔊 Speaker volume: {:.1}%", config.media.audio.speaker_volume * 100.0);
            
            // Available codecs
            let codecs = client.available_codecs().await;
            println!("🎵 Available codecs: {}", codecs.join(", "));
        }

        if refresh == 0 {
            break;
        }

        // Wait for refresh interval or Ctrl+C
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(refresh)) => {
                // Continue to next iteration
            }
            _ = tokio::signal::ctrl_c() => {
                info!("🛑 Status monitoring stopped");
                break;
            }
        }
    }

    Ok(())
} 