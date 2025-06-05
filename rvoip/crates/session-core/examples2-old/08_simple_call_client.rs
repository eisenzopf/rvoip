//! # 08 - Simple Call Client
//! 
//! A basic SIP client that makes outgoing calls.
//! Perfect for basic softphone functionality and testing.

use rvoip_session_core::api::simple::*;
use std::io;
use tokio;

/// Simple SIP call client
struct SimpleCallClient {
    session_manager: SessionManager,
    local_uri: String,
}

impl SimpleCallClient {
    async fn new(local_uri: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let config = SessionConfig::default();
        let session_manager = SessionManager::new(config).await?;

        Ok(Self {
            session_manager,
            local_uri: local_uri.to_string(),
        })
    }

    async fn make_call(&self, to: &str) -> Result<ActiveCall, Box<dyn std::error::Error>> {
        println!("📞 Making call from {} to {}", self.local_uri, to);
        
        let call = self.session_manager
            .make_call(&self.local_uri, to, None)
            .await?;

        println!("📲 Call initiated, waiting for response...");
        Ok(call)
    }

    async fn handle_call_events(&self, call: &ActiveCall) {
        // Set up event handlers for the call
        call.on_ringing(|call| async move {
            println!("📳 Call is ringing to {}", call.remote_party());
        }).await;

        call.on_answered(|call| async move {
            println!("✅ Call answered by {}", call.remote_party());
            println!("🎤 You can now speak! Press Enter to hang up.");
        }).await;

        call.on_rejected(|call, reason| async move {
            println!("🚫 Call rejected by {}: {}", call.remote_party(), reason);
        }).await;

        call.on_ended(|call, reason| async move {
            println!("📴 Call ended with {}: {}", call.remote_party(), reason);
        }).await;
    }

    async fn interactive_mode(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("🎤 Simple Call Client - Interactive Mode");
        println!("📱 Your URI: {}", self.local_uri);
        println!("💡 Type a SIP URI to call (e.g., sip:user@example.com)");
        println!("💡 Type 'quit' to exit");

        loop {
            println!("\n📞 Enter number to call:");
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            let input = input.trim();

            if input == "quit" {
                break;
            }

            if input.is_empty() {
                continue;
            }

            // Add sip: prefix if not present
            let to_uri = if input.starts_with("sip:") {
                input.to_string()
            } else {
                format!("sip:{}", input)
            };

            match self.make_call(&to_uri).await {
                Ok(call) => {
                    // Set up event handlers
                    self.handle_call_events(&call).await;

                    // Wait for user to press Enter to hang up
                    println!("Press Enter to hang up...");
                    let mut hangup_input = String::new();
                    io::stdin().read_line(&mut hangup_input)?;

                    println!("📴 Hanging up call...");
                    call.hangup("User requested hangup").await?;
                },
                Err(e) => {
                    println!("❌ Failed to make call: {}", e);
                }
            }
        }

        Ok(())
    }

    async fn make_single_call(&self, to: &str) -> Result<(), Box<dyn std::error::Error>> {
        let call = self.make_call(to).await?;
        self.handle_call_events(&call).await;

        // Wait for call to complete or timeout
        tokio::select! {
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(30)) => {
                println!("⏰ Call timeout, hanging up");
                call.hangup("Timeout").await?;
            }
            _ = call.wait_for_completion() => {
                println!("📞 Call completed");
            }
        }

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Starting Simple Call Client");

    // Get local SIP URI from command line args or use default
    let args: Vec<String> = std::env::args().collect();
    let local_uri = args.get(1)
        .cloned()
        .unwrap_or_else(|| "sip:client@localhost".to_string());

    let client = SimpleCallClient::new(&local_uri).await?;

    // Check if a target URI was provided for single call mode
    if let Some(target_uri) = args.get(2) {
        println!("📞 Single call mode to: {}", target_uri);
        client.make_single_call(target_uri).await?;
    } else {
        // Interactive mode
        client.interactive_mode().await?;
    }

    println!("👋 Goodbye!");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_simple_call_client_creation() {
        let client = SimpleCallClient::new("sip:test@localhost").await;
        assert!(client.is_ok());
    }

    #[tokio::test]
    async fn test_call_initiation() {
        let client = SimpleCallClient::new("sip:test@localhost").await.unwrap();
        
        // Note: This would fail without a real SIP server, but shows the API
        // let result = client.make_call("sip:target@example.com").await;
        // In a real test, we'd use a mock SIP server
    }
} 