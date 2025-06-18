//! Agent SIP Registration Demo
//!
//! This example demonstrates how agents would register with the call center
//! using SIP REGISTER and how the system tracks their availability.

use anyhow::Result;
use rvoip_call_engine::agent::{SipRegistrar, RegistrationStatus};
use rvoip_sip_core::{Contact, Address};
use rvoip_sip_core::prelude::ContactParamInfo;
use std::collections::HashMap;
use tokio::time::{sleep, Duration, interval};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    println!("🎯 SIP Agent Registration Demo\n");

    // Create a SIP registrar
    let mut registrar = SipRegistrar::new();

    // Simulate agent Alice registering
    println!("📱 Agent Alice registers from her softphone...");
    let alice_uri = "sip:alice@192.168.1.100:5060".parse()?;
    let mut alice_address = Address::new_with_display_name("Alice", alice_uri);
    alice_address.set_param("transport", Some("tcp"));
    let alice_contact_info = ContactParamInfo { address: alice_address };
    let alice_contact = Contact::new_params(vec![alice_contact_info]);
    
    let response = registrar.process_register(
        "sip:alice@callcenter.example.com",  // AOR (Address of Record)
        &alice_contact,
        Some(3600),  // 1 hour expiry
        Some("X-Lite/5.0".to_string()),
        "192.168.1.100:45678".to_string(),
    )?;
    
    match response.status {
        RegistrationStatus::Created => {
            println!("✅ Alice registered successfully!");
            if let Some(addr) = alice_contact.address() {
                println!("   - Contact: {}", addr.uri);
            }
            println!("   - Expires: {} seconds", response.expires);
        }
        _ => println!("❌ Unexpected registration status"),
    }

    // Simulate agent Bob registering from WebRTC
    println!("\n📱 Agent Bob registers from WebRTC client...");
    let bob_uri = "sip:bob@ws-client-xyz.example.com".parse()?;
    let mut bob_address = Address::new_with_display_name("Bob", bob_uri);
    bob_address.set_param("transport", Some("wss"));
    let bob_contact_info = ContactParamInfo { address: bob_address };
    let bob_contact = Contact::new_params(vec![bob_contact_info]);
    
    registrar.process_register(
        "sip:bob@callcenter.example.com",
        &bob_contact,
        Some(600),  // 10 minutes (typical for WebRTC)
        Some("WebPhone/2.0".to_string()),
        "203.0.113.45:8443".to_string(),
    )?;
    println!("✅ Bob registered successfully!");

    // Show current registrations
    println!("\n📊 Current Registrations:");
    for (aor, registration) in registrar.list_registrations() {
        println!("   {} -> {}", aor, registration.contact_uri);
        println!("      Transport: {}", registration.transport);
        println!("      User-Agent: {}", registration.user_agent.as_ref().unwrap_or(&"Unknown".to_string()));
    }

    // Demonstrate how to find where to route calls
    println!("\n📞 Routing a call to Alice:");
    if let Some(alice_reg) = registrar.get_registration("sip:alice@callcenter.example.com") {
        println!("   ➡️ Route to: {}", alice_reg.contact_uri);
        println!("   This is where we'd create an outbound INVITE");
    }

    // Simulate registration refresh
    println!("\n🔄 Alice refreshes her registration...");
    registrar.process_register(
        "sip:alice@callcenter.example.com",
        &alice_contact,
        Some(3600),
        Some("X-Lite/5.0".to_string()),
        "192.168.1.100:45678".to_string(),
    )?;
    println!("✅ Registration refreshed");

    // Simulate de-registration
    println!("\n📴 Bob logs out (de-registers)...");
    registrar.process_register(
        "sip:bob@callcenter.example.com",
        &bob_contact,
        Some(0),  // expires=0 means de-register
        Some("WebPhone/2.0".to_string()),
        "203.0.113.45:8443".to_string(),
    )?;
    println!("✅ Bob de-registered");

    // Show updated registrations
    println!("\n📊 Updated Registrations:");
    for (aor, registration) in registrar.list_registrations() {
        println!("   {} -> {}", aor, registration.contact_uri);
    }

    // Demonstrate the complete flow
    println!("\n🔄 Complete Agent Registration Flow:");
    println!("1. Agent starts softphone/WebRTC client");
    println!("2. Client sends REGISTER to call center");
    println!("3. Call center authenticates agent (TODO: implement auth)");
    println!("4. Call center stores registration with contact URI");
    println!("5. When customer calls arrive:");
    println!("   - Call center looks up agent's contact URI");
    println!("   - Creates outbound INVITE to agent");
    println!("   - Bridges customer and agent calls");
    println!("6. Agent must refresh registration before expiry");
    println!("7. On logout, agent sends REGISTER with expires=0");

    // Show what's missing for full integration
    println!("\n⚠️ Integration Requirements:");
    println!("- Session-core needs to handle REGISTER method");
    println!("- Need to link registration with agent database");
    println!("- Need authentication (digest auth)");
    println!("- Need to handle multiple registrations per agent");
    println!("- Need background task to clean expired registrations");

    Ok(())
} 