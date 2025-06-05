//! # 01 - Auto-Answer Server
//! 
//! The simplest possible SIP server that automatically answers every incoming call.
//! Perfect for testing endpoints and basic connectivity verification.

use rvoip_session_core::api::simple::*;
use std::sync::Arc;
use tokio;

/// Auto-answer server implementation
struct AutoAnswerServer;

impl CallHandler for AutoAnswerServer {
    async fn on_incoming_call(&self, call: &IncomingCall) -> CallAction {
        println!("📞 Auto-answering call from {}", call.from());
        CallAction::Answer
    }

    async fn on_call_connected(&self, call: &ActiveCall) {
        println!("✅ Call connected with {}", call.remote_party());
    }

    async fn on_call_ended(&self, call: &ActiveCall, reason: &str) {
        println!("📴 Call ended with {}: {}", call.remote_party(), reason);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Starting Auto-Answer Server");

    // Create session manager with default config
    let config = SessionConfig::default();
    let session_manager = SessionManager::new(config).await?;

    // Set our auto-answer handler
    session_manager.set_call_handler(Arc::new(AutoAnswerServer)).await?;

    // Start listening for incoming calls
    println!("🎧 Listening on 0.0.0.0:5060 for incoming calls...");
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
    async fn test_auto_answer_server() {
        let server = AutoAnswerServer;
        let mock_call = IncomingCall::mock("sip:test@example.com");
        
        let action = server.on_incoming_call(&mock_call).await;
        assert_eq!(action, CallAction::Answer);
    }
} 