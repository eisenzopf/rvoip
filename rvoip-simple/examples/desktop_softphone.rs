//! Desktop Softphone Example
//!
//! This example demonstrates a comprehensive desktop softphone application
//! with advanced features like call transfer, conferencing, and quality monitoring.

use rvoip_simple::*;
use tracing::{info, warn, error, debug};
use tokio::time::{sleep, Duration};
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    info!("🖥️  Starting Desktop Softphone Example");

    // Create desktop softphone
    let mut softphone = DesktopSoftphone::new().await?;
    
    // Run the softphone simulation
    softphone.run_demo().await?;

    info!("✅ Desktop softphone example completed!");
    Ok(())
}

/// Desktop softphone with advanced features
struct DesktopSoftphone {
    client: SimpleVoipClient,
    active_calls: HashMap<String, Call>,
    call_history: Vec<CallRecord>,
    settings: SoftphoneSettings,
}

/// Call history record
#[derive(Debug, Clone)]
struct CallRecord {
    id: String,
    remote_party: String,
    direction: CallDirection,
    start_time: std::time::SystemTime,
    end_time: Option<std::time::SystemTime>,
    duration: Option<Duration>,
    quality_score: Option<f32>,
}

/// Softphone user settings
#[derive(Debug, Clone)]
struct SoftphoneSettings {
    auto_answer: bool,
    do_not_disturb: bool,
    call_forwarding: Option<String>,
    ring_tone: String,
    microphone_level: u8,    // 0-100
    speaker_level: u8,       // 0-100
    video_enabled: bool,
    presence_status: PresenceStatus,
}

/// User presence status
#[derive(Debug, Clone, PartialEq)]
enum PresenceStatus {
    Available,
    Busy,
    Away,
    DoNotDisturb,
    Offline,
}

impl DesktopSoftphone {
    /// Create a new desktop softphone
    async fn new() -> Result<Self, SimpleVoipError> {
        info!("🖥️  Initializing Desktop Softphone");

        // Create desktop-optimized client
        let client = SimpleVoipClient::desktop("john.doe@company.com", "secure_password")
            .with_display_name("John Doe - Desktop")
            .with_registrar("sip.company.com")
            .with_security(SecurityConfig::Auto)
            .with_media(MediaConfig::desktop())
            .connect().await?;

        info!("✅ Softphone client connected");
        info!("   User: john.doe@company.com");
        info!("   Display Name: John Doe - Desktop");
        info!("   Server: sip.company.com");
        info!("   Security: Automatic (DTLS-SRTP)");
        info!("   Media: Desktop-optimized");

        let settings = SoftphoneSettings {
            auto_answer: false,
            do_not_disturb: false,
            call_forwarding: None,
            ring_tone: "classic".to_string(),
            microphone_level: 75,
            speaker_level: 80,
            video_enabled: true,
            presence_status: PresenceStatus::Available,
        };

        Ok(Self {
            client,
            active_calls: HashMap::new(),
            call_history: Vec::new(),
            settings,
        })
    }

    /// Run the softphone demonstration
    async fn run_demo(&mut self) -> Result<(), SimpleVoipError> {
        info!("🚀 Starting softphone demonstration");

        // Show initial status
        self.show_status().await;

        // Demonstrate various softphone features
        self.demo_basic_calling().await?;
        self.demo_call_management().await?;
        self.demo_conference_calling().await?;
        self.demo_settings_management().await?;
        self.demo_call_history().await?;

        Ok(())
    }

    /// Show current softphone status
    async fn show_status(&self) {
        info!("📊 Softphone Status:");
        info!("   Registration: {:?}", self.client.state());
        info!("   Active calls: {}", self.active_calls.len());
        info!("   Presence: {:?}", self.settings.presence_status);
        info!("   Do Not Disturb: {}", self.settings.do_not_disturb);
        info!("   Call Forwarding: {:?}", self.settings.call_forwarding);
        info!("   Video: {}", if self.settings.video_enabled { "Enabled" } else { "Disabled" });
    }

    /// Demonstrate basic calling features
    async fn demo_basic_calling(&mut self) -> Result<(), SimpleVoipError> {
        info!("📞 Demo: Basic Calling Features");

        // Make an outgoing call
        info!("📞 Making call to colleague...");
        match self.client.make_call("jane.smith@company.com").await {
            Ok(mut call) => {
                let call_id = call.id.clone();
                info!("✅ Call initiated: {} -> {}", call_id, call.remote_party);
                
                // Add to active calls
                self.active_calls.insert(call_id.clone(), call);
                
                // Simulate call progression
                sleep(Duration::from_millis(500)).await;
                
                // Update call to answered state
                if let Some(call) = self.active_calls.get_mut(&call_id) {
                    call.update_state(CallState::Answered);
                    info!("📞 Call answered by Jane Smith");
                    
                    // Show call details
                    self.show_call_details(&call_id).await;
                    
                    // Demonstrate call controls
                    self.demo_call_controls(&call_id).await?;
                    
                    // End call
                    call.hangup().await?;
                    info!("📞 Call ended");
                    
                    // Record in history
                    self.record_call_history(&call_id).await;
                }
                
                // Remove from active calls
                self.active_calls.remove(&call_id);
            }
            Err(e) => {
                warn!("⚠️  Call failed (expected in demo): {}", e);
            }
        }

        Ok(())
    }

    /// Show detailed call information
    async fn show_call_details(&self, call_id: &str) {
        if let Some(call) = self.active_calls.get(call_id) {
            info!("📋 Call Details for {}:", call_id);
            info!("   Remote: {}", call.remote_party);
            info!("   State: {:?}", call.state);
            info!("   Direction: {:?}", call.direction);
            info!("   Duration: {:?}", call.duration());
            
            if let Some(quality) = call.quality() {
                info!("   Quality:");
                info!("     MOS Score: {:.2}", quality.mos_score);
                info!("     Packet Loss: {:.1}%", quality.packet_loss);
                info!("     Jitter: {:?}", quality.jitter);
                info!("     RTT: {:?}", quality.rtt);
            }
            
            info!("   Media Stats:");
            info!("     Packets Sent: {}", call.media_stats.packets_sent);
            info!("     Packets Received: {}", call.media_stats.packets_received);
            info!("     Current Bitrate: {} kbps", call.media_stats.current_bitrate / 1000);
        }
    }

    /// Demonstrate call control features
    async fn demo_call_controls(&mut self, call_id: &str) -> Result<(), SimpleVoipError> {
        info!("🎛️  Demo: Call Controls");
        
        if let Some(call) = self.active_calls.get_mut(call_id) {
            // Demonstrate mute/unmute
            info!("🔇 Muting microphone...");
            call.mute(true).await?;
            sleep(Duration::from_millis(500)).await;
            
            info!("🔊 Unmuting microphone...");
            call.mute(false).await?;
            sleep(Duration::from_millis(500)).await;
            
            // Demonstrate hold/unhold
            info!("⏸️  Putting call on hold...");
            call.hold().await?;
            sleep(Duration::from_secs(1)).await;
            
            info!("▶️  Resuming call from hold...");
            call.unhold().await?;
            sleep(Duration::from_millis(500)).await;
            
            // Demonstrate DTMF
            info!("🔢 Sending DTMF sequence...");
            call.send_dtmf_string("*123#").await?;
            info!("   Sent: *123#");
        }
        
        Ok(())
    }

    /// Demonstrate call management features
    async fn demo_call_management(&mut self) -> Result<(), SimpleVoipError> {
        info!("📋 Demo: Call Management");

        // Simulate incoming call
        self.handle_incoming_call().await?;
        
        // Demonstrate call forwarding
        self.demo_call_forwarding().await?;
        
        // Demonstrate do not disturb
        self.demo_do_not_disturb().await?;

        Ok(())
    }

    /// Handle incoming call scenario
    async fn handle_incoming_call(&mut self) -> Result<(), SimpleVoipError> {
        info!("📞 Incoming call from: sales@company.com");
        info!("   Caller ID: Sales Department");
        info!("   Has Video: Yes");
        
        // Check do not disturb
        if self.settings.do_not_disturb {
            info!("🚫 Call automatically rejected (Do Not Disturb)");
            return Ok(());
        }
        
        // Check call forwarding
        if let Some(forward_to) = &self.settings.call_forwarding {
            info!("📞 Call forwarded to: {}", forward_to);
            return Ok(());
        }
        
        // Simulate user accepting call
        sleep(Duration::from_millis(500)).await;
        info!("✅ Call accepted by user");
        
        match self.client.answer_call("incoming-call-456").await {
            Ok(mut call) => {
                let call_id = call.id.clone();
                self.active_calls.insert(call_id.clone(), call);
                info!("📞 Call answered successfully");
                
                // Brief conversation
                sleep(Duration::from_secs(2)).await;
                
                // End call
                if let Some(call) = self.active_calls.get_mut(&call_id) {
                    call.hangup().await?;
                    self.record_call_history(&call_id).await;
                }
                self.active_calls.remove(&call_id);
            }
            Err(e) => {
                warn!("⚠️  Failed to answer call: {}", e);
            }
        }
        
        Ok(())
    }

    /// Demonstrate call forwarding
    async fn demo_call_forwarding(&mut self) -> Result<(), SimpleVoipError> {
        info!("📞 Demo: Call Forwarding");
        
        // Enable call forwarding
        self.settings.call_forwarding = Some("voicemail@company.com".to_string());
        info!("✅ Call forwarding enabled to: voicemail@company.com");
        
        // Simulate incoming call with forwarding
        info!("📞 Incoming call from: customer@external.com");
        info!("📞 Call forwarded to voicemail (forwarding active)");
        
        // Disable call forwarding
        self.settings.call_forwarding = None;
        info!("❌ Call forwarding disabled");
        
        Ok(())
    }

    /// Demonstrate do not disturb mode
    async fn demo_do_not_disturb(&mut self) -> Result<(), SimpleVoipError> {
        info!("🚫 Demo: Do Not Disturb Mode");
        
        // Enable DND
        self.settings.do_not_disturb = true;
        self.settings.presence_status = PresenceStatus::DoNotDisturb;
        info!("✅ Do Not Disturb enabled");
        info!("   Presence status: Do Not Disturb");
        info!("   Incoming calls will be automatically rejected");
        
        // Simulate rejected call
        info!("📞 Incoming call from: marketing@company.com");
        info!("🚫 Call automatically rejected (DND active)");
        
        // Disable DND
        self.settings.do_not_disturb = false;
        self.settings.presence_status = PresenceStatus::Available;
        info!("❌ Do Not Disturb disabled");
        info!("   Presence status: Available");
        
        Ok(())
    }

    /// Demonstrate conference calling
    async fn demo_conference_calling(&mut self) -> Result<(), SimpleVoipError> {
        info!("👥 Demo: Conference Calling");
        
        // Start conference with multiple participants
        info!("📞 Creating conference call...");
        
        // First participant
        match self.client.make_call("alice@company.com").await {
            Ok(call) => {
                let call_id = call.id.clone();
                self.active_calls.insert(call_id.clone(), call);
                info!("✅ Alice joined conference");
                
                // Second participant  
                match self.client.make_call("bob@company.com").await {
                    Ok(call) => {
                        let call_id2 = call.id.clone();
                        self.active_calls.insert(call_id2.clone(), call);
                        info!("✅ Bob joined conference");
                        
                        info!("👥 Conference active with 3 participants:");
                        info!("   • John Doe (host)");
                        info!("   • Alice");
                        info!("   • Bob");
                        
                        // Conference features
                        info!("🎛️  Conference controls available:");
                        info!("   • Mute/unmute individual participants");
                        info!("   • Remove participants");
                        info!("   • Conference recording");
                        info!("   • Screen sharing");
                        
                        // Simulate conference activity
                        sleep(Duration::from_secs(3)).await;
                        
                        // End conference
                        info!("📞 Ending conference...");
                        if let Some(call) = self.active_calls.get_mut(&call_id) {
                            call.hangup().await?;
                        }
                        if let Some(call) = self.active_calls.get_mut(&call_id2) {
                            call.hangup().await?;
                        }
                        
                        self.active_calls.clear();
                        info!("✅ Conference ended");
                    }
                    Err(e) => warn!("⚠️  Failed to add Bob: {}", e),
                }
            }
            Err(e) => warn!("⚠️  Failed to start conference: {}", e),
        }
        
        Ok(())
    }

    /// Demonstrate settings management
    async fn demo_settings_management(&mut self) -> Result<(), SimpleVoipError> {
        info!("⚙️  Demo: Settings Management");
        
        // Audio settings
        info!("🔊 Audio Settings:");
        self.settings.microphone_level = 85;
        self.settings.speaker_level = 70;
        info!("   Microphone level: {}%", self.settings.microphone_level);
        info!("   Speaker level: {}%", self.settings.speaker_level);
        
        // Video settings
        info!("📹 Video Settings:");
        self.settings.video_enabled = true;
        info!("   Video calling: Enabled");
        info!("   Camera resolution: 1280x720");
        info!("   Frame rate: 30 fps");
        
        // Ring tone settings
        info!("🔔 Ring Tone Settings:");
        self.settings.ring_tone = "professional".to_string();
        info!("   Ring tone: {}", self.settings.ring_tone);
        info!("   Ring volume: 80%");
        
        // Presence settings
        info!("👤 Presence Settings:");
        self.settings.presence_status = PresenceStatus::Available;
        info!("   Status: {:?}", self.settings.presence_status);
        info!("   Status message: Ready for calls");
        
        Ok(())
    }

    /// Demonstrate call history
    async fn demo_call_history(&self) -> Result<(), SimpleVoipError> {
        info!("📜 Demo: Call History");
        
        if self.call_history.is_empty() {
            info!("📭 No calls in history");
        } else {
            info!("📊 Call History ({} records):", self.call_history.len());
            
            for (i, record) in self.call_history.iter().enumerate() {
                info!("   {}. {} ({:?})", i + 1, record.remote_party, record.direction);
                info!("      Duration: {:?}", record.duration);
                if let Some(quality) = record.quality_score {
                    info!("      Quality: {:.1}/5.0", quality);
                }
            }
        }
        
        // Show call statistics
        self.show_call_statistics().await;
        
        Ok(())
    }

    /// Show call statistics
    async fn show_call_statistics(&self) {
        info!("📈 Call Statistics:");
        
        let total_calls = self.call_history.len();
        let outgoing_calls = self.call_history.iter()
            .filter(|r| r.direction == CallDirection::Outgoing)
            .count();
        let incoming_calls = total_calls - outgoing_calls;
        
        let total_duration: Duration = self.call_history.iter()
            .filter_map(|r| r.duration)
            .sum();
        
        let avg_quality: f32 = self.call_history.iter()
            .filter_map(|r| r.quality_score)
            .sum::<f32>() / total_calls.max(1) as f32;
        
        info!("   Total calls: {}", total_calls);
        info!("   Outgoing: {} ({:.1}%)", outgoing_calls, 
              outgoing_calls as f32 / total_calls.max(1) as f32 * 100.0);
        info!("   Incoming: {} ({:.1}%)", incoming_calls,
              incoming_calls as f32 / total_calls.max(1) as f32 * 100.0);
        info!("   Total talk time: {:?}", total_duration);
        info!("   Average quality: {:.1}/5.0", avg_quality);
    }

    /// Record call in history
    async fn record_call_history(&mut self, call_id: &str) {
        if let Some(call) = self.active_calls.get(call_id) {
            let record = CallRecord {
                id: call.id.clone(),
                remote_party: call.remote_party.clone(),
                direction: call.direction.clone(),
                start_time: call.start_time.unwrap_or_else(std::time::SystemTime::now),
                end_time: Some(std::time::SystemTime::now()),
                duration: call.duration(),
                quality_score: call.quality().map(|q| q.mos_score),
            };
            
            self.call_history.push(record);
            info!("📝 Call recorded in history");
        }
    }
} 