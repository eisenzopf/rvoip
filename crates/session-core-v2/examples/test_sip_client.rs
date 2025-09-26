//! Test Client for SIP Registrar Server
//! 
//! This example shows how a SIP client would interact with the registrar server.
//! It demonstrates the complete flow: authenticate → get token → register with SIP.

use reqwest;
use serde::{Deserialize, Serialize};
use std::error::Error;

#[derive(Serialize)]
struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Deserialize)]
struct LoginResponse {
    access_token: String,
    refresh_token: String,
    token_type: String,
    expires_in: u64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("SIP Client Test - Registration Flow Demo\n");
    
    // ==========================================
    // Step 1: Authenticate with users-core
    // ==========================================
    println!("Step 1: Authenticating with users-core...");
    
    let client = reqwest::Client::new();
    let login_req = LoginRequest {
        username: "alice".to_string(),
        password: "SecurePass123!".to_string(),
    };
    
    let auth_response = client
        .post("http://localhost:8081/auth/login")
        .json(&login_req)
        .send()
        .await?;
    
    if !auth_response.status().is_success() {
        eprintln!("Authentication failed: {}", auth_response.status());
        eprintln!("Make sure the server is running first!");
        return Ok(());
    }
    
    let login_resp: LoginResponse = auth_response.json().await?;
    println!("✓ Got JWT token: {}...", &login_resp.access_token[..20]);
    println!("  Token expires in: {} seconds", login_resp.expires_in);
    
    // ==========================================
    // Step 2: Register with SIP server
    // ==========================================
    println!("\nStep 2: Sending SIP REGISTER with JWT token...");
    
    // In a real implementation, you would use a SIP library like rvoip_sip_client
    // For this demo, we'll show what the SIP message would look like:
    
    let sip_register = format!(
        r#"REGISTER sip:example.com SIP/2.0
Via: SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bK776asdhds
Max-Forwards: 70
From: <sip:alice@example.com>;tag=1928301774
To: <sip:alice@example.com>
Call-ID: a84b4c76e66710@pc33.example.com
CSeq: 1 REGISTER
Contact: <sip:alice@192.168.1.100:5060>
Authorization: Bearer {}
Expires: 3600
User-Agent: RVoIP Test Client/1.0
Content-Length: 0

"#, login_resp.access_token);
    
    println!("Would send this SIP message to server:5060:");
    println!("-------------------------------------------");
    println!("{}", sip_register);
    println!("-------------------------------------------");
    
    // ==========================================
    // Step 3: Expected server response
    // ==========================================
    println!("\nStep 3: Expected server response:");
    println!("-------------------------------------------");
    println!(r#"SIP/2.0 200 OK
Via: SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bK776asdhds
From: <sip:alice@example.com>;tag=1928301774
To: <sip:alice@example.com>;tag=37GkEhwl6
Call-ID: a84b4c76e66710@pc33.example.com
CSeq: 1 REGISTER
Contact: <sip:alice@192.168.1.100:5060>;expires=3600
Server: RVoIP/1.0
Content-Length: 0
"#);
    println!("-------------------------------------------");
    
    // ==========================================
    // Using rvoip_sip_client (when available)
    // ==========================================
    println!("\nUsing rvoip_sip_client library (pseudo-code):");
    println!(r#"
    use rvoip_sip_client::SipClient;
    
    // Create SIP client
    let sip_client = SipClient::new("192.168.1.100:5060").await?;
    
    // Register with JWT token
    sip_client.register(
        "sip:example.com",
        "alice",
        Some(login_resp.access_token), // JWT token
        3600, // expires
    ).await?;
    
    // Now alice is registered and can:
    // - Make calls: sip_client.call("sip:bob@example.com").await?
    // - Send messages: sip_client.message("bob", "Hello!").await?
    // - Update presence: sip_client.publish_presence(Available).await?
    "#);
    
    Ok(())
}

/* 
Complete Working Example with pjsua:

1. Save your token to a file:
   ```bash
   TOKEN=$(curl -s -X POST http://localhost:8081/auth/login \
     -H 'Content-Type: application/json' \
     -d '{"username":"alice","password":"SecurePass123!"}' \
     | jq -r '.access_token')
   echo $TOKEN > alice.token
   ```

2. Create pjsua config file (alice.cfg):
   ```
   --id sip:alice@example.com
   --registrar sip:example.com:5060
   --realm example.com
   --username alice
   --password dummy
   ```

3. Use a SIP testing tool that supports custom headers:
   ```bash
   sipp -sn uac_register -s alice -i 192.168.1.100 -p 5061 \
        -m 1 -set token $TOKEN localhost:5060
   ```

Or use the RVoIP SIP client when it supports JWT auth!
*/
