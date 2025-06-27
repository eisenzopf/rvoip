//! Register command - Register with a SIP server

use std::time::Duration;
use tracing::{info, warn};
use crate::{Result, Config, SipClient};

/// Execute register command
pub async fn execute(
    username: &str,
    password: &str,
    domain: &str,
    timeout: u64,
    base_config: &Config,
) -> Result<()> {
    info!("📝 Registering with SIP server");
    info!("   Username: {}", username);
    info!("   Domain: {}", domain);
    info!("   Timeout: {}s", timeout);

    // Create config with provided credentials
    let config = base_config
        .clone()
        .with_credentials(username, password, domain);

    // Create and start client
    let client = SipClient::new(config).await?;
    
    // Attempt registration with timeout
    let registration_result = tokio::time::timeout(
        Duration::from_secs(timeout),
        client.register_with(username, password, domain)
    ).await;

    match registration_result {
        Ok(Ok(())) => {
            info!("✅ Registration successful!");
            
            // Keep client running for a bit to maintain registration
            info!("🔄 Keeping registration active for 30 seconds...");
            tokio::time::sleep(Duration::from_secs(30)).await;
            
            info!("✅ Registration test completed");
        }
        Ok(Err(e)) => {
            warn!("❌ Registration failed: {}", e);
            return Err(e);
        }
        Err(_) => {
            warn!("❌ Registration timed out after {}s", timeout);
            return Err(crate::Error::Timeout("Registration timed out".to_string()));
        }
    }

    Ok(())
} 