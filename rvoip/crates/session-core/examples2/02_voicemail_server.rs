//! # 02 - Voicemail Server
//! 
//! A voicemail server that answers calls, plays a greeting message,
//! records caller messages, and hangs up after 30 seconds.

use rvoip_session_core::api::simple::*;
use std::sync::Arc;
use tokio;
use std::time::Duration;

/// Voicemail server implementation
struct VoicemailServer {
    greeting_file: String,
    recordings_dir: String,
}

impl VoicemailServer {
    fn new() -> Self {
        Self {
            greeting_file: "assets/greeting.wav".to_string(),
            recordings_dir: "recordings/".to_string(),
        }
    }
}

impl CallHandler for VoicemailServer {
    async fn on_incoming_call(&self, call: &IncomingCall) -> CallAction {
        println!("📞 Voicemail: Incoming call from {}", call.from());
        CallAction::Answer
    }

    async fn on_call_connected(&self, call: &ActiveCall) {
        println!("✅ Voicemail: Call connected with {}", call.remote_party());
        
        // Play greeting message
        println!("🔊 Playing greeting message...");
        call.play_audio_file(&self.greeting_file).await.ok();
        
        // Start recording after greeting
        let recording_file = format!("{}/vm_{}.wav", 
            self.recordings_dir, 
            chrono::Utc::now().timestamp()
        );
        
        println!("🎙️ Recording message to: {}", recording_file);
        call.start_recording(&recording_file).await.ok();
        
        // Schedule hangup after 30 seconds
        let call_clone = call.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(30)).await;
            println!("⏰ 30 seconds elapsed, ending call");
            call_clone.hangup("Recording time limit reached").await.ok();
        });
    }

    async fn on_call_ended(&self, call: &ActiveCall, reason: &str) {
        println!("📴 Voicemail: Call ended with {}: {}", call.remote_party(), reason);
        
        // Stop recording when call ends
        call.stop_recording().await.ok();
        println!("💾 Recording saved");
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Starting Voicemail Server");

    // Create recordings directory
    tokio::fs::create_dir_all("recordings").await?;

    // Create session manager with default config
    let config = SessionConfig::default();
    let session_manager = SessionManager::new(config).await?;

    // Set our voicemail handler
    session_manager.set_call_handler(Arc::new(VoicemailServer::new())).await?;

    // Start listening for incoming calls
    println!("🎧 Voicemail server listening on 0.0.0.0:5060");
    println!("📦 Recordings will be saved to: recordings/");
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
    async fn test_voicemail_server() {
        let server = VoicemailServer::new();
        let mock_call = IncomingCall::mock("sip:caller@example.com");
        
        let action = server.on_incoming_call(&mock_call).await;
        assert_eq!(action, CallAction::Answer);
    }
} 