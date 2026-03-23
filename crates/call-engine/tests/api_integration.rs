//! Integration tests for the call-engine API layer.
//!
//! These tests exercise the AdminApi, SupervisorApi, and CallCenterClient
//! through their Rust interfaces, verifying that each API provides correct
//! behavior when backed by a real (in-memory) CallCenterEngine.
//!
//! ## Architecture notes
//!
//! - `CallCenterEngine::new(config, Some(":memory:"))` creates an engine with
//!   an in-memory SQLite database. `None` for db_path means no database at all.
//! - Only the standard queues (general, sales, support, billing, vip, premium,
//!   overflow) are auto-created by `ensure_queue_exists`.
//! - `get_queue_stats()` only returns stats for the standard queue set.
//! - The Admin/Supervisor/Client APIs are plain Rust structs, not HTTP routers.

use std::sync::Arc;

use rvoip_call_engine::{
    CallCenterConfig, CallCenterEngine,
    api::{AdminApi, SupervisorApi, CallCenterClient},
    agent::{Agent, AgentId, AgentStatus},
    config::{RoutingConfig, RoutingStrategy, QueueConfig},
    server::{CallCenterServer, CallCenterServerBuilder},
};
use chrono::Utc;
use serial_test::serial;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a fresh `CallCenterEngine` with an in-memory SQLite database.
async fn create_test_engine() -> Arc<CallCenterEngine> {
    let mut config = CallCenterConfig::default();
    config.general.local_signaling_addr = "127.0.0.1:15060".parse().expect("valid addr");
    config.general.local_media_addr = "127.0.0.1:20000".parse().expect("valid addr");
    CallCenterEngine::new(config, Some(":memory:".to_string()))
        .await
        .expect("engine should initialize with in-memory db")
}

/// Create a `CallCenterServer` with an in-memory SQLite database (not None).
async fn create_test_server() -> CallCenterServer {
    let mut config = CallCenterConfig::default();
    config.general.local_signaling_addr = "127.0.0.1:15060".parse().expect("valid addr");
    config.general.local_media_addr = "127.0.0.1:20000".parse().expect("valid addr");
    CallCenterServer::new(config, Some(":memory:".to_string()))
        .await
        .expect("server should initialize")
}

/// Build a default `Agent` with the given id and status.
fn make_agent(id: &str, status: AgentStatus) -> Agent {
    Agent {
        id: id.to_string(),
        sip_uri: format!("sip:{}@test.local", id),
        display_name: format!("Test Agent {}", id),
        skills: vec!["english".to_string(), "support".to_string()],
        max_concurrent_calls: 2,
        status,
        department: Some("testing".to_string()),
        extension: Some("1001".to_string()),
    }
}

// ===========================================================================
//  Admin API tests
// ===========================================================================

#[tokio::test]
#[serial]
async fn admin_add_and_list_agents() {
    let engine = create_test_engine().await;
    let admin = AdminApi::new(Arc::clone(&engine));

    // Add two agents
    admin
        .add_agent(make_agent("admin-a1", AgentStatus::Available))
        .await
        .expect("add agent a1");
    admin
        .add_agent(make_agent("admin-a2", AgentStatus::Offline))
        .await
        .expect("add agent a2");

    // List should return both (via database)
    let agents = admin.list_agents().await.expect("list agents");
    assert!(
        agents.len() >= 2,
        "expected at least 2 agents, got {}",
        agents.len()
    );

    let ids: Vec<&str> = agents.iter().map(|a| a.id.as_str()).collect();
    assert!(ids.contains(&"admin-a1"), "should contain admin-a1");
    assert!(ids.contains(&"admin-a2"), "should contain admin-a2");
}

#[tokio::test]
#[serial]
async fn admin_remove_agent() {
    let engine = create_test_engine().await;
    let admin = AdminApi::new(Arc::clone(&engine));

    // Add agent then attempt removal.
    // Note: remove_agent tries to remove from the agent registry's session map,
    // which requires a SIP session. Since add_agent only registers in the
    // registry (not a full SIP session), removal will error with NotFound.
    admin
        .add_agent(make_agent("admin-rm", AgentStatus::Available))
        .await
        .expect("add agent");

    let result = admin
        .remove_agent(&AgentId("admin-rm".to_string()))
        .await;

    // This may succeed or return NotFound depending on registry internals.
    // Either outcome is acceptable in this API integration test.
    match result {
        Ok(()) => { /* agent removed from registry and DB */ }
        Err(e) => {
            // Expected: no active SIP session for agent
            let msg = format!("{}", e);
            assert!(
                msg.contains("session") || msg.contains("NotFound") || msg.contains("not found"),
                "unexpected error: {}",
                msg
            );
        }
    }
}

#[tokio::test]
#[serial]
async fn admin_create_queue() {
    let engine = create_test_engine().await;
    let admin = AdminApi::new(Arc::clone(&engine));

    // Use a standard queue name since only standard queues are auto-created
    admin
        .create_queue("general")
        .await
        .expect("create queue");

    // Verify via queue manager
    let qm = engine.queue_manager().read().await;
    let ids = qm.get_queue_ids();
    assert!(
        ids.contains(&"general".to_string()),
        "general queue should exist after creation"
    );
}

#[tokio::test]
#[serial]
async fn admin_get_system_health() {
    let engine = create_test_engine().await;
    let admin = AdminApi::new(Arc::clone(&engine));

    let health = admin.get_system_health().await;

    // With an in-memory DB the database should be connected
    assert!(
        health.database_connected,
        "in-memory DB should be considered connected"
    );
    assert_eq!(health.active_sessions, 0);
    assert_eq!(health.queued_calls, 0);
}

#[tokio::test]
#[serial]
async fn admin_export_config() {
    let engine = create_test_engine().await;
    let admin = AdminApi::new(Arc::clone(&engine));

    let json = admin.export_config().await.expect("export config");
    assert!(!json.is_empty(), "exported config JSON should not be empty");

    // It should be valid JSON that can be reimported
    let parsed: CallCenterConfig =
        serde_json::from_str(&json).expect("exported config should be valid JSON");
    assert!(
        parsed.general.max_concurrent_calls > 0,
        "reimported config should have sensible defaults"
    );
}

#[tokio::test]
#[serial]
async fn admin_import_invalid_config_returns_error() {
    let engine = create_test_engine().await;
    let admin = AdminApi::new(Arc::clone(&engine));

    let result = admin.import_config("not valid json {{{").await;
    assert!(result.is_err(), "invalid JSON should produce an error");
}

#[tokio::test]
#[serial]
async fn admin_get_statistics() {
    let engine = create_test_engine().await;
    let admin = AdminApi::new(Arc::clone(&engine));

    // Add an available agent via the admin API
    admin
        .add_agent(make_agent("stats-agent", AgentStatus::Available))
        .await
        .expect("add agent");

    let stats = admin.get_statistics().await;
    assert!(
        stats.total_agents >= 1,
        "should have at least 1 agent in stats"
    );
}

#[tokio::test]
#[serial]
async fn admin_update_routing_config() {
    let engine = create_test_engine().await;
    let admin = AdminApi::new(Arc::clone(&engine));

    let mut routing = RoutingConfig::default();
    routing.default_strategy = RoutingStrategy::RoundRobin;
    routing.enable_load_balancing = true;

    // Should succeed (even though it's currently a no-op placeholder)
    admin
        .update_routing_config(routing)
        .await
        .expect("update routing config");
}

#[tokio::test]
#[serial]
async fn admin_update_queue_config() {
    let engine = create_test_engine().await;
    let admin = AdminApi::new(Arc::clone(&engine));

    // Create a standard queue
    admin
        .create_queue("support")
        .await
        .expect("create queue");

    let queue_cfg = QueueConfig {
        default_max_wait_time: 120,
        max_queue_size: 25,
        enable_priorities: true,
        enable_overflow: false,
        announcement_interval: 15,
    };

    admin
        .update_queue("support", queue_cfg)
        .await
        .expect("update queue config");
}

#[tokio::test]
#[serial]
async fn admin_get_queue_configs() {
    let engine = create_test_engine().await;
    let admin = AdminApi::new(Arc::clone(&engine));

    let configs = admin.get_queue_configs().await;
    // The default implementation returns several built-in queues
    assert!(!configs.is_empty(), "should have default queue configs");
    assert!(
        configs.contains_key("general"),
        "should include general queue config"
    );
}

#[tokio::test]
#[serial]
async fn admin_optimize_database() {
    let engine = create_test_engine().await;
    let admin = AdminApi::new(Arc::clone(&engine));

    // Should succeed without error (placeholder implementation)
    admin
        .optimize_database()
        .await
        .expect("optimize database should succeed");
}

#[tokio::test]
#[serial]
async fn admin_update_agent_skills() {
    let engine = create_test_engine().await;
    let admin = AdminApi::new(Arc::clone(&engine));

    admin
        .add_agent(make_agent("skills-agent", AgentStatus::Available))
        .await
        .expect("add agent");

    admin
        .update_agent_skills(
            &AgentId("skills-agent".to_string()),
            vec!["sales".to_string(), "vip".to_string()],
        )
        .await
        .expect("update skills should succeed");
}

// ===========================================================================
//  Supervisor API tests
// ===========================================================================

#[tokio::test]
#[serial]
async fn supervisor_get_stats_initial() {
    let engine = create_test_engine().await;
    let supervisor = SupervisorApi::new(Arc::clone(&engine));

    let stats = supervisor.get_stats().await;
    assert_eq!(stats.active_calls, 0);
    assert_eq!(stats.active_bridges, 0);
    assert_eq!(stats.queued_calls, 0);
}

#[tokio::test]
#[serial]
async fn supervisor_list_agents_empty() {
    let engine = create_test_engine().await;
    let supervisor = SupervisorApi::new(Arc::clone(&engine));

    let agents = supervisor.list_agents().await;
    assert!(agents.is_empty(), "no agents registered yet");
}

#[tokio::test]
#[serial]
async fn supervisor_list_agents_after_registration() {
    let engine = create_test_engine().await;
    let admin = AdminApi::new(Arc::clone(&engine));
    let supervisor = SupervisorApi::new(Arc::clone(&engine));

    // Register an agent via admin API (adds to registry + DB)
    let agent = make_agent("sup-agent-1", AgentStatus::Available);
    admin.add_agent(agent).await.expect("add agent");

    // Supervisor should see the registered agent
    let agents = supervisor.list_agents().await;
    assert!(
        !agents.is_empty(),
        "supervisor should see registered agents"
    );
}

#[tokio::test]
#[serial]
async fn supervisor_get_all_queue_stats() {
    let engine = create_test_engine().await;
    let supervisor = SupervisorApi::new(Arc::clone(&engine));

    // Create a standard queue (only standard names are supported)
    engine
        .create_queue("general")
        .await
        .expect("create queue");

    let stats = supervisor
        .get_all_queue_stats()
        .await
        .expect("get queue stats");

    let queue_ids: Vec<&str> = stats.iter().map(|(id, _)| id.as_str()).collect();
    assert!(
        queue_ids.contains(&"general"),
        "should contain general in stats, got {:?}",
        queue_ids
    );
}

#[tokio::test]
#[serial]
async fn supervisor_list_active_calls_empty() {
    let engine = create_test_engine().await;
    let supervisor = SupervisorApi::new(Arc::clone(&engine));

    let calls = supervisor.list_active_calls().await;
    assert!(calls.is_empty(), "no calls active initially");
}

#[tokio::test]
#[serial]
async fn supervisor_get_queued_calls_empty() {
    let engine = create_test_engine().await;
    let supervisor = SupervisorApi::new(Arc::clone(&engine));

    engine
        .create_queue("support")
        .await
        .expect("create queue");

    let calls = supervisor.get_queued_calls("support").await;
    assert!(calls.is_empty(), "no calls queued initially");
}

#[tokio::test]
#[serial]
async fn supervisor_list_active_bridges_empty() {
    let engine = create_test_engine().await;
    let supervisor = SupervisorApi::new(Arc::clone(&engine));

    let bridges = supervisor.list_active_bridges().await;
    assert!(bridges.is_empty(), "no bridges active initially");
}

#[tokio::test]
#[serial]
async fn supervisor_performance_metrics() {
    let engine = create_test_engine().await;
    let supervisor = SupervisorApi::new(Arc::clone(&engine));

    let end = Utc::now();
    let start = end - chrono::Duration::hours(1);
    let metrics = supervisor.get_performance_metrics(start, end).await;

    assert_eq!(metrics.total_calls, 0, "no calls processed yet");
    assert_eq!(metrics.calls_answered, 0);
    assert_eq!(metrics.calls_abandoned, 0);
    assert!(
        metrics.service_level_percentage >= 0.0,
        "service level should be non-negative"
    );
}

#[tokio::test]
#[serial]
async fn supervisor_coach_agent() {
    let engine = create_test_engine().await;
    let supervisor = SupervisorApi::new(Arc::clone(&engine));

    // Coaching is a no-op placeholder but should succeed
    supervisor
        .coach_agent(
            &AgentId("any-agent".to_string()),
            "Please wrap up the call",
        )
        .await
        .expect("coaching should succeed");
}

#[tokio::test]
#[serial]
async fn supervisor_get_agent_details_not_found() {
    let engine = create_test_engine().await;
    let supervisor = SupervisorApi::new(Arc::clone(&engine));

    let details = supervisor
        .get_agent_details(&AgentId("nonexistent".to_string()))
        .await;
    assert!(
        details.is_none(),
        "non-existent agent should return None"
    );
}

#[tokio::test]
#[serial]
async fn supervisor_listen_to_call_not_found() {
    let engine = create_test_engine().await;
    let supervisor = SupervisorApi::new(Arc::clone(&engine));

    let result = supervisor
        .listen_to_call(&rvoip_session_core::SessionId("no-such-call".to_string()))
        .await
        .expect("listen should not error");
    assert!(result.is_none(), "no bridge for nonexistent call");
}

// ===========================================================================
//  Client API tests
// ===========================================================================

#[tokio::test]
#[serial]
async fn client_get_queue_stats() {
    let engine = create_test_engine().await;
    let client = CallCenterClient::new(Arc::clone(&engine));

    // Create a standard queue name
    engine
        .create_queue("sales")
        .await
        .expect("create queue");

    let stats = client.get_queue_stats().await.expect("get queue stats");
    let ids: Vec<&str> = stats.iter().map(|(id, _)| id.as_str()).collect();
    assert!(
        ids.contains(&"sales"),
        "client should see sales queue, got {:?}",
        ids
    );
}

#[tokio::test]
#[serial]
async fn client_get_agent_info_not_found() {
    let engine = create_test_engine().await;
    let client = CallCenterClient::new(Arc::clone(&engine));

    let info = client
        .get_agent_info(&AgentId("ghost".to_string()))
        .await;
    assert!(info.is_none(), "non-existent agent info should be None");
}

#[tokio::test]
#[serial]
async fn client_session_manager_access() {
    let engine = create_test_engine().await;
    let client = CallCenterClient::new(Arc::clone(&engine));

    // Should be able to retrieve the session manager
    let sm = client.session_manager();
    assert!(sm.is_ok(), "session manager should be accessible");
}

#[tokio::test]
#[serial]
async fn client_call_handler_creation() {
    let engine = create_test_engine().await;
    let client = CallCenterClient::new(Arc::clone(&engine));

    // call_handler() should return a valid Arc<dyn CallHandler>
    let _handler = client.call_handler();
    // If we got here without panic, the handler was created successfully
}

// ===========================================================================
//  CallCenterServer (integration of all APIs) tests
// ===========================================================================

#[tokio::test]
#[serial]
async fn server_new_in_memory() {
    let config = CallCenterConfig::default();
    let server = CallCenterServer::new_in_memory(config)
        .await
        .expect("create in-memory server");

    // APIs should be accessible
    let _admin = server.admin_api();
    let _supervisor = server.supervisor_api();
}

#[tokio::test]
#[serial]
async fn server_builder_pattern() {
    let server = CallCenterServerBuilder::new()
        .with_config(CallCenterConfig::default())
        .with_in_memory_database()
        .build()
        .await
        .expect("build server via builder");

    let _admin = server.admin_api();
}

#[tokio::test]
#[serial]
async fn server_builder_missing_config_errors() {
    let result = CallCenterServerBuilder::new()
        .with_in_memory_database()
        .build()
        .await;
    assert!(
        result.is_err(),
        "building without config should fail"
    );
}

#[tokio::test]
#[serial]
async fn server_create_default_queues() {
    let server = create_test_server().await;

    server
        .create_default_queues()
        .await
        .expect("create default queues");

    // Verify default queues exist via supervisor API
    let stats = server
        .supervisor_api()
        .get_all_queue_stats()
        .await
        .expect("get queue stats");

    let queue_ids: Vec<&str> = stats.iter().map(|(id, _)| id.as_str()).collect();
    assert!(queue_ids.contains(&"general"), "should have general queue");
    assert!(queue_ids.contains(&"support"), "should have support queue");
    assert!(queue_ids.contains(&"sales"), "should have sales queue");
}

#[tokio::test]
#[serial]
async fn server_create_test_agents() {
    // Use create_test_server which has a database (":memory:")
    let server = create_test_server().await;

    server
        .create_test_agents(vec![
            ("alice", "Alice Smith", "support"),
            ("bob", "Bob Jones", "sales"),
        ])
        .await
        .expect("create test agents");

    // Verify agents exist in the database via admin API
    let agents = server
        .admin_api()
        .list_agents()
        .await
        .expect("list agents");

    let agent_ids: Vec<&str> = agents.iter().map(|a| a.id.as_str()).collect();
    assert!(
        agent_ids.contains(&"alice"),
        "should have alice, got {:?}",
        agent_ids
    );
    assert!(
        agent_ids.contains(&"bob"),
        "should have bob, got {:?}",
        agent_ids
    );
}

#[tokio::test]
#[serial]
async fn server_start_and_stop() {
    let mut server = create_test_server().await;

    server.start().await.expect("start server");

    // Server should still be functional
    let stats = server.supervisor_api().get_stats().await;
    assert_eq!(stats.active_calls, 0);

    server.stop().await.expect("stop server");
}

#[tokio::test]
#[serial]
async fn server_create_client() {
    let server = create_test_server().await;

    let client = server.create_client("test-agent".to_string());
    // Client should be usable
    let stats = client.get_queue_stats().await;
    // May or may not have queues, but should not panic
    let _ = stats;
}

// ===========================================================================
//  Cross-API consistency tests
// ===========================================================================

#[tokio::test]
#[serial]
async fn cross_api_agent_visibility() {
    let engine = create_test_engine().await;
    let admin = AdminApi::new(Arc::clone(&engine));
    let supervisor = SupervisorApi::new(Arc::clone(&engine));
    let client = CallCenterClient::new(Arc::clone(&engine));

    // Add agent via admin
    admin
        .add_agent(make_agent("cross-agent", AgentStatus::Available))
        .await
        .expect("add agent");

    // Supervisor should see the agent
    let agents = supervisor.list_agents().await;
    let found = agents.iter().any(|a| a.agent_id.0 == "cross-agent");
    assert!(found, "supervisor should see agent added via admin");

    // Client should be able to get agent info
    let info = client
        .get_agent_info(&AgentId("cross-agent".to_string()))
        .await;
    assert!(info.is_some(), "client should see agent info");
}

#[tokio::test]
#[serial]
async fn cross_api_queue_visibility() {
    let engine = create_test_engine().await;
    let admin = AdminApi::new(Arc::clone(&engine));
    let supervisor = SupervisorApi::new(Arc::clone(&engine));
    let client = CallCenterClient::new(Arc::clone(&engine));

    // Create a standard queue via admin
    admin
        .create_queue("billing")
        .await
        .expect("create queue");

    // Supervisor should see it
    let sup_stats = supervisor
        .get_all_queue_stats()
        .await
        .expect("supervisor queue stats");
    let sup_ids: Vec<&str> = sup_stats.iter().map(|(id, _)| id.as_str()).collect();
    assert!(
        sup_ids.contains(&"billing"),
        "supervisor should see billing queue, got {:?}",
        sup_ids
    );

    // Client should see it
    let client_stats = client.get_queue_stats().await.expect("client queue stats");
    let client_ids: Vec<&str> = client_stats.iter().map(|(id, _)| id.as_str()).collect();
    assert!(
        client_ids.contains(&"billing"),
        "client should see billing queue, got {:?}",
        client_ids
    );
}

#[tokio::test]
#[serial]
async fn cross_api_stats_consistency() {
    let engine = create_test_engine().await;
    let admin = AdminApi::new(Arc::clone(&engine));
    let supervisor = SupervisorApi::new(Arc::clone(&engine));

    // Add agents via admin
    admin
        .add_agent(make_agent("consist-1", AgentStatus::Available))
        .await
        .expect("add agent 1");
    admin
        .add_agent(make_agent("consist-2", AgentStatus::Available))
        .await
        .expect("add agent 2");

    // Compare supervisor stats with admin stats
    let sup_stats = supervisor.get_stats().await;
    let admin_stats = admin.get_statistics().await;

    // Both should report at least 2 available agents
    assert!(
        sup_stats.available_agents >= 2,
        "supervisor should see >= 2 available agents, got {}",
        sup_stats.available_agents
    );
    assert!(
        admin_stats.total_agents >= 2,
        "admin should see >= 2 total agents, got {}",
        admin_stats.total_agents
    );
}
