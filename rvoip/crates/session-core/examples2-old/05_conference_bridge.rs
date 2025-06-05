//! # 05 - Conference Bridge Server
//! 
//! A conference bridge server that connects all callers together in a group call.
//! Perfect for group meetings and team conferences.

use rvoip_session_core::api::simple::*;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashMap;
use tokio;

/// Conference bridge server that manages group calls
struct ConferenceBridgeServer {
    conferences: Arc<Mutex<HashMap<String, Conference>>>,
    default_conference_id: String,
}

#[derive(Debug, Clone)]
struct Conference {
    id: String,
    participants: Vec<String>,
    max_participants: usize,
}

impl ConferenceBridgeServer {
    fn new() -> Self {
        Self {
            conferences: Arc::new(Mutex::new(HashMap::new())),
            default_conference_id: "main-conference".to_string(),
        }
    }

    async fn join_conference(&self, conference_id: &str, participant: &str) -> bool {
        let mut conferences = self.conferences.lock().await;
        
        let conference = conferences.entry(conference_id.to_string()).or_insert_with(|| {
            Conference {
                id: conference_id.to_string(),
                participants: Vec::new(),
                max_participants: 10,
            }
        });

        if conference.participants.len() < conference.max_participants {
            conference.participants.push(participant.to_string());
            println!("👥 {} joined conference '{}' ({}/{} participants)", 
                participant, conference_id, conference.participants.len(), conference.max_participants);
            true
        } else {
            println!("🚫 Conference '{}' is full ({} participants)", 
                conference_id, conference.max_participants);
            false
        }
    }

    async fn leave_conference(&self, conference_id: &str, participant: &str) {
        let mut conferences = self.conferences.lock().await;
        
        if let Some(conference) = conferences.get_mut(conference_id) {
            conference.participants.retain(|p| p != participant);
            println!("👋 {} left conference '{}' ({} participants remaining)", 
                participant, conference_id, conference.participants.len());
            
            // Remove empty conferences
            if conference.participants.is_empty() {
                conferences.remove(conference_id);
                println!("🗑️ Removed empty conference '{}'", conference_id);
            }
        }
    }
}

impl CallHandler for ConferenceBridgeServer {
    async fn on_incoming_call(&self, call: &IncomingCall) -> CallAction {
        let caller = call.from();
        println!("📞 Conference: Incoming call from {}", caller);

        // Extract conference ID from call (could be in SIP headers or URI parameters)
        let conference_id = call.get_parameter("conference")
            .unwrap_or(&self.default_conference_id);

        let can_join = self.join_conference(conference_id, caller).await;

        if can_join {
            CallAction::Answer
        } else {
            CallAction::Reject {
                reason: "Conference full".to_string(),
                play_message: Some("assets/conference_full.wav".to_string()),
            }
        }
    }

    async fn on_call_connected(&self, call: &ActiveCall) {
        let participant = call.remote_party();
        println!("✅ Conference: {} connected to conference", participant);

        // Play welcome message
        call.play_audio_file("assets/conference_welcome.wav").await.ok();

        // Add participant to the conference bridge
        let conference_id = call.get_parameter("conference")
            .unwrap_or(&self.default_conference_id);
        
        call.join_conference_bridge(conference_id).await.ok();
        
        // Announce participant joined to others
        call.announce_to_conference(&format!("{} has joined the conference", participant)).await.ok();
    }

    async fn on_call_ended(&self, call: &ActiveCall, reason: &str) {
        let participant = call.remote_party();
        println!("📴 Conference: {} left conference: {}", participant, reason);

        // Remove from conference bridge
        let conference_id = call.get_parameter("conference")
            .unwrap_or(&self.default_conference_id);
        
        call.leave_conference_bridge().await.ok();
        self.leave_conference(conference_id, participant).await;

        // Announce participant left to others
        call.announce_to_conference(&format!("{} has left the conference", participant)).await.ok();
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Starting Conference Bridge Server");

    // Create session manager with default config
    let config = SessionConfig::default();
    let session_manager = SessionManager::new(config).await?;

    // Set our conference bridge handler
    session_manager.set_call_handler(Arc::new(ConferenceBridgeServer::new())).await?;

    // Start listening for incoming calls
    println!("🎧 Conference bridge listening on 0.0.0.0:5060");
    println!("📞 Call sip:conference@server to join main conference");
    println!("📞 Call sip:conference@server?conference=meeting1 to join specific conference");
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
    async fn test_conference_join() {
        let server = ConferenceBridgeServer::new();
        let can_join = server.join_conference("test-conf", "sip:user1@example.com").await;
        assert!(can_join);
        
        let can_join2 = server.join_conference("test-conf", "sip:user2@example.com").await;
        assert!(can_join2);
    }

    #[tokio::test]
    async fn test_conference_bridge_handler() {
        let server = ConferenceBridgeServer::new();
        let mock_call = IncomingCall::mock("sip:participant@example.com");
        
        let action = server.on_incoming_call(&mock_call).await;
        assert_eq!(action, CallAction::Answer);
    }
} 