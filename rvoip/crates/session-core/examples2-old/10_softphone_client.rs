//! # 10 - Full Softphone Client
//! 
//! A complete softphone client with call management, contacts, and full SIP functionality.
//! Perfect for desktop/mobile SIP applications and comprehensive VoIP clients.

use rvoip_session_core::api::simple::*;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::io;
use serde::{Deserialize, Serialize};

/// Full-featured softphone client
struct SoftphoneClient {
    session_manager: SessionManager,
    user_account: UserAccount,
    active_calls: Arc<Mutex<HashMap<String, ActiveCall>>>,
    contacts: Arc<Mutex<HashMap<String, Contact>>>,
    call_history: Arc<Mutex<Vec<CallHistoryEntry>>>,
    config: SoftphoneConfig,
}

#[derive(Debug, Clone)]
struct UserAccount {
    username: String,
    display_name: String,
    sip_uri: String,
    server: String,
    password: String,
    registered: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Contact {
    id: String,
    name: String,
    sip_uri: String,
    phone_number: Option<String>,
    company: Option<String>,
    favorite: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CallHistoryEntry {
    timestamp: chrono::DateTime<chrono::Utc>,
    direction: CallDirection,
    remote_party: String,
    duration: Option<u64>, // seconds
    status: CallStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum CallDirection {
    Incoming,
    Outgoing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum CallStatus {
    Completed,
    Missed,
    Busy,
    Failed,
}

#[derive(Debug, Clone)]
struct SoftphoneConfig {
    auto_answer: bool,
    call_waiting: bool,
    call_forwarding: Option<String>,
    do_not_disturb: bool,
    recording_enabled: bool,
}

impl SoftphoneClient {
    async fn new(account: UserAccount) -> Result<Self, Box<dyn std::error::Error>> {
        let config = SessionConfig::default();
        let session_manager = SessionManager::new(config).await?;

        let softphone_config = SoftphoneConfig {
            auto_answer: false,
            call_waiting: true,
            call_forwarding: None,
            do_not_disturb: false,
            recording_enabled: false,
        };

        Ok(Self {
            session_manager,
            user_account: account,
            active_calls: Arc::new(Mutex::new(HashMap::new())),
            contacts: Arc::new(Mutex::new(HashMap::new())),
            call_history: Arc::new(Mutex::new(Vec::new())),
            config: softphone_config,
        })
    }

    async fn register(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        println!("📱 Registering {} with server {}", self.user_account.username, self.user_account.server);

        let registration = self.session_manager
            .register(&self.user_account.sip_uri, &self.user_account.server, &self.user_account.password)
            .await?;

        if registration.is_successful() {
            self.user_account.registered = true;
            println!("✅ Registration successful");
        } else {
            println!("❌ Registration failed: {}", registration.error_message());
            return Err("Registration failed".into());
        }

        // Set up incoming call handler
        self.setup_call_handler().await?;
        Ok(())
    }

    async fn setup_call_handler(&self) -> Result<(), Box<dyn std::error::Error>> {
        let active_calls = self.active_calls.clone();
        let call_history = self.call_history.clone();
        let config = self.config.clone();

        self.session_manager.set_incoming_call_handler(move |incoming_call| {
            let active_calls = active_calls.clone();
            let call_history = call_history.clone();
            let config = config.clone();

            async move {
                let caller = incoming_call.from();
                println!("📞 Incoming call from {}", caller);

                // Check do not disturb
                if config.do_not_disturb {
                    println!("🔕 Do not disturb enabled - rejecting call");
                    return CallAction::Reject {
                        reason: "Do not disturb".to_string(),
                        play_message: None,
                    };
                }

                // Check if we already have active calls and call waiting is disabled
                let active_calls_guard = active_calls.lock().await;
                if !config.call_waiting && !active_calls_guard.is_empty() {
                    println!("📞 Call waiting disabled - rejecting call");
                    return CallAction::Reject {
                        reason: "Busy".to_string(),
                        play_message: None,
                    };
                }
                drop(active_calls_guard);

                // Auto-answer if enabled
                if config.auto_answer {
                    println!("🤖 Auto-answering call");
                    return CallAction::Answer;
                }

                // Ring and wait for user input
                println!("📳 Call ringing... (a)nswer, (r)eject, (v)oicemail");
                CallAction::Ring
            }
        }).await?;

        Ok(())
    }

    async fn make_call(&self, to: &str) -> Result<String, Box<dyn std::error::Error>> {
        if !self.user_account.registered {
            return Err("Not registered with server".into());
        }

        println!("📞 Making call to {}", to);

        let call = self.session_manager
            .make_call(&self.user_account.sip_uri, to, None)
            .await?;

        let call_id = call.id().to_string();
        
        // Store the active call
        {
            let mut active_calls = self.active_calls.lock().await;
            active_calls.insert(call_id.clone(), call.clone());
        }

        // Set up call event handlers
        self.setup_call_events(&call).await;

        // Add to call history
        self.add_to_history(to, CallDirection::Outgoing, CallStatus::Completed).await;

        Ok(call_id)
    }

    async fn answer_call(&self, call_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let active_calls = self.active_calls.lock().await;
        if let Some(call) = active_calls.get(call_id) {
            call.answer().await?;
            println!("✅ Call answered");
        } else {
            return Err("Call not found".into());
        }
        Ok(())
    }

    async fn hangup_call(&self, call_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let mut active_calls = self.active_calls.lock().await;
        if let Some(call) = active_calls.remove(call_id) {
            call.hangup("User hangup").await?;
            println!("📴 Call ended");
        } else {
            return Err("Call not found".into());
        }
        Ok(())
    }

    async fn hold_call(&self, call_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let active_calls = self.active_calls.lock().await;
        if let Some(call) = active_calls.get(call_id) {
            call.hold().await?;
            println!("⏸️ Call on hold");
        } else {
            return Err("Call not found".into());
        }
        Ok(())
    }

    async fn unhold_call(&self, call_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let active_calls = self.active_calls.lock().await;
        if let Some(call) = active_calls.get(call_id) {
            call.unhold().await?;
            println!("▶️ Call resumed");
        } else {
            return Err("Call not found".into());
        }
        Ok(())
    }

    async fn transfer_call(&self, call_id: &str, target: &str) -> Result<(), Box<dyn std::error::Error>> {
        let active_calls = self.active_calls.lock().await;
        if let Some(call) = active_calls.get(call_id) {
            call.transfer(target).await?;
            println!("↗️ Call transferred to {}", target);
        } else {
            return Err("Call not found".into());
        }
        Ok(())
    }

    async fn setup_call_events(&self, call: &ActiveCall) {
        let call_history = self.call_history.clone();
        let remote_party = call.remote_party().to_string();

        call.on_answered(|_call| async move {
            println!("✅ Call connected");
        }).await;

        call.on_ended(move |_call, reason| {
            let call_history = call_history.clone();
            let remote_party = remote_party.clone();
            async move {
                println!("📴 Call ended: {}", reason);
                // Update call history with actual status
            }
        }).await;
    }

    async fn add_contact(&self, contact: Contact) {
        let mut contacts = self.contacts.lock().await;
        contacts.insert(contact.id.clone(), contact);
        println!("👤 Contact added");
    }

    async fn list_contacts(&self) {
        let contacts = self.contacts.lock().await;
        println!("\n👥 Contacts:");
        for contact in contacts.values() {
            let favorite = if contact.favorite { "⭐" } else { "  " };
            println!("{} {} - {} ({})", favorite, contact.name, contact.sip_uri, 
                contact.company.as_deref().unwrap_or(""));
        }
    }

    async fn show_call_history(&self) {
        let history = self.call_history.lock().await;
        println!("\n📋 Call History:");
        for entry in history.iter().rev().take(10) {
            let direction = match entry.direction {
                CallDirection::Incoming => "📞",
                CallDirection::Outgoing => "📱",
            };
            let status = match entry.status {
                CallStatus::Completed => "✅",
                CallStatus::Missed => "❌",
                CallStatus::Busy => "📞",
                CallStatus::Failed => "💥",
            };
            println!("{} {} {} - {} ({:?})", 
                direction, status, entry.remote_party, 
                entry.timestamp.format("%Y-%m-%d %H:%M"),
                entry.duration.map_or("--:--".to_string(), |d| format!("{}:{:02}", d / 60, d % 60))
            );
        }
    }

    async fn add_to_history(&self, remote_party: &str, direction: CallDirection, status: CallStatus) {
        let mut history = self.call_history.lock().await;
        history.push(CallHistoryEntry {
            timestamp: chrono::Utc::now(),
            direction,
            remote_party: remote_party.to_string(),
            duration: None, // Would be calculated from call events
            status,
        });
    }

    async fn interactive_mode(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n📱 Softphone Client - Interactive Mode");
        println!("🏠 Registered as: {}", self.user_account.sip_uri);
        println!("💡 Commands: call, answer, hangup, hold, unhold, transfer, contacts, history, quit");

        loop {
            println!("\n> ");
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            let input = input.trim();
            let parts: Vec<&str> = input.split_whitespace().collect();

            if parts.is_empty() {
                continue;
            }

            match parts[0] {
                "call" => {
                    if parts.len() > 1 {
                        match self.make_call(parts[1]).await {
                            Ok(call_id) => println!("📞 Call initiated: {}", call_id),
                            Err(e) => println!("❌ Call failed: {}", e),
                        }
                    } else {
                        println!("Usage: call <sip_uri>");
                    }
                },
                "answer" => {
                    if parts.len() > 1 {
                        match self.answer_call(parts[1]).await {
                            Ok(_) => println!("✅ Call answered"),
                            Err(e) => println!("❌ Failed to answer: {}", e),
                        }
                    } else {
                        println!("Usage: answer <call_id>");
                    }
                },
                "hangup" => {
                    if parts.len() > 1 {
                        match self.hangup_call(parts[1]).await {
                            Ok(_) => println!("📴 Call ended"),
                            Err(e) => println!("❌ Failed to hangup: {}", e),
                        }
                    } else {
                        println!("Usage: hangup <call_id>");
                    }
                },
                "hold" => {
                    if parts.len() > 1 {
                        self.hold_call(parts[1]).await.ok();
                    }
                },
                "unhold" => {
                    if parts.len() > 1 {
                        self.unhold_call(parts[1]).await.ok();
                    }
                },
                "contacts" => {
                    self.list_contacts().await;
                },
                "history" => {
                    self.show_call_history().await;
                },
                "quit" => break,
                _ => println!("Unknown command: {}", parts[0]),
            }
        }

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Starting Softphone Client");

    // Get credentials from command line or use defaults
    let args: Vec<String> = std::env::args().collect();
    let username = args.get(1).cloned().unwrap_or_else(|| "user".to_string());
    let server = args.get(2).cloned().unwrap_or_else(|| "localhost".to_string());
    let password = args.get(3).cloned().unwrap_or_else(|| "password".to_string());

    let account = UserAccount {
        username: username.clone(),
        display_name: format!("Softphone User ({})", username),
        sip_uri: format!("sip:{}@{}", username, server),
        server: server.clone(),
        password,
        registered: false,
    };

    let mut client = SoftphoneClient::new(account).await?;

    // Register with SIP server
    client.register().await?;

    // Add some demo contacts
    client.add_contact(Contact {
        id: "alice".to_string(),
        name: "Alice Johnson".to_string(),
        sip_uri: "sip:alice@example.com".to_string(),
        phone_number: Some("+1-555-0123".to_string()),
        company: Some("ACME Corp".to_string()),
        favorite: true,
    }).await;

    client.add_contact(Contact {
        id: "bob".to_string(),
        name: "Bob Smith".to_string(),
        sip_uri: "sip:bob@company.com".to_string(),
        phone_number: Some("+1-555-0456".to_string()),
        company: Some("Tech Inc".to_string()),
        favorite: false,
    }).await;

    // Start interactive mode
    client.interactive_mode().await?;

    println!("👋 Softphone shutting down");
    Ok(())
} 