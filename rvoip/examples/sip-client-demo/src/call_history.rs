use std::net::SocketAddr;
use std::time::{Duration, SystemTime};
use std::str::FromStr;
use std::fmt::Write;

use clap::Parser;
use tracing::{info, debug, error, warn};

// Updated imports from the refactored SIP client library
use rvoip_sip_client::{
    UserAgent, ClientConfig, CallEvent, CallState, CallDirection,
    call_registry::{CallRegistry, CallRecord, CallFilter, CallStatistics, SerializableCallLookupResult}
};

/// Command-line arguments for the call history demo
#[derive(Parser, Debug)]
#[clap(name = "call_history", about = "SIP call history demo")]
struct Args {
    /// Local IP address to bind to
    #[clap(short = 'i', long, default_value = "127.0.0.1")]
    local_ip: String,
    
    /// Local port to bind to
    #[clap(short = 'p', long, default_value = "5072")]
    local_port: u16,
    
    /// Username
    #[clap(short, long, default_value = "charlie")]
    username: String,
    
    /// Domain
    #[clap(short, long, default_value = "rvoip.local")]
    domain: String,
    
    /// Number of calls to keep in history
    #[clap(short = 'n', long, default_value = "10")]
    history_size: usize,
    
    /// Whether to accept incoming calls
    #[clap(short, long)]
    auto_answer: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();
    
    // Parse command-line arguments
    let args = Args::parse();
    
    // Create local address
    let local_addr = SocketAddr::from_str(&format!("{}:{}", args.local_ip, args.local_port))?;
    
    info!("Starting SIP call history demo on {}", local_addr);
    info!("Username: {}, Domain: {}", args.username, args.domain);
    info!("Auto-answer: {}", args.auto_answer);
    info!("History size: {}", args.history_size);
    
    // Create client configuration
    let config = ClientConfig::new()
        .with_username(args.username)
        .with_domain(args.domain)
        .with_local_addr(local_addr)
        .with_max_call_history(Some(args.history_size))
        .with_persist_call_history(false);  // Not persisting for demo
    
    // Create user agent
    let mut user_agent = UserAgent::new(config).await?;
    
    // Start the user agent
    user_agent.start().await?;
    
    // Get the call registry
    let registry = user_agent.registry();
    
    // Create a channel for call events
    let mut call_events = user_agent.event_stream();
    
    info!("SIP call history demo started and listening for calls...");
    info!("Press Ctrl+C to exit");
    
    // Periodically print call history and statistics
    let registry_clone = registry.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(10)).await;
            
            // Calculate overall statistics
            let stats = registry_clone.calculate_statistics().await;
            
            // Format and display statistics
            info!("Call Statistics Summary:");
            info!("  Total Calls: {}", stats.total_calls);
            info!("  Incoming: {}, Outgoing: {}", stats.incoming_calls, stats.outgoing_calls);
            info!("  Established: {}, Failed: {}, Missed: {}", 
                 stats.established_calls, stats.failed_calls, stats.missed_calls);
            
            if let Some(avg_duration) = stats.average_duration {
                info!("  Average Duration: {:.1}s", avg_duration.as_secs_f32());
            }
            
            if let Some(max_duration) = stats.max_duration {
                info!("  Longest Call: {:.1}s", max_duration.as_secs_f32());
            }
            
            let mut state_counts = String::new();
            for (state, count) in &stats.calls_by_state {
                if *count > 0 {
                    let _ = write!(state_counts, "{}:{} ", state, count);
                }
            }
            if !state_counts.is_empty() {
                info!("  Calls by State: {}", state_counts);
            }
            
            // Get recent statistics (last 30 minutes)
            let recent_stats = registry_clone.calculate_recent_statistics(Duration::from_secs(30 * 60)).await;
            info!("Recent Call Activity (last 30 minutes):");
            info!("  Total: {}, Established: {}, Failed: {}", 
                 recent_stats.total_calls, recent_stats.established_calls, recent_stats.failed_calls);
            
            // Get time range statistics for demonstration
            let five_mins_ago = SystemTime::now().checked_sub(Duration::from_secs(5 * 60))
                .unwrap_or(SystemTime::UNIX_EPOCH);
            let now = SystemTime::now();
            
            let range_stats = registry_clone.calculate_statistics_in_time_range(five_mins_ago, now).await;
            info!("Call Activity in the last 5 minutes:");
            info!("  Total: {}, Incoming: {}, Outgoing: {}", 
                 range_stats.total_calls, range_stats.incoming_calls, range_stats.outgoing_calls);
            
            // Get all calls
            let call_history = registry_clone.call_history().await;
            if !call_history.is_empty() {
                info!("Call History ({} calls):", call_history.len());
                
                // Sort calls by start time (most recent first)
                let mut calls: Vec<_> = call_history.values().collect();
                calls.sort_by(|a, b| b.start_time.cmp(&a.start_time));
                
                // Show at most 5 most recent calls
                for record in calls.iter().take(5) {
                    let duration_str = match record.duration {
                        Some(d) => format!("{:.1}s", d.as_secs_f32()),
                        None => "ongoing".to_string(),
                    };
                    
                    info!("  Call {}: {} {} {}, {}",
                        record.id,
                        match record.direction {
                            CallDirection::Incoming => "from",
                            CallDirection::Outgoing => "to",
                        },
                        record.remote_uri,
                        record.state,
                        duration_str
                    );
                }
                
                if calls.len() > 5 {
                    info!("  ... and {} more calls", calls.len() - 5);
                }
            } else {
                info!("No calls in history yet");
            }
        }
    });
    
    // Process call events in the foreground
    while let Some(event) = call_events.recv().await {
        match event {
            CallEvent::Ready => {
                info!("SIP call event system ready");
            },
            CallEvent::IncomingCall(call) => {
                info!("Incoming call from {}", call.remote_uri());
                
                if args.auto_answer {
                    // Check call state before answering
                    let state = call.state().await;
                    if state == CallState::Ringing {
                        info!("Auto-answering call in Ringing state");
                        match call.answer().await {
                            Ok(_) => info!("Call answered successfully"),
                            Err(e) => error!("Failed to answer call: {}", e),
                        }
                    } else {
                        info!("Call already in {} state, not sending explicit answer", state);
                    }
                } else {
                    info!("Not auto-answering call (auto_answer disabled)");
                }
            },
            CallEvent::StateChanged { call, previous, current } => {
                info!("Call state changed: {} -> {}", previous, current);
                
                // Demonstrate finding a call by ID using find_call_by_id
                let call_id = call.id().to_string();
                match registry.find_call_by_id(&call_id).await {
                    Some(lookup_result) => {
                        info!("Found call {} in registry:", call_id);
                        info!("  Call record state: {}", lookup_result.record.state);
                        info!("  Call record duration: {:?}", lookup_result.record.duration);
                        
                        // Demonstrate using the active call reference if available
                        if lookup_result.active_call.is_some() {
                            info!("  Call is active");
                        }
                        
                        // Clone the lookup result for weak_call to avoid partial move
                        let lookup_clone = lookup_result.clone();
                        
                        // Demonstrate using the weak call reference (safer)
                        if let Some(weak_call) = lookup_clone.weak_call {
                            match weak_call.state().await {
                                Ok(state) => info!("  Call weak reference available, state: {}", state),
                                Err(e) => info!("  Call weak reference available, error getting state: {}", e),
                            }
                        }
                        
                        // Demonstrate creating a serializable version for API responses
                        let serializable = SerializableCallLookupResult::from(lookup_result);
                        info!("  Created serializable version with has_active_call: {}", serializable.has_active_call);
                    },
                    None => {
                        // This should never happen as we're examining an existing call event
                        error!("Call {} not found in registry!", call_id);
                    }
                }
                
                // If a call is established, print some information
                if current == CallState::Established {
                    info!("Call established with {}", call.remote_uri());
                    
                    // Hang up the call after 10 seconds
                    let call_clone = call.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(10)).await;
                        info!("Automatically hanging up call after 10 seconds");
                        if let Err(e) = call_clone.hangup().await {
                            error!("Failed to hang up call: {}", e);
                        }
                    });
                }
            },
            CallEvent::Terminated { call, reason } => {
                info!("Call terminated: {}", reason);
                
                // Get the call record from registry
                let record = registry.get_call(call.id()).await;
                if let Some(record) = record {
                    info!("Call record: {} {} -> {}, duration: {:?}", 
                          record.id, 
                          match record.direction {
                              CallDirection::Incoming => "from",
                              CallDirection::Outgoing => "to",
                          },
                          record.remote_uri,
                          record.duration);
                }
            },
            CallEvent::MediaAdded { call, media_type } => {
                info!("Media added to call: {:?}", media_type);
            },
            _ => {
                // Ignore other events
            }
        }
    }
    
    Ok(())
} 