//! Multi-device presence example
//!
//! This demonstrates how users-core supports multiple device registrations
//! for presence aggregation in session-core-v2

use anyhow::Result;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use users_core::{init, CreateUserRequest, UsersConfig};

// Simulated device registration
#[derive(Debug, Clone)]
struct DeviceRegistration {
    user_id: String,
    username: String,
    device_id: String,
    device_type: DeviceType,
    contact: String,
    user_agent: String,
    token: String,
    registered_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
enum DeviceType {
    Desktop,
    Mobile,
    WebBrowser,
    DeskPhone,
}

// Simulated presence state
#[derive(Debug, Clone)]
enum PresenceState {
    Available,
    Busy,
    DoNotDisturb,
    Away,
    Offline,
}

#[derive(Debug)]
struct UserPresence {
    user_id: String,
    overall_state: PresenceState,
    devices: Vec<DevicePresence>,
    last_activity: DateTime<Utc>,
}

#[derive(Debug)]
struct DevicePresence {
    device_id: String,
    device_type: DeviceType,
    state: PresenceState,
    status_message: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("📱 Multi-Device Presence Example\n");

    // Initialize users-core
    let config = UsersConfig {
        database_url: "sqlite://presence_example.db?mode=rwc".to_string(),
        ..Default::default()
    };

    let auth_service = init(config).await?;

    // Create a user who will use multiple devices
    println!("📝 Creating user with presence capability...");

    let user = auth_service
        .create_user(CreateUserRequest {
            username: "alice".to_string(),
            password: "SecurePresence2024!".to_string(),
            email: Some("alice@company.com".to_string()),
            display_name: Some("Alice Johnson".to_string()),
            roles: vec!["user".to_string()],
        })
        .await?;

    println!("✅ Created user: {}", user.username);

    // Simulate device registrations
    let mut device_registrations = HashMap::new();

    println!("\n🖥️ Device 1: Desktop Softphone");
    let desktop_auth = auth_service
        .authenticate_password("alice", "SecurePresence2024!")
        .await?;

    let desktop_reg = DeviceRegistration {
        user_id: user.id.clone(),
        username: user.username.clone(),
        device_id: "desktop-001".to_string(),
        device_type: DeviceType::Desktop,
        contact: "sip:alice@192.168.1.50:5060".to_string(),
        user_agent: "RVoIP Desktop/1.0".to_string(),
        token: desktop_auth.access_token.clone(),
        registered_at: Utc::now(),
    };

    device_registrations.insert(desktop_reg.device_id.clone(), desktop_reg.clone());
    println!("   ✓ Registered from {}", desktop_reg.contact);
    println!("   ✓ User Agent: {}", desktop_reg.user_agent);

    println!("\n📱 Device 2: Mobile App");
    let mobile_auth = auth_service
        .authenticate_password("alice", "SecurePresence2024!")
        .await?;

    let mobile_reg = DeviceRegistration {
        user_id: user.id.clone(),
        username: user.username.clone(),
        device_id: "mobile-001".to_string(),
        device_type: DeviceType::Mobile,
        contact: "sip:alice@10.0.0.100:5060;transport=tcp".to_string(),
        user_agent: "RVoIP Mobile/2.0 (iOS)".to_string(),
        token: mobile_auth.access_token.clone(),
        registered_at: Utc::now(),
    };

    device_registrations.insert(mobile_reg.device_id.clone(), mobile_reg.clone());
    println!("   ✓ Registered from {}", mobile_reg.contact);
    println!("   ✓ User Agent: {}", mobile_reg.user_agent);

    println!("\n🌐 Device 3: Web Browser");
    let web_auth = auth_service
        .authenticate_password("alice", "SecurePresence2024!")
        .await?;

    let web_reg = DeviceRegistration {
        user_id: user.id.clone(),
        username: user.username.clone(),
        device_id: "web-001".to_string(),
        device_type: DeviceType::WebBrowser,
        contact: "sip:alice@wss.company.com;transport=wss".to_string(),
        user_agent: "Mozilla/5.0 RVoIP-WebRTC/1.0".to_string(),
        token: web_auth.access_token.clone(),
        registered_at: Utc::now(),
    };

    device_registrations.insert(web_reg.device_id.clone(), web_reg.clone());
    println!("   ✓ Registered from {}", web_reg.contact);
    println!("   ✓ User Agent: {}", web_reg.user_agent);

    // Show device summary
    println!("\n📊 Device Registration Summary:");
    println!("   User: {}", user.username);
    println!("   User ID: {}", user.id);
    println!("   Total devices: {}", device_registrations.len());
    println!("   Each device has unique JWT token");

    // Simulate presence updates
    println!("\n🟢 Simulating presence updates...");

    let mut user_presence = UserPresence {
        user_id: user.id.clone(),
        overall_state: PresenceState::Available,
        devices: vec![],
        last_activity: Utc::now(),
    };

    // Desktop is available
    user_presence.devices.push(DevicePresence {
        device_id: "desktop-001".to_string(),
        device_type: DeviceType::Desktop,
        state: PresenceState::Available,
        status_message: Some("In office".to_string()),
    });
    println!("   Desktop: Available - 'In office'");

    // Mobile is busy
    user_presence.devices.push(DevicePresence {
        device_id: "mobile-001".to_string(),
        device_type: DeviceType::Mobile,
        state: PresenceState::Busy,
        status_message: Some("On a call".to_string()),
    });
    println!("   Mobile: Busy - 'On a call'");

    // Web is away
    user_presence.devices.push(DevicePresence {
        device_id: "web-001".to_string(),
        device_type: DeviceType::WebBrowser,
        state: PresenceState::Away,
        status_message: None,
    });
    println!("   Web: Away");

    // Calculate overall presence
    println!("\n🔄 Aggregating presence state...");
    user_presence.overall_state = calculate_overall_presence(&user_presence.devices);
    println!("   Overall state: {:?}", user_presence.overall_state);

    // Show presence priority rules
    println!("\n📋 Presence Aggregation Rules:");
    println!("   1. If any device is 'Busy' → Overall: Busy");
    println!("   2. Else if any device is 'Available' → Overall: Available");
    println!("   3. Else if any device is 'Away' → Overall: Away");
    println!("   4. Else if any device is 'Do Not Disturb' → Overall: Do Not Disturb");
    println!("   5. Else → Overall: Offline");

    // Simulate device logout
    println!("\n🚪 Device logout scenario...");

    // Mobile device logs out
    device_registrations.remove("mobile-001");
    user_presence
        .devices
        .retain(|d| d.device_id != "mobile-001");

    println!("   Mobile device logged out");
    println!("   Remaining devices: {}", device_registrations.len());

    // Recalculate presence
    user_presence.overall_state = calculate_overall_presence(&user_presence.devices);
    println!("   New overall state: {:?}", user_presence.overall_state);

    // Show presence document (PIDF format hint)
    println!("\n📄 Presence Document (PIDF) Structure:");
    println!("   <?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    println!("   <presence xmlns=\"urn:ietf:params:xml:ns:pidf\"");
    println!("             entity=\"sip:{}@company.com\">", user.username);
    for device in &user_presence.devices {
        println!("     <tuple id=\"{}\">", device.device_id);
        println!("       <status>");
        println!(
            "         <basic>{}</basic>",
            match device.state {
                PresenceState::Available => "open",
                _ => "closed",
            }
        );
        println!("       </status>");
        println!("     </tuple>");
    }
    println!("   </presence>");

    // Show subscription handling
    println!("\n📬 Presence Subscription Flow:");
    println!("   1. Watcher sends SUBSCRIBE for alice@company.com");
    println!("   2. Presence server authenticates watcher's token");
    println!("   3. Check if watcher is authorized (buddy list)");
    println!("   4. Send initial NOTIFY with current presence");
    println!("   5. Send NOTIFY on any device state change");

    // Best practices
    println!("\n💡 Multi-Device Presence Best Practices:");
    println!("   • Each device gets unique token (security)");
    println!("   • Aggregate presence based on priority rules");
    println!("   • Handle device timeouts gracefully");
    println!("   • Support rich presence (status messages)");
    println!("   • Implement presence authorization (privacy)");
    println!("   • Use PIDF for standard compliance");

    // Clean up
    std::fs::remove_file("presence_example.db").ok();

    println!("\n✨ Multi-device presence example completed!");
    Ok(())
}

fn calculate_overall_presence(devices: &[DevicePresence]) -> PresenceState {
    // Priority: Busy > Available > Away > DND > Offline
    if devices
        .iter()
        .any(|d| matches!(d.state, PresenceState::Busy))
    {
        PresenceState::Busy
    } else if devices
        .iter()
        .any(|d| matches!(d.state, PresenceState::Available))
    {
        PresenceState::Available
    } else if devices
        .iter()
        .any(|d| matches!(d.state, PresenceState::Away))
    {
        PresenceState::Away
    } else if devices
        .iter()
        .any(|d| matches!(d.state, PresenceState::DoNotDisturb))
    {
        PresenceState::DoNotDisturb
    } else {
        PresenceState::Offline
    }
}
