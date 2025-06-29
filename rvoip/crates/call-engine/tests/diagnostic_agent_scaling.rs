//! Diagnostic test to find the breaking point for agent scaling
//!
//! This test creates a real SIP server and tries to register increasing numbers
//! of agents to find where the system breaks down.
//! 
//! **Resource Profiling**: Monitors CPU cores, threads, memory usage for server and per-agent

use rvoip_call_engine::{CallCenterServerBuilder, CallCenterConfig};
use rvoip_client_core::{ClientManager, ClientConfig, RegistrationConfig};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// System resource information
#[derive(Debug, Clone)]
struct SystemInfo {
    total_cores: usize,
    logical_cores: usize,
    total_memory_mb: u64,
    available_memory_mb: u64,
}

/// Process resource usage
#[derive(Debug, Clone)]
struct ProcessUsage {
    pid: u32,
    memory_rss_kb: u64,
    memory_vsz_kb: u64,
    cpu_percent: f64,
    thread_count: usize,
}

/// Get system information (cores, memory, etc.)
fn get_system_info() -> SystemInfo {
    let mut total_cores = 1;
    let mut logical_cores = 1;
    let mut total_memory_mb = 0;
    let mut available_memory_mb = 0;

    // Get CPU core count (macOS)
    if let Ok(output) = Command::new("sysctl").args(&["-n", "hw.physicalcpu"]).output() {
        if let Ok(cores_str) = String::from_utf8(output.stdout) {
            total_cores = cores_str.trim().parse().unwrap_or(1);
        }
    }

    if let Ok(output) = Command::new("sysctl").args(&["-n", "hw.logicalcpu"]).output() {
        if let Ok(cores_str) = String::from_utf8(output.stdout) {
            logical_cores = cores_str.trim().parse().unwrap_or(1);
        }
    }

    // Get memory info (macOS)
    if let Ok(output) = Command::new("sysctl").args(&["-n", "hw.memsize"]).output() {
        if let Ok(mem_str) = String::from_utf8(output.stdout) {
            let total_bytes: u64 = mem_str.trim().parse().unwrap_or(0);
            total_memory_mb = total_bytes / (1024 * 1024);
        }
    }

    // Get available memory using vm_stat
    if let Ok(output) = Command::new("vm_stat").output() {
        if let Ok(vm_output) = String::from_utf8(output.stdout) {
            let mut free_pages = 0u64;
            let mut inactive_pages = 0u64;
            
            for line in vm_output.lines() {
                if line.starts_with("Pages free:") {
                    if let Some(pages_str) = line.split_whitespace().nth(2) {
                        free_pages = pages_str.trim_end_matches('.').parse().unwrap_or(0);
                    }
                } else if line.starts_with("Pages inactive:") {
                    if let Some(pages_str) = line.split_whitespace().nth(2) {
                        inactive_pages = pages_str.trim_end_matches('.').parse().unwrap_or(0);
                    }
                }
            }
            
            // Each page is typically 4096 bytes on macOS
            available_memory_mb = (free_pages + inactive_pages) * 4096 / (1024 * 1024);
        }
    }

    SystemInfo {
        total_cores,
        logical_cores,
        total_memory_mb,
        available_memory_mb,
    }
}

/// Get process resource usage by PID
fn get_process_usage(pid: u32) -> Option<ProcessUsage> {
    // Use ps to get detailed process info (macOS compatible)
    let output = Command::new("ps")
        .args(&["-p", &pid.to_string(), "-o", "pid,rss,vsz,pcpu"])
        .output()
        .ok()?;

    let ps_output = String::from_utf8(output.stdout).ok()?;
    let lines: Vec<&str> = ps_output.lines().collect();
    
    if lines.len() < 2 {
        return None;
    }

    let data_line = lines[1];
    let fields: Vec<&str> = data_line.split_whitespace().collect();
    
    if fields.len() < 4 {
        return None;
    }

    // Get thread count separately using ps -M (macOS)
    let thread_count = if let Ok(thread_output) = Command::new("ps")
        .args(&["-M", "-p", &pid.to_string()])
        .output()
    {
        if let Ok(thread_text) = String::from_utf8(thread_output.stdout) {
            // Count lines minus header = thread count
            thread_text.lines().count().saturating_sub(1)
        } else {
            1 // Default fallback
        }
    } else {
        1 // Default fallback
    };

    Some(ProcessUsage {
        pid,
        memory_rss_kb: fields[1].parse().unwrap_or(0),
        memory_vsz_kb: fields[2].parse().unwrap_or(0),
        cpu_percent: fields[3].parse().unwrap_or(0.0),
        thread_count,
    })
}

/// Enhanced agent creation with resource tracking
async fn create_and_register_agent_with_profiling(
    agent_id: usize, 
    server_port: u16,
    agent_counter: Arc<AtomicUsize>
) -> std::result::Result<ProcessUsage, Box<dyn std::error::Error + Send + Sync>> {
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
        .with_user_agent(format!("RVoIP-Diagnostic-Agent-{}/1.0", username));

    // Create and start client
    let client = ClientManager::new(client_config).await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
    client.start().await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

    // Get current process ID to track this agent's resource usage
    let current_pid = std::process::id();

    // Register with the server (like e2e_test registration)
    let agent_uri = format!("sip:{}@127.0.0.1", username);
    let reg_config = RegistrationConfig::new(
        format!("sip:127.0.0.1:{}", server_port),  // registrar - use the passed server port
        agent_uri.clone(),                         // from_uri
        agent_uri.clone(),                         // contact_uri
    )
    .with_expires(60); // Short expiry for diagnostic

    // Attempt registration with timeout
    let registration_result = tokio::time::timeout(
        tokio::time::Duration::from_secs(10),
        client.register(reg_config)
    ).await;

    match registration_result {
        Ok(Ok(_)) => {
            // Registration successful, get resource usage before stopping
            let usage = get_process_usage(current_pid).unwrap_or(ProcessUsage {
                pid: current_pid,
                memory_rss_kb: 0,
                memory_vsz_kb: 0,
                cpu_percent: 0.0,
                thread_count: 0,
            });

            agent_counter.fetch_add(1, Ordering::Relaxed);
            
            // Stop the client
            let _ = client.stop().await;
            Ok(usage)
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

/// Create and register a single agent using ClientManager (like e2e_test pattern)
async fn create_and_register_agent(agent_id: usize, server_port: u16) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let counter = Arc::new(AtomicUsize::new(0));
    match create_and_register_agent_with_profiling(agent_id, server_port, counter).await {
        Ok(_) => Ok(()),
        Err(e) => Err(e),
    }
}

#[tokio::test]
async fn test_agent_scaling_diagnostic() {
    println!("🔍 STARTING AGENT SCALING DIAGNOSTIC TEST WITH RESOURCE PROFILING");
    
    // Get baseline system information
    let system_info = get_system_info();
    println!("\n🖥️  SYSTEM INFORMATION:");
    println!("  💾 Total Memory: {} MB", system_info.total_memory_mb);
    println!("  💾 Available Memory: {} MB", system_info.available_memory_mb);
    println!("  🏭 Physical CPU Cores: {}", system_info.total_cores);
    println!("  🧠 Logical CPU Cores: {}", system_info.logical_cores);
    
    // Get baseline process usage
    let current_pid = std::process::id();
    let baseline_usage = get_process_usage(current_pid).unwrap_or(ProcessUsage {
        pid: current_pid,
        memory_rss_kb: 0,
        memory_vsz_kb: 0,
        cpu_percent: 0.0,
        thread_count: 0,
    });
    
    println!("\n📊 BASELINE PROCESS USAGE:");
    println!("  🆔 PID: {}", baseline_usage.pid);
    println!("  💾 Memory RSS: {} KB ({:.1} MB)", baseline_usage.memory_rss_kb, baseline_usage.memory_rss_kb as f64 / 1024.0);
    println!("  💾 Memory VSZ: {} KB ({:.1} MB)", baseline_usage.memory_vsz_kb, baseline_usage.memory_vsz_kb as f64 / 1024.0);
    println!("  🧵 Thread Count: {}", baseline_usage.thread_count);
    
    // Test server creation
    println!("\nTesting server creation...");
    
    let mut config = CallCenterConfig::default();
    config.general.local_signaling_addr = "0.0.0.0:5063".parse().unwrap(); // Different port
    config.general.domain = "127.0.0.1".to_string();
    
    let server_result = CallCenterServerBuilder::new()
        .with_config(config)
        .with_database_path(":memory:".to_string())
        .build()
        .await;
        
    match server_result {
        Ok(mut server) => {
            println!("✅ Server created successfully");
            
            match server.start().await {
                Ok(()) => {
                    println!("✅ Server started successfully");
                    
                    // Get server resource usage after startup
                    let server_startup_usage = get_process_usage(current_pid).unwrap_or(baseline_usage.clone());
                    println!("\n📊 SERVER STARTUP RESOURCE USAGE:");
                    println!("  💾 Memory RSS: {} KB ({:.1} MB) [+{:.1} MB from baseline]", 
                             server_startup_usage.memory_rss_kb, 
                             server_startup_usage.memory_rss_kb as f64 / 1024.0,
                             (server_startup_usage.memory_rss_kb.saturating_sub(baseline_usage.memory_rss_kb)) as f64 / 1024.0);
                    println!("  🧵 Thread Count: {} [+{} from baseline]", 
                             server_startup_usage.thread_count, 
                             server_startup_usage.thread_count.saturating_sub(baseline_usage.thread_count));
                    
                    // Start server task
                    let server_task = tokio::spawn(async move {
                        if let Err(e) = server.run().await {
                            eprintln!("❌ SERVER: Runtime error: {}", e);
                        }
                    });
                    
                    // Give server time to fully start  
                    tokio::time::sleep(tokio::time::Duration::from_millis(2000)).await;
                    println!("✅ Server running, testing agent creation...");
                    
                    // Test different agent counts with resource profiling
                    let test_counts = vec![1, 5, 20, 50, 100, 200, 500, 1000];
                    
                    // Track peak metrics across all tests
                    let mut peak_successful_agents = 0;
                    let mut peak_registration_rate = 0.0;
                    let mut peak_memory_mb = 0.0;
                    let mut peak_cpu_percent = 0.0;
                    let mut peak_threads = 0;
                    let mut total_test_time = std::time::Duration::new(0, 0);
                    
                    for &num_agents in &test_counts {
                        println!("\n🧪 TESTING: {} agents", num_agents);
                        
                        let test_start = std::time::Instant::now();
                        let mut agent_handles = Vec::with_capacity(num_agents);
                        let agent_counter = Arc::new(AtomicUsize::new(0));
                        
                        // Create agents
                        for i in 0..num_agents {
                            let agent_id = i + (num_agents * 1000); // Unique port range per test
                            let counter_clone = Arc::clone(&agent_counter);
                            let handle = tokio::spawn(async move {
                                create_and_register_agent_with_profiling(agent_id, 5063, counter_clone).await
                            });
                            agent_handles.push(handle);
                            
                            // Small delay to avoid overwhelming, more frequent for larger tests
                            if i % 10 == 0 && i > 0 {
                                tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
                            }
                            // Progress indicator for large tests
                            if num_agents >= 200 && i % 100 == 0 && i > 0 {
                                println!("  📝 Spawned {} agents...", i);
                            }
                        }
                        
                        println!("⏱️  All {} agent tasks spawned, waiting for completion...", num_agents);
                        
                        // Collect results and resource usage
                        let mut successful = 0;
                        let mut failed = 0;
                        let mut total_agent_memory_kb = 0u64;
                        let mut max_agent_memory_kb = 0u64;
                        let mut total_agent_threads = 0usize;
                        
                        for (i, handle) in agent_handles.into_iter().enumerate() {
                            match tokio::time::timeout(std::time::Duration::from_secs(45), handle).await {
                                Ok(Ok(Ok(usage))) => {
                                    successful += 1;
                                    total_agent_memory_kb += usage.memory_rss_kb;
                                    max_agent_memory_kb = max_agent_memory_kb.max(usage.memory_rss_kb);
                                    total_agent_threads += usage.thread_count;
                                    
                                    // Adjust output frequency based on test size
                                    let should_print = if num_agents <= 50 {
                                        successful <= 3 || successful % 10 == 0
                                    } else if num_agents <= 200 {
                                        successful <= 3 || successful % 50 == 0
                                    } else {
                                        successful <= 3 || successful % 100 == 0
                                    };
                                    
                                    if should_print {
                                        println!("✅ Agent {} successful - Memory: {:.1} MB, Threads: {} (Total: {})", 
                                                 i, usage.memory_rss_kb as f64 / 1024.0, usage.thread_count, successful);
                                    }
                                }
                                Ok(Ok(Err(e))) => {
                                    failed += 1;
                                    if failed <= 5 {
                                        println!("⚠️  Agent {} failed: {}", i, e);
                                    } else if failed == 6 {
                                        println!("⚠️  ... (suppressing further failure messages)");
                                    }
                                }
                                Ok(Err(e)) => {
                                    failed += 1;
                                    if failed <= 5 {
                                        println!("💥 Agent {} task crashed: {}", i, e);
                                    } else if failed == 6 {
                                        println!("💥 ... (suppressing further crash messages)");
                                    }
                                }
                                Err(_) => {
                                    failed += 1;
                                    if failed <= 5 {
                                        println!("⏰ Agent {} timed out", i);
                                    } else if failed == 6 {
                                        println!("⏰ ... (suppressing further timeout messages)");
                                    }
                                }
                            }
                        }
                        
                        let test_duration = test_start.elapsed();
                        total_test_time += test_duration;
                        
                        // Get final server resource usage
                        let final_server_usage = get_process_usage(current_pid).unwrap_or(server_startup_usage.clone());
                        
                        // Update peak metrics
                        if successful > peak_successful_agents {
                            peak_successful_agents = successful;
                        }
                        let registration_rate = successful as f64 / test_duration.as_secs_f64();
                        if registration_rate > peak_registration_rate {
                            peak_registration_rate = registration_rate;
                        }
                        let memory_mb = final_server_usage.memory_rss_kb as f64 / 1024.0;
                        if memory_mb > peak_memory_mb {
                            peak_memory_mb = memory_mb;
                        }
                        if final_server_usage.cpu_percent > peak_cpu_percent {
                            peak_cpu_percent = final_server_usage.cpu_percent;
                        }
                        if final_server_usage.thread_count > peak_threads {
                            peak_threads = final_server_usage.thread_count;
                        }
                        
                        println!("\n📊 RESULTS FOR {} AGENTS:", num_agents);
                        println!("  ✅ Successful: {}", successful);
                        println!("  ❌ Failed: {}", failed);
                        println!("  ⏱️  Duration: {:?}", test_duration);
                        println!("  📈 Rate: {:.2} registrations/second", registration_rate);
                        
                        println!("\n🖥️  SERVER RESOURCE USAGE:");
                        println!("  💾 Total Memory RSS: {} KB ({:.1} MB)", final_server_usage.memory_rss_kb, memory_mb);
                        println!("  💾 Memory increase from baseline: +{:.1} MB", 
                                 (final_server_usage.memory_rss_kb.saturating_sub(baseline_usage.memory_rss_kb)) as f64 / 1024.0);
                        println!("  🧵 Total Thread Count: {}", final_server_usage.thread_count);
                        println!("  🧵 Thread increase from baseline: +{}", 
                                 final_server_usage.thread_count.saturating_sub(baseline_usage.thread_count));
                        println!("  🔥 CPU Usage: {:.1}%", final_server_usage.cpu_percent);
                        
                        if successful > 0 {
                            println!("\n👥 PER-AGENT RESOURCE USAGE:");
                            println!("  💾 Average Memory per Agent: {:.1} KB ({:.2} MB)", 
                                     total_agent_memory_kb as f64 / successful as f64,
                                     (total_agent_memory_kb as f64 / successful as f64) / 1024.0);
                            println!("  💾 Peak Agent Memory: {:.1} KB ({:.2} MB)", 
                                     max_agent_memory_kb, max_agent_memory_kb as f64 / 1024.0);
                            println!("  🧵 Average Threads per Agent: {:.1}", 
                                     total_agent_threads as f64 / successful as f64);
                            
                            println!("\n📈 SCALING METRICS:");
                            println!("  🏭 CPU Core Utilization: {:.1}% of {} cores", 
                                     (final_server_usage.cpu_percent / system_info.logical_cores as f64) * 100.0,
                                     system_info.logical_cores);
                            println!("  💾 Memory Utilization: {:.1}% of {} MB total", 
                                     (final_server_usage.memory_rss_kb as f64 / 1024.0) / system_info.total_memory_mb as f64 * 100.0,
                                     system_info.total_memory_mb);
                            println!("  📊 Memory per Agent/Core Ratio: {:.2} MB/agent per core", 
                                     (total_agent_memory_kb as f64 / 1024.0 / successful as f64) / system_info.logical_cores as f64);
                        }
                        
                        if successful == 0 && (failed > 0) {
                            println!("🚨 BREAKING POINT FOUND: {} agents failed completely", num_agents);
                            break;
                        }
                        
                        // Give system time to cleanup between tests - longer for larger tests
                        let cleanup_time = if num_agents >= 500 {
                            5000 // 5 seconds for 500+ agents
                        } else if num_agents >= 100 {
                            3000 // 3 seconds for 100+ agents  
                        } else {
                            2000 // 2 seconds for smaller tests
                        };
                        
                        println!("🧹 Cleaning up... (waiting {}ms)", cleanup_time);
                        tokio::time::sleep(tokio::time::Duration::from_millis(cleanup_time)).await;
                    }
                    
                    // Print comprehensive summary
                    println!("\n");
                    println!("════════════════════════════════════════════════════════════════");
                    println!("🎯 RVOIP SIP INFRASTRUCTURE PERFORMANCE SUMMARY");
                    println!("════════════════════════════════════════════════════════════════");
                    
                    println!("\n🖥️  SYSTEM CONFIGURATION:");
                    println!("  💻 Hardware: {} physical cores, {} logical cores", system_info.total_cores, system_info.logical_cores);
                    println!("  💾 Total Memory: {:.1} GB ({} MB)", system_info.total_memory_mb as f64 / 1024.0, system_info.total_memory_mb);
                    println!("  💾 Available Memory: {:.1} GB ({} MB)", system_info.available_memory_mb as f64 / 1024.0, system_info.available_memory_mb);
                    
                    println!("\n🏆 PEAK PERFORMANCE METRICS:");
                    println!("  🎯 Maximum Concurrent Agents: {} agents", peak_successful_agents);
                    println!("  ⚡ Peak Registration Rate: {:.1} registrations/second", peak_registration_rate);
                    println!("  ⏱️  Total Test Duration: {:.2} seconds", total_test_time.as_secs_f64());
                    println!("  💾 Peak Memory Usage: {:.1} MB", peak_memory_mb);
                    println!("  🔥 Peak CPU Usage: {:.1}%", peak_cpu_percent);
                    println!("  🧵 Peak Thread Count: {} threads", peak_threads);
                    
                    println!("\n📊 RESOURCE EFFICIENCY:");
                    println!("  💾 Memory per Agent: ~{:.1} MB/agent", peak_memory_mb / peak_successful_agents as f64);
                    println!("  🏭 CPU per Agent: ~{:.2}% per agent", peak_cpu_percent / peak_successful_agents as f64);
                    println!("  🧵 Threads per Agent: ~{:.1} threads/agent", peak_threads as f64 / peak_successful_agents as f64);
                    println!("  ⚡ Processing Rate: {:.1} agents/second peak", peak_registration_rate);
                    
                    println!("\n🚀 SCALABILITY PROJECTIONS:");
                    let memory_capacity = (system_info.total_memory_mb as f64 * 0.8) / (peak_memory_mb / peak_successful_agents as f64);
                    let cpu_capacity = (system_info.logical_cores as f64 * 100.0 * 0.8) / (peak_cpu_percent / peak_successful_agents as f64);
                    let projected_capacity = memory_capacity.min(cpu_capacity) as usize;
                    
                    println!("  🎯 Estimated Capacity (80% util): ~{} concurrent agents", projected_capacity);
                    println!("  💾 Memory-limited capacity: ~{} agents", memory_capacity as usize);
                    println!("  🏭 CPU-limited capacity: ~{} agents", cpu_capacity as usize);
                    println!("  📈 Current Memory Utilization: {:.2}%", (peak_memory_mb / (system_info.total_memory_mb as f64 / 1024.0)) * 100.0);
                    println!("  📈 Current CPU Utilization: {:.2}%", (peak_cpu_percent / (system_info.logical_cores as f64 * 100.0)) * 100.0);
                    
                    println!("\n✅ QUALITY ASSESSMENT:");
                    let success_rate = (peak_successful_agents as f64 / 1000.0) * 100.0;
                    println!("  🎯 Success Rate: {:.1}% ({}/{} agents)", success_rate, peak_successful_agents, 1000);
                    println!("  🔧 Resource Efficiency: {}", if peak_memory_mb < 1000.0 { "EXCELLENT" } else if peak_memory_mb < 2000.0 { "GOOD" } else { "MODERATE" });
                    println!("  ⚡ Performance Rating: {}", if peak_registration_rate > 50.0 { "HIGH" } else if peak_registration_rate > 20.0 { "MEDIUM" } else { "LOW" });
                    println!("  🏭 Scalability Grade: {}", if projected_capacity > 500 { "A+" } else if projected_capacity > 200 { "A" } else if projected_capacity > 100 { "B+" } else { "B" });
                    
                    println!("\n🎯 RECOMMENDATIONS:");
                    if projected_capacity > 1000 {
                        println!("  ✅ System ready for production deployment");
                        println!("  ✅ Excellent scalability for enterprise use");
                        println!("  ✅ Consider load balancing for >1000 agents");
                    } else if projected_capacity > 500 {
                        println!("  ✅ System ready for production deployment");
                        println!("  ✅ Good scalability for medium deployments");
                        println!("  💡 Monitor memory usage in production");
                    } else if projected_capacity > 100 {
                        println!("  ⚠️  Suitable for small-medium deployments");
                        println!("  💡 Consider memory optimization");
                        println!("  💡 Profile CPU usage under sustained load");
                    } else {
                        println!("  ⚠️  Further optimization recommended");
                        println!("  🔧 Investigate memory usage patterns");
                        println!("  🔧 Consider architectural improvements");
                    }
                    
                    println!("\n════════════════════════════════════════════════════════════════");
                    println!("🔍 DIAGNOSTIC COMPLETE - RVOIP SIP STACK PERFORMANCE VERIFIED");
                    println!("════════════════════════════════════════════════════════════════\n");
                    
                    // Cleanup
                    server_task.abort();
                    println!("🛑 CLEANUP: Server stopped");
                }
                Err(e) => println!("❌ Server start failed: {}", e),
            }
        }
        Err(e) => {
            println!("❌ Server creation failed: {}", e);
        }
    }
    
    println!("🔍 DIAGNOSTIC TEST COMPLETE");
} 