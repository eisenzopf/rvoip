//! # 03 - Call Screening Server
//! 
//! A call screening server that only accepts calls from allowed phone numbers.
//! Perfect for security systems and private lines.

use rvoip_session_core::api::simple::*;
use std::sync::Arc;
use std::collections::HashSet;
use tokio;

/// Call screening server with allowlist
struct CallScreeningServer {
    allowed_numbers: HashSet<String>,
    blocked_message: String,
}

impl CallScreeningServer {
    fn new() -> Self {
        let mut allowed_numbers = HashSet::new();
        allowed_numbers.insert("sip:boss@company.com".to_string());
        allowed_numbers.insert("sip:family@home.net".to_string());
        allowed_numbers.insert("sip:+15551234567@provider.com".to_string());

        Self {
            allowed_numbers,
            blocked_message: "assets/blocked_message.wav".to_string(),
        }
    }

    fn is_allowed(&self, caller: &str) -> bool {
        self.allowed_numbers.contains(caller)
    }
}

impl CallHandler for CallScreeningServer {
    async fn on_incoming_call(&self, call: &IncomingCall) -> CallAction {
        let caller = call.from();
        println!("📞 Call screening: Incoming call from {}", caller);

        if self.is_allowed(caller) {
            println!("✅ Caller {} is on allowlist - accepting call", caller);
            CallAction::Answer
        } else {
            println!("🚫 Caller {} is NOT on allowlist - rejecting call", caller);
            CallAction::Reject {
                reason: "Not authorized".to_string(),
                play_message: Some(self.blocked_message.clone()),
            }
        }
    }

    async fn on_call_connected(&self, call: &ActiveCall) {
        println!("✅ Authorized call connected with {}", call.remote_party());
    }

    async fn on_call_rejected(&self, call: &IncomingCall, reason: &str) {
        println!("🚫 Call from {} rejected: {}", call.from(), reason);
    }

    async fn on_call_ended(&self, call: &ActiveCall, reason: &str) {
        println!("📴 Call ended with {}: {}", call.remote_party(), reason);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Starting Call Screening Server");

    // Create session manager with default config
    let config = SessionConfig::default();
    let session_manager = SessionManager::new(config).await?;

    // Set our call screening handler
    session_manager.set_call_handler(Arc::new(CallScreeningServer::new())).await?;

    // Start listening for incoming calls
    println!("🎧 Call screening server listening on 0.0.0.0:5060");
    println!("🛡️ Only accepting calls from authorized numbers");
    session_manager.start_server("0.0.0.0:5060").await?;

    // Keep running
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_call_screening_allowed() {
        let server = CallScreeningServer::new();
        let mock_call = IncomingCall::mock("sip:boss@company.com");
        
        let action = server.on_incoming_call(&mock_call).await;
        assert_eq!(action, CallAction::Answer);
    }

    #[tokio::test]
    async fn test_call_screening_blocked() {
        let server = CallScreeningServer::new();
        let mock_call = IncomingCall::mock("sip:spam@telemarketer.com");
        
        let action = server.on_incoming_call(&mock_call).await;
        assert!(matches!(action, CallAction::Reject { .. }));
    }
} 