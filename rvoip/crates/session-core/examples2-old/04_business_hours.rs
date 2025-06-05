//! # 04 - Business Hours Server
//! 
//! A business hours server that accepts calls during 9-5 business hours
//! and rejects calls after hours with a professional message.

use rvoip_session_core::api::simple::*;
use std::sync::Arc;
use chrono::{Local, Timelike, Weekday};
use tokio;

/// Business hours server implementation
struct BusinessHoursServer {
    business_start: u32,  // 9 AM
    business_end: u32,    // 5 PM (17:00)
    after_hours_message: String,
}

impl BusinessHoursServer {
    fn new() -> Self {
        Self {
            business_start: 9,
            business_end: 17,
            after_hours_message: "assets/after_hours_message.wav".to_string(),
        }
    }

    fn is_business_hours(&self) -> bool {
        let now = Local::now();
        let hour = now.hour();
        let weekday = now.weekday();

        // Check if it's a weekday (Monday-Friday)
        let is_weekday = matches!(weekday, 
            Weekday::Mon | Weekday::Tue | Weekday::Wed | Weekday::Thu | Weekday::Fri
        );

        // Check if it's during business hours
        let is_business_time = hour >= self.business_start && hour < self.business_end;

        is_weekday && is_business_time
    }

    fn get_hours_info(&self) -> String {
        format!("Business hours: Monday-Friday {}:00 AM - {}:00 PM", 
            self.business_start, 
            if self.business_end > 12 { self.business_end - 12 } else { self.business_end }
        )
    }
}

impl CallHandler for BusinessHoursServer {
    async fn on_incoming_call(&self, call: &IncomingCall) -> CallAction {
        let caller = call.from();
        let now = Local::now();
        
        println!("📞 Business Hours: Call from {} at {}", caller, now.format("%A %H:%M"));

        if self.is_business_hours() {
            println!("🏢 Currently in business hours - accepting call");
            CallAction::Answer
        } else {
            println!("🌙 After business hours - rejecting call with message");
            CallAction::Reject {
                reason: "After business hours".to_string(),
                play_message: Some(self.after_hours_message.clone()),
            }
        }
    }

    async fn on_call_connected(&self, call: &ActiveCall) {
        println!("✅ Business call connected with {}", call.remote_party());
        
        // Optionally play a business greeting
        call.play_audio_file("assets/business_greeting.wav").await.ok();
    }

    async fn on_call_rejected(&self, call: &IncomingCall, reason: &str) {
        println!("🚫 After-hours call from {} rejected: {}", call.from(), reason);
    }

    async fn on_call_ended(&self, call: &ActiveCall, reason: &str) {
        println!("📴 Business call ended with {}: {}", call.remote_party(), reason);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Starting Business Hours Server");

    let server = BusinessHoursServer::new();
    println!("🕘 {}", server.get_hours_info());

    // Create session manager with default config
    let config = SessionConfig::default();
    let session_manager = SessionManager::new(config).await?;

    // Set our business hours handler
    session_manager.set_call_handler(Arc::new(server)).await?;

    // Start listening for incoming calls
    println!("🎧 Business hours server listening on 0.0.0.0:5060");
    session_manager.start_server("0.0.0.0:5060").await?;

    // Keep running
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_business_hours_logic() {
        let server = BusinessHoursServer::new();
        // Note: This test would need to be more sophisticated to mock time
        // For demonstration purposes only
        println!("Business hours check: {}", server.is_business_hours());
    }

    #[tokio::test]
    async fn test_business_hours_server() {
        let server = BusinessHoursServer::new();
        let mock_call = IncomingCall::mock("sip:customer@company.com");
        
        // The action will depend on current time
        let _action = server.on_incoming_call(&mock_call).await;
    }
} 