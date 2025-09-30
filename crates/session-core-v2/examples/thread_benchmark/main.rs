//! Thread benchmark: 1 answering peer + 5 calling peers
//! This tests how many threads are used when handling multiple concurrent sessions

use rvoip_session_core_v2::api::simple::{SimplePeer, Config};
use tokio::time::{sleep, Duration};
use tokio::sync::Mutex;
use std::sync::Arc;
use tracing::{info, warn};

/// The answering peer that accepts all incoming calls
async fn run_answering_peer() -> Result<(), Box<dyn std::error::Error>> {
    info!("[ANSWERER] Starting on port 6000...");

    let config = Config {
        sip_port: 6000,
        media_port_start: 20000,
        media_port_end: 20100,
        local_ip: "127.0.0.1".parse()?,
        bind_addr: "127.0.0.1:6000".parse()?,
        state_table_path: None,
        local_uri: "sip:answerer@127.0.0.1:6000".to_string(),
    };

    let answerer = Arc::new(Mutex::new(SimplePeer::with_config("answerer", config).await?));
    info!("[ANSWERER] Ready to accept calls");

    // Accept calls for 30 seconds
    let mut active_calls = Vec::new();
    let start = std::time::Instant::now();

    while start.elapsed() < Duration::from_secs(30) {
        // Check for incoming calls (non-blocking)
        let incoming_call = {
            let mut answerer_guard = answerer.lock().await;
            answerer_guard.incoming_call().await
        };

        if let Some(call) = incoming_call {
            info!("[ANSWERER] Incoming call from {} with ID {}", call.from, call.id);

            // Accept the call
            {
                let answerer_guard = answerer.lock().await;
                answerer_guard.accept(&call.id).await?;
            }
            info!("[ANSWERER] Accepted call {}", call.id);
            active_calls.push(call.id);
        }

        // Keep the loop responsive
        sleep(Duration::from_millis(100)).await;
    }

    info!("[ANSWERER] Handled {} calls", active_calls.len());

    // Hang up all active calls
    for call_id in active_calls {
        let answerer_guard = answerer.lock().await;
        if let Err(e) = answerer_guard.hangup(&call_id).await {
            warn!("[ANSWERER] Error hanging up call {}: {}", call_id, e);
        }
    }

    info!("[ANSWERER] Shutting down");
    Ok(())
}

/// A calling peer that makes a call to the answerer
async fn run_calling_peer(id: usize, answerer_uri: &str) -> Result<(), Box<dyn std::error::Error>> {
    let port = 6001 + id as u16;
    info!("[CALLER-{}] Starting on port {}...", id, port);

    let config = Config {
        sip_port: port,
        media_port_start: 21000 + (id * 100) as u16,
        media_port_end: 21100 + (id * 100) as u16,
        local_ip: "127.0.0.1".parse()?,
        bind_addr: format!("127.0.0.1:{}", port).parse()?,
        state_table_path: None,
        local_uri: format!("sip:caller{}@127.0.0.1:{}", id, port),
    };

    let caller = SimplePeer::with_config(&format!("caller{}", id), config).await?;

    // Wait a bit for answerer to be ready
    sleep(Duration::from_secs(2)).await;

    // Make the call
    info!("[CALLER-{}] Calling answerer...", id);
    let call_id = caller.call(answerer_uri).await?;
    info!("[CALLER-{}] Made call with ID: {}", id, call_id);

    // Keep the call active for 20 seconds
    sleep(Duration::from_secs(20)).await;

    // Hang up
    info!("[CALLER-{}] Hanging up...", id);
    caller.hangup(&call_id).await?;

    info!("[CALLER-{}] Done", id);
    Ok(())
}

/// Print thread and runtime metrics
async fn monitor_metrics() {
    let handle = tokio::runtime::Handle::current();
    let mut interval = tokio::time::interval(Duration::from_secs(5));

    for i in 0..7 {
        interval.tick().await;
        let metrics = handle.metrics();

        println!("\n=== Runtime Metrics (t={}s) ===", i * 5);
        println!("Tokio worker threads: {}", metrics.num_workers());
        println!("Global queue depth: {}", metrics.global_queue_depth());

        // Show per-worker metrics
        for worker in 0..metrics.num_workers() {
            let park_count = metrics.worker_park_count(worker);
            println!("  Worker {}: {} parks", worker, park_count);
        }

        // System thread count
        #[cfg(target_os = "macos")]
        {
            if let Ok(output) = std::process::Command::new("ps")
                .args(&["-M", "-p", &std::process::id().to_string()])
                .output()
            {
                let thread_count = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .count()
                    .saturating_sub(1); // Subtract header line
                println!("System thread count: {}", thread_count);
            }
        }

        // Memory usage
        #[cfg(target_os = "macos")]
        {
            if let Ok(output) = std::process::Command::new("ps")
                .args(&["-o", "rss=,vsz=", "-p", &std::process::id().to_string()])
                .output()
            {
                if let Ok(stats) = String::from_utf8(output.stdout) {
                    let parts: Vec<&str> = stats.trim().split_whitespace().collect();
                    if parts.len() >= 2 {
                        println!("Memory - RSS: {} KB, VSZ: {} KB", parts[0], parts[1]);
                    }
                }
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("rvoip_session_core_v2=info".parse()?)
                .add_directive("rvoip_dialog_core=info".parse()?)
                .add_directive("rvoip_media_core=info".parse()?)
        )
        .init();

    println!("=== Thread Benchmark: 1 Answerer + 5 Callers ===");
    println!("This will create 6 concurrent SIP sessions");
    println!();

    // Print initial metrics
    let handle = tokio::runtime::Handle::current();
    let metrics = handle.metrics();
    println!("Initial Tokio workers: {}", metrics.num_workers());
    println!();

    // Start the monitoring task
    let monitor_handle = tokio::spawn(monitor_metrics());

    // Start the answering peer
    let answerer_handle = tokio::spawn(async {
        if let Err(e) = run_answering_peer().await {
            eprintln!("[ANSWERER] Error: {}", e);
        }
    });

    // Give answerer time to start
    sleep(Duration::from_secs(1)).await;

    // Start 5 calling peers concurrently
    let mut caller_handles = Vec::new();
    for i in 0..5 {
        let handle = tokio::spawn(async move {
            if let Err(e) = run_calling_peer(i, "sip:answerer@127.0.0.1:6000").await {
                eprintln!("[CALLER-{}] Error: {}", i, e);
            }
        });
        caller_handles.push(handle);

        // Stagger the calls slightly to avoid race conditions
        sleep(Duration::from_millis(500)).await;
    }

    // Wait for all callers to complete
    for handle in caller_handles {
        handle.await?;
    }

    // Wait for answerer to complete
    answerer_handle.await?;

    // Stop monitoring
    monitor_handle.abort();

    println!("\n=== Benchmark Complete ===");
    println!("Final metrics:");
    let final_metrics = handle.metrics();
    println!("Tokio workers: {}", final_metrics.num_workers());
    println!("Global queue depth: {}", final_metrics.global_queue_depth());

    Ok(())
}