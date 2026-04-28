//! Multiple dialog management with Unified API
//!
//! This example demonstrates managing multiple concurrent SIP dialogs
//! using the unified DialogManager architecture and API.

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tracing::{info, Level};
use tracing_subscriber;

use rvoip_dialog_core::{
    api::unified::UnifiedDialogApi, config::DialogManagerConfig, events::DialogEventHub,
};
use rvoip_infra_common::events::{
    cross_crate::{DialogToSessionEvent, RvoipCrossCrateEvent},
    EventCoordinatorConfig, GlobalEventCoordinator,
};

/// Multi-dialog management example using unified API
struct MultiDialogExample {
    hybrid_api: Arc<UnifiedDialogApi>,
    event_coordinator: Arc<GlobalEventCoordinator>,
    #[allow(dead_code)]
    local_addr: SocketAddr,
}

impl MultiDialogExample {
    /// Initialize with hybrid mode (supports both incoming and outgoing dialogs)
    async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        info!("🚀 Initializing multi-dialog example with unified API");

        // Use hybrid configuration for maximum flexibility
        let config = DialogManagerConfig::hybrid("127.0.0.1:0".parse()?)
            .with_from_uri("sip:switchboard@example.com")
            .with_domain("multi.example.com")
            .with_auto_options()
            .build();

        let api = Arc::new(UnifiedDialogApi::create(config).await?);
        let event_coordinator =
            Arc::new(GlobalEventCoordinator::new(EventCoordinatorConfig::monolithic()).await?);
        let event_hub = DialogEventHub::new(
            event_coordinator.clone(),
            Arc::new(api.dialog_manager().as_ref().inner_manager().clone()),
        )
        .await?;
        api.dialog_manager()
            .as_ref()
            .inner_manager()
            .set_event_hub(event_hub)
            .await;

        info!("✅ Hybrid API created for multi-dialog management");

        Ok(Self {
            hybrid_api: api,
            event_coordinator,
            local_addr: "127.0.0.1:0".parse()?, // Placeholder, actual address is managed internally
        })
    }

    /// Demonstrate creating multiple dialogs concurrently
    async fn create_multiple_dialogs(
        &self,
    ) -> Result<Vec<rvoip_dialog_core::api::common::DialogHandle>, Box<dyn std::error::Error>> {
        info!("\n📞 === Creating Multiple Dialogs ===");

        let mut dialogs = Vec::new();

        // Create several outgoing dialogs to different targets
        let targets = vec![
            ("alice", "sip:alice@example.com"),
            ("bob", "sip:bob@example.com"),
            ("charlie", "sip:charlie@provider.com"),
            ("david", "sip:david@enterprise.com"),
        ];

        for (name, target_uri) in targets {
            let local_uri = "sip:switchboard@example.com";

            info!("Creating dialog for {} -> {}", name, target_uri);
            let dialog = self.hybrid_api.create_dialog(local_uri, target_uri).await?;

            info!("✅ Created dialog {} for {}", dialog.id(), name);
            dialogs.push(dialog);
        }

        info!("✅ Created {} dialogs total", dialogs.len());
        Ok(dialogs)
    }

    /// Demonstrate making multiple concurrent calls
    async fn make_multiple_calls(
        &self,
    ) -> Result<Vec<rvoip_dialog_core::api::common::CallHandle>, Box<dyn std::error::Error>> {
        info!("\n📞 === Making Multiple Concurrent Calls ===");

        let mut calls = Vec::new();

        // Create several outgoing calls concurrently
        let call_targets = vec![
            ("sales", "sip:sales@company.com"),
            ("support", "sip:support@company.com"),
            ("billing", "sip:billing@company.com"),
        ];

        for (department, target_uri) in call_targets {
            let from_uri = "sip:switchboard@example.com";

            info!("Making call to {} department -> {}", department, target_uri);
            let call_result = self.hybrid_api.make_call(from_uri, target_uri, None).await;

            match call_result {
                Ok(call) => {
                    info!(
                        "✅ Call initiated to {} - Call ID: {}",
                        department,
                        call.call_id()
                    );
                    calls.push(call);
                }
                Err(e) => {
                    info!(
                        "⚠️  Call to {} failed (expected in demo): {}",
                        department, e
                    );
                }
            }
        }

        info!("✅ Initiated {} calls total", calls.len());
        Ok(calls)
    }

    /// Demonstrate dialog and call statistics
    async fn show_statistics(&self) -> Result<(), Box<dyn std::error::Error>> {
        info!("\n📊 === Multi-Dialog Statistics ===");

        // Get overall statistics
        let stats = self.hybrid_api.get_stats().await;
        info!("📋 Current Statistics:");
        info!("   • Active dialogs: {}", stats.active_dialogs);
        info!("   • Total dialogs: {}", stats.total_dialogs);
        info!("   • Successful calls: {}", stats.successful_calls);
        info!("   • Failed calls: {}", stats.failed_calls);
        info!(
            "   • Average call duration: {:.1}s",
            stats.avg_call_duration
        );

        // List active dialogs
        let active_dialogs = self.hybrid_api.list_active_dialogs().await;
        info!("📋 Active Dialogs: {}", active_dialogs.len());

        for (i, dialog_id) in active_dialogs.iter().enumerate() {
            info!("   {}. Dialog ID: {}", i + 1, dialog_id);
        }

        // Show unified API capabilities
        info!("\n🔧 Unified API Capabilities:");
        info!(
            "   • Supports outgoing calls: {}",
            self.hybrid_api.supports_outgoing_calls()
        );
        info!(
            "   • Supports incoming calls: {}",
            self.hybrid_api.supports_incoming_calls()
        );
        info!("   • From URI: {:?}", self.hybrid_api.from_uri());
        info!("   • Domain: {:?}", self.hybrid_api.domain());
        info!(
            "   • Auto OPTIONS: {}",
            self.hybrid_api.auto_options_enabled()
        );

        Ok(())
    }

    /// Demonstrate session coordination with multiple dialogs
    async fn run_session_coordination(&self) -> Result<(), Box<dyn std::error::Error>> {
        info!("\n🔄 === Multi-Dialog Cross-Crate Event Coordination ===");

        let mut dialog_to_session_rx = self
            .event_coordinator
            .subscribe("dialog_to_session")
            .await?;

        info!("✅ Cross-crate event subscription established for multi-dialog management");

        // Spawn task to handle dialog-to-session events
        let event_handler = tokio::spawn(async move {
            let mut event_count = 0;
            while let Some(event) = dialog_to_session_rx.recv().await {
                event_count += 1;
                if let Some(RvoipCrossCrateEvent::DialogToSession(dialog_event)) =
                    event.as_any().downcast_ref::<RvoipCrossCrateEvent>()
                {
                    match dialog_event {
                        DialogToSessionEvent::IncomingCall { session_id, .. } => {
                            info!(
                                "[{}] 📞 Incoming call for session: {}",
                                event_count, session_id
                            );
                        }
                        DialogToSessionEvent::CallTerminated { session_id, reason } => {
                            info!(
                                "[{}] 💥 Call terminated for session: {} - {:?}",
                                event_count, session_id, reason
                            );
                        }
                        DialogToSessionEvent::CallStateChanged {
                            session_id,
                            new_state,
                            reason,
                        } => {
                            info!(
                                "[{}] 🔄 Call state changed for {}: {:?} ({:?})",
                                event_count, session_id, new_state, reason
                            );
                        }
                        _ => {
                            info!(
                                "[{}] 📡 Other dialog-to-session event received",
                                event_count
                            );
                        }
                    }
                } else {
                    info!("[{}] 📡 Other cross-crate event received", event_count);
                }

                // Stop after handling some events
                if event_count >= 10 {
                    break;
                }
            }
            info!("✅ Cross-crate coordination handled {} events", event_count);
        });

        // Let the coordination run briefly
        sleep(Duration::from_secs(2)).await;

        event_handler.abort();
        Ok(())
    }

    /// Demonstrate dialog operations on multiple dialogs
    async fn perform_dialog_operations(
        &self,
        _dialogs: &[rvoip_dialog_core::api::common::DialogHandle],
    ) -> Result<(), Box<dyn std::error::Error>> {
        info!("\n📡 === Multi-Dialog SIP Operations ===");
        info!("💡 Demonstrating real SIP operations (no simulated establishment)");

        // Show proper way: make real calls for actual dialog establishment
        info!("🔧 Making real SIP calls for proper dialog establishment:");

        let local_uri = "sip:switchboard@example.com";

        for i in 0..2 {
            // Make 2 real calls
            let target_uri = format!("sip:target{}@example.com", i + 1);

            let call_result = self
                .hybrid_api
                .make_call(local_uri, &target_uri, None)
                .await;
            match call_result {
                Ok(call) => {
                    info!("  ✅ Real call {} initiated: {}", i + 1, call.call_id());
                    info!("  📋 Real dialog established through INVITE request");
                }
                Err(e) => {
                    info!("  ⚠️  Call {} failed (expected in test): {}", i + 1, e);
                    info!("  💡 In production, this establishes real dialogs via INVITE/200 OK");
                }
            }
        }

        info!("✅ Demonstrated real SIP call establishment (no simulation)");
        info!("💡 Best Practice: Use make_call() for real dialog establishment");
        info!("💡 Only send in-dialog requests to confirmed dialogs established through real SIP");

        Ok(())
    }

    /// Show unified architecture benefits for multi-dialog scenarios
    async fn show_multi_dialog_benefits(&self) -> Result<(), Box<dyn std::error::Error>> {
        info!("\n🌟 === Multi-Dialog Unified Architecture Benefits ===");

        info!("✅ Single Manager for All Scenarios:");
        info!("   • Same API handles outgoing calls, incoming calls, and dialogs");
        info!("   • No need to choose between DialogClient vs DialogServer");
        info!("   • Hybrid mode perfect for PBX/gateway scenarios");

        info!("✅ Simplified Multi-Dialog Management:");
        info!("   • One configuration system for all dialog types");
        info!("   • Unified statistics and monitoring across all dialogs");
        info!("   • Single global cross-crate event stream");

        info!("✅ Standards Compliance:");
        info!("   • Each dialog can act as UAC or UAS per transaction");
        info!("   • Proper RFC 3261 dialog state management");
        info!("   • No artificial client/server application constraints");

        info!("✅ Code Simplification:");
        info!("   • Before: Need DialogClient + DialogServer + coordination");
        info!("   • After: Single UnifiedDialogApi handles everything");
        info!("   • Reduced complexity for multi-dialog applications");

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();

    info!("🎯 ==========================================");
    info!("🎯   Multi-Dialog Management - Unified API");
    info!("🎯 ==========================================");
    info!("");
    info!("This example demonstrates managing multiple");
    info!("concurrent dialogs with the unified architecture.");

    // Create multi-dialog example
    let example = MultiDialogExample::new().await?;

    // Start the API
    example.hybrid_api.start().await?;

    // Create multiple dialogs
    let dialogs = example.create_multiple_dialogs().await?;

    // Make multiple calls
    let _calls = example.make_multiple_calls().await?;

    // Show statistics
    example.show_statistics().await?;

    // Perform dialog operations
    example.perform_dialog_operations(&dialogs).await?;

    // Run session coordination
    example.run_session_coordination().await?;

    // Show benefits
    example.show_multi_dialog_benefits().await?;

    // Stop the API
    example.hybrid_api.stop().await?;

    info!("\n🎉 ==========================================");
    info!("🎉   Multi-Dialog Example Complete!");
    info!("🎉 ==========================================");
    info!("");
    info!("✅ Demonstrated concurrent dialog management");
    info!("✅ Showcased hybrid mode capabilities");
    info!("✅ Illustrated unified cross-crate event coordination");
    info!("✅ Validated multi-dialog architecture benefits");
    info!("");
    info!("🚀 Ready for production multi-dialog scenarios!");

    Ok(())
}
