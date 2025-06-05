//! # 07 - Multi-Tenant Server
//! 
//! A multi-tenant SIP server that handles calls for multiple companies/organizations.
//! Each tenant has their own configuration, routing rules, and call handling.

use rvoip_session_core::api::simple::*;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashMap;
use tokio;

/// Multi-tenant server that manages multiple organizations
struct MultiTenantServer {
    tenants: Arc<Mutex<HashMap<String, Tenant>>>,
    default_tenant: String,
}

#[derive(Debug, Clone)]
struct Tenant {
    id: String,
    name: String,
    domain: String,
    config: TenantConfig,
    extensions: HashMap<String, Extension>,
}

#[derive(Debug, Clone)]
struct TenantConfig {
    business_hours: BusinessHours,
    voicemail_enabled: bool,
    call_recording: bool,
    max_concurrent_calls: usize,
    greeting_message: Option<String>,
    after_hours_message: Option<String>,
}

#[derive(Debug, Clone)]
struct BusinessHours {
    start_hour: u32,
    end_hour: u32,
    days: Vec<chrono::Weekday>,
}

#[derive(Debug, Clone)]
struct Extension {
    number: String,
    user_id: String,
    name: String,
    status: ExtensionStatus,
    forwarding: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum ExtensionStatus {
    Available,
    Busy,
    DoNotDisturb,
    Offline,
}

impl MultiTenantServer {
    fn new() -> Self {
        let mut tenants = HashMap::new();

        // Add demo tenants
        tenants.insert("acme-corp".to_string(), Tenant {
            id: "acme-corp".to_string(),
            name: "ACME Corporation".to_string(),
            domain: "acme-corp.com".to_string(),
            config: TenantConfig {
                business_hours: BusinessHours {
                    start_hour: 9,
                    end_hour: 17,
                    days: vec![
                        chrono::Weekday::Mon, chrono::Weekday::Tue, 
                        chrono::Weekday::Wed, chrono::Weekday::Thu, 
                        chrono::Weekday::Fri
                    ],
                },
                voicemail_enabled: true,
                call_recording: true,
                max_concurrent_calls: 100,
                greeting_message: Some("assets/acme_greeting.wav".to_string()),
                after_hours_message: Some("assets/acme_after_hours.wav".to_string()),
            },
            extensions: {
                let mut ext = HashMap::new();
                ext.insert("100".to_string(), Extension {
                    number: "100".to_string(),
                    user_id: "john.doe".to_string(),
                    name: "John Doe - Sales".to_string(),
                    status: ExtensionStatus::Available,
                    forwarding: None,
                });
                ext.insert("101".to_string(), Extension {
                    number: "101".to_string(),
                    user_id: "jane.smith".to_string(),
                    name: "Jane Smith - Support".to_string(),
                    status: ExtensionStatus::Available,
                    forwarding: None,
                });
                ext
            },
        });

        tenants.insert("tech-startup".to_string(), Tenant {
            id: "tech-startup".to_string(),
            name: "Tech Startup Inc".to_string(),
            domain: "techstartup.io".to_string(),
            config: TenantConfig {
                business_hours: BusinessHours {
                    start_hour: 8,
                    end_hour: 20,
                    days: vec![
                        chrono::Weekday::Mon, chrono::Weekday::Tue, 
                        chrono::Weekday::Wed, chrono::Weekday::Thu, 
                        chrono::Weekday::Fri, chrono::Weekday::Sat
                    ],
                },
                voicemail_enabled: true,
                call_recording: false,
                max_concurrent_calls: 25,
                greeting_message: Some("assets/startup_greeting.wav".to_string()),
                after_hours_message: Some("assets/startup_after_hours.wav".to_string()),
            },
            extensions: {
                let mut ext = HashMap::new();
                ext.insert("200".to_string(), Extension {
                    number: "200".to_string(),
                    user_id: "ceo".to_string(),
                    name: "CEO".to_string(),
                    status: ExtensionStatus::Available,
                    forwarding: None,
                });
                ext
            },
        });

        Self {
            tenants: Arc::new(Mutex::new(tenants)),
            default_tenant: "acme-corp".to_string(),
        }
    }

    async fn identify_tenant(&self, call: &IncomingCall) -> Option<String> {
        // Try to identify tenant from:
        // 1. Domain in the To header
        // 2. Custom header (X-Tenant-ID)
        // 3. URL parameter
        // 4. Default to first tenant

        if let Some(tenant_id) = call.get_header("X-Tenant-ID") {
            return Some(tenant_id.to_string());
        }

        if let Some(tenant_id) = call.get_parameter("tenant") {
            return Some(tenant_id.to_string());
        }

        // Extract domain from To header
        let to_uri = call.to();
        if let Some(domain) = extract_domain(to_uri) {
            let tenants = self.tenants.lock().await;
            for (tenant_id, tenant) in tenants.iter() {
                if tenant.domain == domain {
                    return Some(tenant_id.clone());
                }
            }
        }

        // Default tenant
        Some(self.default_tenant.clone())
    }

    async fn get_tenant(&self, tenant_id: &str) -> Option<Tenant> {
        let tenants = self.tenants.lock().await;
        tenants.get(tenant_id).cloned()
    }

    async fn is_business_hours(&self, tenant: &Tenant) -> bool {
        let now = chrono::Local::now();
        let hour = now.hour();
        let weekday = now.weekday();

        tenant.config.business_hours.days.contains(&weekday) &&
        hour >= tenant.config.business_hours.start_hour &&
        hour < tenant.config.business_hours.end_hour
    }

    async fn find_extension(&self, tenant: &Tenant, number: &str) -> Option<Extension> {
        tenant.extensions.get(number).cloned()
    }

    async fn route_call(&self, tenant: &Tenant, call: &IncomingCall) -> CallAction {
        let to_uri = call.to();
        
        // Extract extension number from URI (e.g., sip:100@domain.com -> "100")
        if let Some(extension_number) = extract_extension(to_uri) {
            if let Some(extension) = self.find_extension(tenant, &extension_number).await {
                match extension.status {
                    ExtensionStatus::Available => {
                        println!("📞 Routing to extension {}: {}", extension.number, extension.name);
                        return CallAction::Transfer {
                            target: format!("sip:{}@{}", extension.user_id, tenant.domain),
                        };
                    }
                    ExtensionStatus::Busy => {
                        if tenant.config.voicemail_enabled {
                            return CallAction::Voicemail {
                                mailbox: extension.user_id.clone(),
                            };
                        } else {
                            return CallAction::Reject {
                                reason: "Extension busy".to_string(),
                                play_message: None,
                            };
                        }
                    }
                    ExtensionStatus::DoNotDisturb => {
                        return CallAction::Reject {
                            reason: "Do not disturb".to_string(),
                            play_message: Some("assets/dnd_message.wav".to_string()),
                        };
                    }
                    ExtensionStatus::Offline => {
                        if let Some(forwarding) = &extension.forwarding {
                            return CallAction::Transfer {
                                target: forwarding.clone(),
                            };
                        } else if tenant.config.voicemail_enabled {
                            return CallAction::Voicemail {
                                mailbox: extension.user_id.clone(),
                            };
                        }
                    }
                }
            }
        }

        // Default routing - main number
        if self.is_business_hours(tenant).await {
            CallAction::Answer
        } else {
            CallAction::Reject {
                reason: "After business hours".to_string(),
                play_message: tenant.config.after_hours_message.clone(),
            }
        }
    }
}

impl CallHandler for MultiTenantServer {
    async fn on_incoming_call(&self, call: &IncomingCall) -> CallAction {
        let caller = call.from();
        println!("📞 Multi-Tenant: Incoming call from {}", caller);

        // Identify which tenant this call belongs to
        if let Some(tenant_id) = self.identify_tenant(call).await {
            if let Some(tenant) = self.get_tenant(&tenant_id).await {
                println!("🏢 Tenant identified: {} ({})", tenant.name, tenant_id);
                return self.route_call(&tenant, call).await;
            } else {
                println!("❌ Unknown tenant: {}", tenant_id);
            }
        }

        // Fallback for unidentified tenants
        CallAction::Reject {
            reason: "Unknown tenant".to_string(),
            play_message: None,
        }
    }

    async fn on_call_connected(&self, call: &ActiveCall) {
        let caller = call.remote_party();
        
        if let Some(tenant_id) = call.get_parameter("tenant") {
            if let Some(tenant) = self.get_tenant(&tenant_id).await {
                println!("✅ {} connected to {}", caller, tenant.name);
                
                // Play tenant-specific greeting
                if let Some(greeting) = &tenant.config.greeting_message {
                    call.play_audio_file(greeting).await.ok();
                }

                // Start call recording if enabled
                if tenant.config.call_recording {
                    let recording_file = format!("recordings/{}/{}.wav", 
                        tenant_id, 
                        chrono::Utc::now().timestamp()
                    );
                    call.start_recording(&recording_file).await.ok();
                }
            }
        }
    }

    async fn on_call_ended(&self, call: &ActiveCall, reason: &str) {
        let caller = call.remote_party();
        println!("📴 Multi-Tenant: Call ended with {}: {}", caller, reason);

        // Stop recording if active
        call.stop_recording().await.ok();
    }
}

// Helper functions
fn extract_domain(uri: &str) -> Option<String> {
    // Extract domain from SIP URI (e.g., "sip:user@domain.com" -> "domain.com")
    if let Some(at_pos) = uri.find('@') {
        let domain_part = &uri[at_pos + 1..];
        if let Some(param_pos) = domain_part.find(';') {
            Some(domain_part[..param_pos].to_string())
        } else {
            Some(domain_part.to_string())
        }
    } else {
        None
    }
}

fn extract_extension(uri: &str) -> Option<String> {
    // Extract extension from SIP URI (e.g., "sip:100@domain.com" -> "100")
    if let Some(colon_pos) = uri.find(':') {
        if let Some(at_pos) = uri.find('@') {
            if colon_pos < at_pos {
                return Some(uri[colon_pos + 1..at_pos].to_string());
            }
        }
    }
    None
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Starting Multi-Tenant Server");

    // Create recordings directories for tenants
    tokio::fs::create_dir_all("recordings/acme-corp").await?;
    tokio::fs::create_dir_all("recordings/tech-startup").await?;

    // Create session manager with default config
    let config = SessionConfig::default();
    let session_manager = SessionManager::new(config).await?;

    // Set our multi-tenant handler
    session_manager.set_call_handler(Arc::new(MultiTenantServer::new())).await?;

    // Start listening for incoming calls
    println!("🎧 Multi-tenant server listening on 0.0.0.0:5060");
    println!("🏢 Supporting tenants:");
    println!("   - ACME Corporation (acme-corp.com)");
    println!("   - Tech Startup Inc (techstartup.io)");
    println!("📞 Call sip:100@acme-corp.com for ACME extension 100");
    println!("📞 Call sip:200@techstartup.io for Startup extension 200");
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
    fn test_domain_extraction() {
        assert_eq!(extract_domain("sip:user@example.com"), Some("example.com".to_string()));
        assert_eq!(extract_domain("sip:100@acme-corp.com;transport=tcp"), Some("acme-corp.com".to_string()));
    }

    #[test]
    fn test_extension_extraction() {
        assert_eq!(extract_extension("sip:100@example.com"), Some("100".to_string()));
        assert_eq!(extract_extension("sip:support@company.com"), Some("support".to_string()));
    }

    #[tokio::test]
    async fn test_tenant_identification() {
        let server = MultiTenantServer::new();
        // Would need a mock IncomingCall to test properly
        let tenant = server.get_tenant("acme-corp").await;
        assert!(tenant.is_some());
        assert_eq!(tenant.unwrap().name, "ACME Corporation");
    }
} 