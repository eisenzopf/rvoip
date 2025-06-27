//! Performance benchmarks for the call center engine
//!
//! These benchmarks measure performance of critical call center operations.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rvoip_call_engine::prelude::*;
use tokio::runtime::Runtime;

/// Benchmark call center initialization
fn benchmark_call_center_init(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    c.bench_function("call_center_initialization", |b| {
        b.to_async(&rt).iter(|| async {
            let database = CallCenterDatabase::new_in_memory().await.unwrap();
            let config = CallCenterConfig::default();
            
            let call_center = CallCenterEngine::new(config, database).await.unwrap();
            black_box(call_center);
        });
    });
}

/// Benchmark agent registry operations
fn benchmark_agent_registry(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    c.bench_function("agent_registry_operations", |b| {
        b.to_async(&rt).iter(|| async {
            let database = CallCenterDatabase::new_in_memory().await.unwrap();
            let mut registry = AgentRegistry::new(database);
            
            // Benchmark agent registration
            let agent = Agent {
                id: "agent_001".to_string(),
                sip_uri: "sip:agent001@call-center.local".parse().unwrap(),
                display_name: "Test Agent".to_string(),
                skills: vec!["general".to_string()],
                max_concurrent_calls: 3,
                status: AgentStatus::Available,
                department: Some("sales".to_string()),
                extension: Some("1001".to_string()),
            };
            
            let result = registry.register_agent(agent).await.unwrap();
            black_box(result);
        });
    });
}

/// Benchmark queue operations
fn benchmark_queue_operations(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    c.bench_function("queue_operations", |b| {
        b.to_async(&rt).iter(|| async {
            let mut queue_manager = QueueManager::new();
            
            // Create queue
            queue_manager.create_queue(
                "test_queue".to_string(),
                "Test Queue".to_string(),
                100
            ).unwrap();
            
            // Benchmark enqueue operation
            let queued_call = QueuedCall {
                session_id: "session_001".to_string(),
                caller_id: "1234567890".to_string(),
                priority: 5,
                queued_at: chrono::Utc::now(),
                estimated_wait_time: Some(60),
            };
            
            let result = queue_manager.enqueue_call("test_queue", queued_call).unwrap();
            black_box(result);
        });
    });
}

// TODO: Add more benchmarks as modules are implemented:
// - Benchmark call routing decisions
// - Benchmark bridge creation and management
// - Benchmark database operations
// - Benchmark concurrent call handling
// - Benchmark metrics collection

criterion_group!(
    benches,
    benchmark_call_center_init,
    benchmark_agent_registry,
    benchmark_queue_operations
);

criterion_main!(benches); 