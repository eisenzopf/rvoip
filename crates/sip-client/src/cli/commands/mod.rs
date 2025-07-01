//! CLI command implementations
//!
//! This module contains the implementation of all CLI commands for the SIP client.

pub mod register;
pub mod call;
pub mod receive;
pub mod status;

// Agent command (call-engine integration)
pub mod agent {
    use tracing::{info, warn};
    use crate::{Result, Config, SipClient};

    /// Execute agent command - register as call center agent
    pub async fn execute(
        queue: &str,
        server: &str,
        agent_id: Option<&str>,
        config: &Config,
    ) -> Result<()> {
        info!("🏢 Starting call center agent mode");
        info!("   Queue: {}", queue);
        info!("   Server: {}", server);
        if let Some(id) = agent_id {
            info!("   Agent ID: {}", id);
        }

        // Create client with call-engine integration
        let config = config.clone().with_call_engine(server);
        let mut client = SipClient::new(config).await?;

        // Register with SIP server first
        client.register().await?;
        info!("✅ Registered with SIP server");

        // Register as agent with call-engine
        client.register_as_agent(queue).await?;
        info!("✅ Registered as agent with call-engine queue: {}", queue);

        // Wait for assigned calls
        info!("📞 Waiting for assigned calls from call center...");
        while let Some(assigned_call) = client.next_assigned_call().await {
            info!("📞 Call assigned from call center");
            
            // Answer the call
            let call = assigned_call.answer().await?;
            info!("✅ Call answered, handling customer...");
            
            // Keep call active (in a real agent app, this would be interactive)
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            
            // Hang up
            call.hangup().await?;
            info!("📴 Call completed");
        }

        Ok(())
    }
} 