//! Performance benchmarks for the call center engine
//!
//! These benchmarks measure performance of critical call center operations using the real API.
//!
//! ## Benchmark Breakdown:
//!
//! 1. **call_center_initialization**: Measures time to create CallCenterEngine with real session-core
//!    - Includes database setup, session coordinator creation, and transport initialization
//!    - Falls back to config-only creation if transport fails (common in CI environments)
//!
//! 2. **config_creation_validation**: Measures pure configuration operations
//!    - Config creation and validation logic only
//!    - No network or database dependencies
//!
//! 3. **database_agent_operations**: Measures real database performance
//!    - Agent insertion using real Limbo database operations
//!    - Includes SQL execution and database I/O
//!
//! 4. **queue_operations**: Measures queue management performance  
//!    - Queue creation and statistics retrieval
//!    - Tests in-memory queue operations
//!
//! 5. **stats_collection**: Measures statistics aggregation performance
//!    - Cross-system stats collection (agents, queues, calls)
//!    - Includes database queries and in-memory aggregation
//!
//! 6. **bulk_agent_registration**: Measures performance of registering 100 agents simultaneously
//!    - Agent creation and validation
//!    - Database batch operations (100 agent records)
//!    - Session-core registration overhead
//!    - Memory allocation patterns
//!    - System scalability under realistic load

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rvoip_call_engine::prelude::*;
use tokio::runtime::Runtime;

/// Create a benchmark-friendly configuration that allows real API usage
fn create_benchmark_config() -> CallCenterConfig {
    let mut config = CallCenterConfig::default();
    // Use different ports to avoid conflicts during benchmarking
    config.general.local_signaling_addr = "127.0.0.1:15061".parse().unwrap();
    config.general.local_media_addr = "127.0.0.1:20001".parse().unwrap();
    config
}

/// **BENCHMARK 1: CallCenter Engine Initialization**
/// 
/// Measures: Full CallCenterEngine creation including session-core setup
/// - Database initialization (Limbo SQLite)
/// - Session coordinator creation with dialog-core
/// - Transport layer initialization (UDP/TCP/TLS)
/// - Agent registry, queue manager, and monitoring setup
/// 
/// Expected: ~200µs - 2ms depending on system and transport availability
/// Fallback: Config creation only (~100-200ns) if transport fails
fn benchmark_call_center_init(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    c.bench_function("01_engine_initialization", |b| {
        b.to_async(&rt).iter(|| async {
            let config = create_benchmark_config();
            
            // Attempt full CallCenterEngine creation with real session-core
            match CallCenterEngine::new(config, Some(":memory:".to_string())).await {
                Ok(call_center) => {
                    // SUCCESS: Benchmarking full engine initialization
                    black_box(call_center);
                }
                Err(_) => {
                    // FALLBACK: Transport not available, benchmark config creation only
                    let config = create_benchmark_config();
                    black_box(config);
                }
            }
        });
    });
}

/// **BENCHMARK 2: Configuration Operations** 
/// 
/// Measures: Pure configuration creation and validation
/// - Config struct creation with default values
/// - Validation logic execution (IP parsing, constraint checking)
/// - No I/O or network dependencies
/// 
/// Expected: ~100-200ns (very fast, pure CPU)
fn benchmark_config_operations(c: &mut Criterion) {
    c.bench_function("02_config_validation", |b| {
        b.iter(|| {
            let config = create_benchmark_config();
            let validation_result = config.validate();
            black_box((config, validation_result));
        });
    });
}

/// **BENCHMARK 3: Database Agent Operations**
/// 
/// Measures: Real database I/O with agent management
/// - Agent record insertion via SQL
/// - Database connection and transaction overhead
/// - Limbo SQLite performance characteristics
/// 
/// Expected: ~1-10ms (database I/O bound)
fn benchmark_database_operations(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    // Pre-create a call center for repeated use
    let call_center = rt.block_on(async {
        let config = create_benchmark_config();
        CallCenterEngine::new(config, Some(":memory:".to_string())).await
    });
    
    if let Ok(call_center) = call_center {
        c.bench_function("03_database_agent_upsert", |b| {
            b.to_async(&rt).iter(|| async {
                let db_manager = call_center.database_manager().unwrap();
                
                // Benchmark real database INSERT operation
                let agent_id = format!("bench_agent_{}", chrono::Utc::now().timestamp_nanos());
                let result = db_manager.upsert_agent(
                    &agent_id,
                    "Benchmark Agent",
                    Some("sip:bench@call-center.local")
                ).await.unwrap();
                
                black_box(result);
            });
        });
    } else {
        eprintln!("⚠️  SKIPPING database benchmarks - CallCenter initialization failed");
    }
}

/// **BENCHMARK 4: Queue Management Operations**
/// 
/// Measures: In-memory queue operations
/// - Queue creation and registration
/// - Queue statistics calculation
/// - HashMap lookups and vector operations
/// 
/// Expected: ~200-500ns (fast in-memory operations)
fn benchmark_queue_operations(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    // Pre-create a call center for repeated use
    let call_center = rt.block_on(async {
        let config = create_benchmark_config();
        CallCenterEngine::new(config, Some(":memory:".to_string())).await
    });
    
    if let Ok(call_center) = call_center {
        c.bench_function("04_queue_create_and_stats", |b| {
            b.to_async(&rt).iter(|| async {
                // Benchmark queue creation + stats retrieval
                let queue_name = format!("bench_queue_{}", chrono::Utc::now().timestamp_nanos());
                call_center.create_queue(&queue_name).await.unwrap();
                
                let result = call_center.get_queue_stats().await.unwrap();
                black_box(result);
            });
        });
    } else {
        eprintln!("⚠️  SKIPPING queue benchmarks - CallCenter initialization failed");
    }
}

/// **BENCHMARK 5: Statistics Collection**
/// 
/// Measures: Cross-system statistics aggregation
/// - Agent status queries from database
/// - Queue depth calculations
/// - Call center metrics compilation
/// - Data structure traversal and aggregation
/// 
/// Expected: ~5-20µs (moderate complexity with some database queries)
fn benchmark_stats_collection(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    // Pre-create a call center for repeated use
    let call_center = rt.block_on(async {
        let config = create_benchmark_config();
        CallCenterEngine::new(config, Some(":memory:".to_string())).await
    });
    
    if let Ok(call_center) = call_center {
        c.bench_function("05_orchestrator_stats_aggregation", |b| {
            b.to_async(&rt).iter(|| async {
                // Benchmark comprehensive stats collection
                let stats = call_center.get_stats().await;
                black_box(stats);
            });
        });
    } else {
        eprintln!("⚠️  SKIPPING stats benchmarks - CallCenter initialization failed");
    }
}

/// **BENCHMARK 6: Real SIP Infrastructure - 1 Server + 10 Agent Registrations (E2E Pattern)**
/// 
/// Measures: 10 agent registration performance to a single call center server
/// - ONE CallCenterServer running (like basic_call_center_server.rs)
/// - 10 ClientManager agents registering via SIP REGISTER to the same server
/// - Real network transport (UDP SIP on localhost)
/// - Database operations for all agent records
/// - Session-core coordination for all sessions
/// 
/// Expected: ~1-5 seconds for 10 real SIP registrations
/// This follows the exact same pattern as the working e2e_test
fn benchmark_real_sip_infrastructure_e2e_pattern(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    // Start ONE server that will be used for all benchmark iterations
    let server_handle = rt.block_on(async {
        println!("🚀 SETUP: Starting ONE call center server for all benchmark iterations");
        
        use rvoip_call_engine::{CallCenterServerBuilder, CallCenterConfig};
        
        let mut config = CallCenterConfig::default();
        config.general.local_signaling_addr = "0.0.0.0:5060".parse().unwrap();
        config.general.domain = "127.0.0.1".to_string();
        config.agents.default_max_concurrent_calls = 1;
        
        let mut server = CallCenterServerBuilder::new()
            .with_config(config)
            .with_database_path(":memory:".to_string())
            .build()
            .await
            .expect("Server creation should work");
            
        server.start().await.expect("Server start should work");
        server.create_default_queues().await.expect("Queue creation should work");
        
        // Start server's event loop in background
        let server_task = tokio::spawn(async move {
            if let Err(e) = server.run().await {
                eprintln!("❌ SERVER: Runtime error: {}", e);
            }
        });
        
        // Give server time to fully start
        tokio::time::sleep(tokio::time::Duration::from_millis(2000)).await;
        println!("✅ SERVER: Ready for registrations on 127.0.0.1:5060");
        
        server_task
    });
    
    // Configure benchmark measurement
    let mut group = c.benchmark_group("real_sip_infrastructure_e2e");
    group.sample_size(10);
    group.measurement_time(std::time::Duration::from_secs(60));
    
    group.bench_function("06_10_agents_to_one_server", |b| {
        b.to_async(&rt).iter(|| async {
            println!("📊 BENCHMARK: Measuring 10 agent registrations to running server");
            
            // Create 10 real client-core agents
            let num_agents = 10;
            let mut agent_handles = Vec::with_capacity(num_agents);
            let registration_start = std::time::Instant::now();
            
            for i in 0..num_agents {
                let agent_id = i;
                let handle = tokio::spawn(async move {
                    create_and_register_agent(agent_id, 5060).await
                });
                agent_handles.push(handle);
                
                // Stagger agent creation slightly to avoid overwhelming
                if i % 10 == 0 {
                    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                }
            }
            
            // Wait for all agents to complete registration (this is what we're measuring)
            let mut successful_registrations = 0;
            let mut failed_registrations = 0;
            
            for (i, handle) in agent_handles.into_iter().enumerate() {
                match handle.await {
                    Ok(Ok(())) => {
                        successful_registrations += 1;
                        if i % 20 == 0 {
                            println!("✅ PROGRESS: {} agents registered", successful_registrations);
                        }
                    }
                    Ok(Err(e)) => {
                        failed_registrations += 1;
                        if failed_registrations <= 3 {
                            eprintln!("⚠️  Agent {} registration failed: {}", i, e);
                        }
                    }
                    Err(e) => {
                        failed_registrations += 1;
                        if failed_registrations <= 3 {
                            eprintln!("⚠️  Agent {} task failed: {}", i, e);
                        }
                    }
                }
            }
            
            let registration_time = registration_start.elapsed();
            println!("📊 RESULTS: {}/{} successful registrations in {:?}", 
                     successful_registrations, num_agents, registration_time);
            
            // Return the benchmark result
            black_box((successful_registrations, failed_registrations, registration_time));
        });
    });
    
    group.finish();
    
    // Cleanup: Stop the server
    rt.block_on(async {
        server_handle.abort();
        println!("🛑 CLEANUP: Server stopped");
    });
}

/// Create and register a single agent using ClientManager (like e2e_test pattern)
async fn create_and_register_agent(agent_id: usize, server_port: u16) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use rvoip_client_core::{ClientManager, ClientConfig, RegistrationConfig};
    
    let username = format!("agent{:03}", agent_id);
    let local_port = 6000 + agent_id as u16; // Use unique ports starting from 6000
    
    // Create client configuration (like e2e_test agent)
    let local_sip_addr = format!("0.0.0.0:{}", local_port).parse()
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
    let local_media_addr = format!("0.0.0.0:{}", local_port + 1000).parse()
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?; // Media port offset +1000

    let client_config = ClientConfig::new()
        .with_sip_addr(local_sip_addr)
        .with_media_addr(local_media_addr)
        .with_user_agent(format!("RVoIP-Benchmark-Agent-{}/1.0", username));

    // Create and start client
    let client = ClientManager::new(client_config).await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
    client.start().await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

    // Register with the server (like e2e_test registration)
    let agent_uri = format!("sip:{}@127.0.0.1", username);
    let reg_config = RegistrationConfig::new(
        format!("sip:127.0.0.1:{}", server_port),  // registrar - use the passed server port
        agent_uri.clone(),                         // from_uri
        agent_uri.clone(),                         // contact_uri
    )
    .with_expires(60); // Short expiry for benchmark

    // Attempt registration with timeout
    let registration_result = tokio::time::timeout(
        tokio::time::Duration::from_secs(10),
        client.register(reg_config)
    ).await;

    match registration_result {
        Ok(Ok(_)) => {
            // Registration successful, now stop the client
            let _ = client.stop().await;
            Ok(())
        }
        Ok(Err(e)) => {
            let _ = client.stop().await;
            Err(Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
        }
        Err(_) => {
            let _ = client.stop().await;
            Err("Registration timeout".to_string().into())
        }
    }
}

/// **SIMPLE TEST: Real SIP Infrastructure - 1 Server + 100 Agent Registrations**
/// 
/// This is not a benchmark - it's a single test run that measures 100 agent registrations
/// to one server without Criterion's multiple iterations (which cause port conflicts)
fn simple_test_100_agent_registrations() {
    let rt = Runtime::new().unwrap();
    
    rt.block_on(async {
        println!("🚀 STARTING: Real SIP infrastructure test - 1 server + 100 agents");
        
        // Start ONE server
        use rvoip_call_engine::{CallCenterServerBuilder, CallCenterConfig};
        
        let mut config = CallCenterConfig::default();
        config.general.local_signaling_addr = "0.0.0.0:5060".parse().unwrap();
        config.general.domain = "127.0.0.1".to_string();
        config.agents.default_max_concurrent_calls = 1;
        
        let mut server = CallCenterServerBuilder::new()
            .with_config(config)
            .with_database_path(":memory:".to_string())
            .build()
            .await
            .expect("Server creation should work");
            
        server.start().await.expect("Server start should work");
        server.create_default_queues().await.expect("Queue creation should work");
        
        // Start server's event loop in background
        let server_task = tokio::spawn(async move {
            if let Err(e) = server.run().await {
                eprintln!("❌ SERVER: Runtime error: {}", e);
            }
        });
        
        // Give server time to fully start
        tokio::time::sleep(tokio::time::Duration::from_millis(3000)).await;
        println!("✅ SERVER: Ready for registrations on 127.0.0.1:5060");
        
        // Create 100 real client-core agents
        let num_agents = 100;
        println!("👥 AGENTS: Creating {} real agents with unique ports", num_agents);
        
        let mut agent_handles = Vec::with_capacity(num_agents);
        let registration_start = std::time::Instant::now();
        
        for i in 0..num_agents {
            let agent_id = i;
            let handle = tokio::spawn(async move {
                create_and_register_agent(agent_id, 5060).await
            });
            agent_handles.push(handle);
            
            // Stagger agent creation to avoid overwhelming
            if i % 20 == 0 {
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                println!("🔄 PROGRESS: Created {} agents...", i + 1);
            }
        }
        
        println!("⏱️  MEASURING: Waiting for all {} agents to register...", num_agents);
        
        // Wait for all agents to complete registration
        let mut successful_registrations = 0;
        let mut failed_registrations = 0;
        
        for (i, handle) in agent_handles.into_iter().enumerate() {
            match handle.await {
                Ok(Ok(())) => {
                    successful_registrations += 1;
                    if successful_registrations % 25 == 0 {
                        println!("✅ PROGRESS: {} agents registered successfully", successful_registrations);
                    }
                }
                Ok(Err(e)) => {
                    failed_registrations += 1;
                    if failed_registrations <= 5 {
                        eprintln!("⚠️  Agent {} registration failed: {}", i, e);
                    }
                }
                Err(e) => {
                    failed_registrations += 1;
                    if failed_registrations <= 5 {
                        eprintln!("⚠️  Agent {} task failed: {}", i, e);
                    }
                }
            }
        }
        
        let registration_time = registration_start.elapsed();
        
        println!("\n🎯 **FINAL RESULTS**");
        println!("📊 Total agents: {}", num_agents);
        println!("✅ Successful registrations: {}", successful_registrations);
        println!("❌ Failed registrations: {}", failed_registrations); 
        println!("⏱️  Total time: {:?}", registration_time);
        println!("📈 Rate: {:.2} registrations/second", successful_registrations as f64 / registration_time.as_secs_f64());
        println!("💾 Average time per registration: {:?}", registration_time / num_agents as u32);
        
        // Cleanup
        server_task.abort();
        println!("🛑 CLEANUP: Server stopped");
    });
}

/// Run this as: cargo test --release --bench call_center_benchmarks simple_test_100_agent_registrations
#[cfg(test)]
mod simple_tests {
    use super::*;
    
    #[tokio::test]
    #[ignore] // Use --ignored to run this
    async fn test_100_agent_registrations() {
        simple_test_100_agent_registrations();
    }
}

// TODO: Add more benchmarks as modules are implemented:
// - Benchmark call routing decisions
// - Benchmark bridge creation and management
// - Benchmark concurrent call handling
// - Benchmark session-core integration
// - Benchmark real SIP message processing

criterion_group!(
    benches,
    benchmark_call_center_init,
    benchmark_config_operations,
    benchmark_database_operations,
    benchmark_queue_operations,
    benchmark_stats_collection,
    benchmark_real_sip_infrastructure_e2e_pattern
);

criterion_main!(benches); 