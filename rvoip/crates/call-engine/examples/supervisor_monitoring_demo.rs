//! Supervisor Monitoring Demo
//!
//! This example demonstrates how supervisors can use the SupervisorApi
//! to monitor and manage call center operations in real-time.

use anyhow::Result;
use rvoip_call_engine::{
    prelude::*,
    api::{CallCenterClient, SupervisorApi, AdminApi},
    agent::{Agent, AgentId, AgentStatus},
};
use std::sync::Arc;
use tokio::time::{sleep, Duration, interval};
use chrono::{DateTime, Utc};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    println!("👁️ Supervisor Monitoring Demo\n");

    // Step 1: Set up the call center
    println!("🏢 Setting up call center...");
    let mut config = CallCenterConfig::default();
    config.general.local_signaling_addr = "127.0.0.1:5060".parse()?;
    let engine = CallCenterEngine::new(config, Some(":memory:".to_string())).await?;
    
    // Create APIs
    let admin_api = AdminApi::new(engine.clone());
    let agent_client = CallCenterClient::new(engine.clone());
    let supervisor_api = SupervisorApi::new(engine.clone());
    
    println!("✅ Call center ready\n");

    // Step 2: Add and register agents
    println!("👥 Setting up agents...");
    
    let agents = vec![
        ("alice", "Alice Smith", vec!["english", "sales"], "sales"),
        ("bob", "Bob Johnson", vec!["english", "support"], "support"),
        ("carlos", "Carlos Garcia", vec!["spanish", "english", "support"], "support"),
        ("diana", "Diana Chen", vec!["mandarin", "english", "sales"], "sales"),
    ];
    
    for (id, name, skills, dept) in agents {
        let agent = Agent {
            id: format!("{}-001", id),
            sip_uri: format!("sip:{}@127.0.0.1", id),
            display_name: name.to_string(),
            skills: skills.into_iter().map(|s| s.to_string()).collect(),
            max_concurrent_calls: 2,
            status: AgentStatus::Available,
            department: Some(dept.to_string()),
            extension: Some(format!("10{}", &id[0..2])),
        };
        
        admin_api.add_agent(agent.clone()).await?;
        agent_client.register_agent(&agent).await?;
        println!("  ✅ {} registered", name);
    }
    println!();

    // Step 3: Demonstrate supervisor monitoring capabilities
    println!("📊 === SUPERVISOR DASHBOARD ===\n");
    
    // 3.1: Real-time statistics
    let stats = supervisor_api.get_stats().await;
    println!("📈 Real-Time Statistics:");
    println!("  • Available Agents: {}", stats.available_agents);
    println!("  • Busy Agents: {}", stats.busy_agents);
    println!("  • Active Calls: {}", stats.active_calls);
    println!("  • Queued Calls: {}", stats.queued_calls);
    println!("  • Total Handled: {}", stats.total_calls_handled);
    println!("  • Avg Routing Time: {}ms", stats.routing_stats.average_routing_time_ms);
    println!();

    // 3.2: Agent monitoring
    println!("👤 Agent Status Monitor:");
    let agents = supervisor_api.list_agents().await;
    for agent in &agents {
        println!("  {} ({}):", agent.agent_id, agent.skills.join(", "));
        println!("    Status: {:?}", agent.status);
        println!("    Current Calls: {}/{}", agent.current_calls, agent.max_calls);
        println!("    Performance: {:.1}%", agent.performance_score * 100.0);
    }
    println!();

    // 3.3: Queue monitoring
    println!("📋 Queue Status:");
    let queue_stats = supervisor_api.get_all_queue_stats().await?;
    for (queue_id, stats) in &queue_stats {
        if stats.total_calls > 0 {
            println!("  Queue '{}': {} calls (avg wait: {}s)", 
                     queue_id, stats.total_calls, stats.average_wait_time_seconds);
        }
    }
    if queue_stats.is_empty() || queue_stats.iter().all(|(_, s)| s.total_calls == 0) {
        println!("  No calls in queues");
    }
    println!();

    // Step 4: Simulate some activity
    println!("🎬 Simulating call center activity...\n");
    
    // Bob takes a call
    agent_client.update_agent_status(
        &AgentId("bob-001".to_string()),
        AgentStatus::Busy(vec![])
    ).await?;
    println!("  📞 Bob is now on a call");
    
    // Alice goes offline (instead of break)
    agent_client.update_agent_status(
        &AgentId("alice-001".to_string()),
        AgentStatus::Offline
    ).await?;
    println!("  🚪 Alice is now offline");
    
    // Check updated stats
    sleep(Duration::from_millis(100)).await;
    let updated_stats = supervisor_api.get_stats().await;
    println!("\n  📊 Updated Stats:");
    println!("    Available: {} (was {})", updated_stats.available_agents, stats.available_agents);
    println!("    Busy: {} (was {})", updated_stats.busy_agents, stats.busy_agents);
    println!();

    // Step 5: Performance monitoring
    println!("📊 Performance Metrics (Last Hour):");
    let end_time = Utc::now();
    let start_time = end_time - chrono::Duration::hours(1);
    
    let metrics = supervisor_api.get_performance_metrics(start_time, end_time).await;
    println!("  • Total Calls: {}", metrics.total_calls);
    println!("  • Calls Answered: {}", metrics.calls_answered);
    println!("  • Calls Queued: {}", metrics.calls_queued);
    println!("  • Calls Abandoned: {}", metrics.calls_abandoned);
    println!("  • Avg Wait Time: {}ms", metrics.average_wait_time_ms);
    println!("  • Avg Handle Time: {}s", metrics.average_handle_time_ms / 1000);
    println!("  • Service Level: {:.1}%", metrics.service_level_percentage);
    println!();

    // Step 6: Demonstrate supervisor interventions
    println!("🎯 Supervisor Interventions:\n");
    
    // 6.1: View specific agent's calls
    let bob_id = AgentId("bob-001".to_string());
    let bob_calls = supervisor_api.monitor_agent_calls(&bob_id).await;
    println!("  📞 Bob's Active Calls: {}", bob_calls.len());
    
    // 6.2: List active bridges (if any)
    let bridges = supervisor_api.list_active_bridges().await;
    println!("  🌉 Active Bridges: {}", bridges.len());
    
    // 6.3: Demonstrate manual call assignment (would be used if a call was queued)
    println!("  🎯 Manual assignment available via force_assign_call()");
    
    // 6.4: Call monitoring placeholder
    println!("  👂 Call listening available via listen_to_call()");
    
    // 6.5: Agent coaching placeholder
    println!("  💬 Agent coaching available via coach_agent()");
    println!();

    // Step 7: Continuous monitoring simulation
    println!("🔄 Real-Time Monitoring (5-second updates):\n");
    
    let mut monitor_interval = interval(Duration::from_secs(5));
    let mut iterations = 0;
    
    loop {
        monitor_interval.tick().await;
        iterations += 1;
        
        let current_stats = supervisor_api.get_stats().await;
        let timestamp = Utc::now().format("%H:%M:%S");
        
        println!("[{}] Agents: {} avail, {} busy | Calls: {} active, {} queued",
                 timestamp,
                 current_stats.available_agents,
                 current_stats.busy_agents,
                 current_stats.active_calls,
                 current_stats.queued_calls);
        
        // Simulate some changes
        if iterations == 2 {
            // Bob finishes his call
            agent_client.update_agent_status(
                &AgentId("bob-001".to_string()),
                AgentStatus::Available
            ).await?;
            println!("         ✅ Bob finished his call and is available");
        }
        
        if iterations == 3 {
            // Alice returns from being offline
            agent_client.update_agent_status(
                &AgentId("alice-001".to_string()),
                AgentStatus::Available
            ).await?;
            println!("         ✅ Alice is back online and available");
        }
        
        if iterations >= 4 {
            break;
        }
    }
    
    println!("\n✅ Supervisor Monitoring Demo Complete!");
    println!("\n📋 Key SupervisorApi Features Demonstrated:");
    println!("  • Real-time statistics and agent monitoring");
    println!("  • Queue management and performance metrics");
    println!("  • Agent call monitoring and interventions");
    println!("  • Continuous dashboard updates");
    println!("  • Manual call routing capabilities");

    Ok(())
} 