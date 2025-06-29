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
    benchmark_stats_collection
);

criterion_main!(benches); 