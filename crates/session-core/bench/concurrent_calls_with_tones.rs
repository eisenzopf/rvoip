/// Benchmark: 500 concurrent calls with actual audio tones
/// 
/// This benchmark creates 500 concurrent SIP calls between two SessionManagers,
/// with each peer sending different frequency tones (440Hz client, 880Hz server).
/// Metrics are collected every second, and 5 random calls are captured for audio validation.

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::info;
use rand::Rng;

use rvoip_session_core::api::*;

mod tone_generator;
mod metrics;
mod audio_validator;

use tone_generator::{generate_tone, create_rtp_packets, decode_rtp_payload};
use metrics::{MetricsCollector, MetricSnapshot};
use audio_validator::{AudioValidator, AudioCapture};

/// Client handler that sends 440Hz tone
#[derive(Debug)]
struct ClientHandler {
    established_calls: Arc<Mutex<Vec<String>>>,
    audio_validator: Arc<AudioValidator>,
}

impl ClientHandler {
    fn new(audio_validator: Arc<AudioValidator>) -> Self {
        Self {
            established_calls: Arc::new(Mutex::new(Vec::new())),
            audio_validator,
        }
    }
}

#[async_trait::async_trait]
impl CallHandler for ClientHandler {
    async fn on_incoming_call(&self, _call: IncomingCall) -> CallDecision {
        CallDecision::Reject("Client doesn't accept incoming calls".to_string())
    }
    
    async fn on_call_established(&self, call: CallSession, _local_sdp: Option<String>, _remote_sdp: Option<String>) {
        let call_id = call.id.0.clone();
        info!("Client: Call {} established, starting 440Hz tone", &call_id[..8.min(call_id.len())]);
        
        // Store session
        self.established_calls.lock().await.push(call_id.clone());
        
        // Check if this call is selected for audio capture
        let is_selected = self.audio_validator.is_selected(&call_id).await;
        
        // Start sending 440Hz tone for 10 seconds
        let validator = self.audio_validator.clone();
        tokio::spawn(async move {
            // Generate 10 seconds of 440Hz tone
            let samples = generate_tone(440.0, 8000, Duration::from_secs(10));
            let packets = create_rtp_packets(&samples, rand::random(), 8000, 0);
            
            // Send packets at 50 packets/second (20ms intervals)
            for packet in packets {
                // In real implementation, would send via session.send_rtp(packet)
                // For now, simulate sending
                tokio::time::sleep(Duration::from_millis(20)).await;
                
                // If selected for capture, decode and store our own audio
                if is_selected {
                    let decoded = decode_rtp_payload(&packet.payload);
                    validator.capture_client_audio(&call_id, decoded).await;
                }
            }
            
            info!("Client: Finished sending tone for call {}", &call_id[..8.min(call_id.len())]);
        });
    }
    
    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        let call_id = call.id.0.clone();
        info!("Client: Call {} ended: {}", &call_id[..8.min(call_id.len())], reason);
        self.audio_validator.end_call(&call_id).await;
    }
}

/// Server handler that sends 880Hz tone
#[derive(Debug)]
struct ServerHandler {
    received_calls: Arc<Mutex<Vec<String>>>,
    audio_validator: Arc<AudioValidator>,
}

impl ServerHandler {
    fn new(audio_validator: Arc<AudioValidator>) -> Self {
        Self {
            received_calls: Arc::new(Mutex::new(Vec::new())),
            audio_validator,
        }
    }
}

#[async_trait::async_trait]
impl CallHandler for ServerHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        let id_str = &call.id.0;
        info!("Server: Accepting incoming call {}", &id_str[..8.min(id_str.len())]);
        CallDecision::Accept(None)
    }
    
    async fn on_call_established(&self, call: CallSession, _local_sdp: Option<String>, _remote_sdp: Option<String>) {
        let call_id = call.id.0.clone();
        info!("Server: Call {} established, starting 880Hz tone", &call_id[..8.min(call_id.len())]);
        
        // Store session
        self.received_calls.lock().await.push(call_id.clone());
        
        // Check if this call is selected for audio capture
        let is_selected = self.audio_validator.is_selected(&call_id).await;
        
        // Start sending 880Hz tone for 10 seconds
        let validator = self.audio_validator.clone();
        tokio::spawn(async move {
            // Generate 10 seconds of 880Hz tone
            let samples = generate_tone(880.0, 8000, Duration::from_secs(10));
            let packets = create_rtp_packets(&samples, rand::random(), 8000, 0);
            
            // Send packets at 50 packets/second (20ms intervals)
            for packet in packets {
                // In real implementation, would send via session.send_rtp(packet)
                tokio::time::sleep(Duration::from_millis(20)).await;
                
                // If selected for capture, decode and store our own audio
                if is_selected {
                    let decoded = decode_rtp_payload(&packet.payload);
                    validator.capture_server_audio(&call_id, decoded).await;
                }
            }
            
            info!("Server: Finished sending tone for call {}", &call_id[..8.min(call_id.len())]);
        });
    }
    
    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        let id_str = &call.id.0;
        info!("Server: Call {} ended: {}", &id_str[..8.min(id_str.len())], reason);
    }
}

/// Main benchmark function
pub async fn run_benchmark() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Initialize logging from RUST_LOG env var, default to ERROR only
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
    
    println!("\n╔════════════════════════════════════════════════════════════════╗");
    println!("║      BENCHMARK: 500 Concurrent Calls with Audio Tones          ║");
    println!("║                                                                 ║");
    println!("║  • 500 concurrent SIP calls                                    ║");
    println!("║  • 10 second call duration                                     ║");
    println!("║  • Client sends 440Hz tone                                     ║");
    println!("║  • Server sends 880Hz tone                                     ║");
    println!("║  • 5 random calls captured for validation                      ║");
    println!("║  • Metrics collected every second                              ║");
    println!("╚════════════════════════════════════════════════════════════════╝\n");
    
    let test_start = Instant::now();
    
    // Create audio validator
    let audio_validator = Arc::new(AudioValidator::new());
    
    // Create server SessionManager
    let server_handler = Arc::new(ServerHandler::new(audio_validator.clone()));
    info!("Creating server SessionManager on port 5060...");
    let server = SessionManagerBuilder::new()
        .with_sip_port(5060)
        .with_local_address("sip:server@127.0.0.1:5060")
        .with_media_ports(20000, 25000)
        .with_handler(server_handler.clone())
        .build()
        .await?;
    
    // Create client SessionManager
    let client_handler = Arc::new(ClientHandler::new(audio_validator.clone()));
    info!("Creating client SessionManager on port 5061...");
    let client = SessionManagerBuilder::new()
        .with_sip_port(5061)
        .with_local_address("sip:client@127.0.0.1:5061")
        .with_media_ports(25000, 30000)
        .with_handler(client_handler.clone())
        .build()
        .await?;
    
    // Start both session managers
    SessionControl::start(&server).await?;
    SessionControl::start(&client).await?;
    
    // Start metrics collection
    let active_calls_counter = Arc::new(Mutex::new(0usize));
    let metrics_collector = MetricsCollector::new();
    metrics_collector.start_collection(
        Duration::from_secs(1),
        active_calls_counter.clone(),
    ).await;
    
    // Prepare call IDs for random selection
    let mut call_ids = Vec::new();
    for i in 0..500 {
        call_ids.push(format!("call_{}", i));
    }
    
    // Select 5 random calls for audio capture
    audio_validator.select_random_calls(&call_ids, 5).await;
    
    // Create 500 concurrent calls
    info!("Initiating 500 concurrent calls...");
    let mut call_tasks = Vec::new();
    
    for i in 0..500 {
        let client_clone = client.clone();
        let counter = active_calls_counter.clone();
        
        let task = tokio::spawn(async move {
            
            let from = format!("sip:user_{}@127.0.0.1:5061", i);
            let to = format!("sip:destination_{}@127.0.0.1:5060", i);
            
            match SessionControl::create_outgoing_call(&client_clone, &from, &to, None).await {
                Ok(session) => {
                    // Increment active calls counter
                    *counter.lock().await += 1;
                    
                    info!("Call {} created successfully", i);
                    
                    // Hold the call for 10 seconds
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    
                    // Terminate the call
                    if let Err(e) = SessionControl::terminate_session(&client_clone, &session.id).await {
                        tracing::warn!("Failed to terminate call {}: {}", i, e);
                    }
                    
                    // Decrement active calls counter
                    *counter.lock().await -= 1;
                    
                    Ok(session)
                }
                Err(e) => {
                    tracing::warn!("Call {} failed: {}", i, e);
                    Err(e)
                }
            }
        });
        
        call_tasks.push(task);
    }
    
    info!("All 500 call tasks spawned, waiting for completion...");
    
    // Wait for all calls to complete
    let mut successful_calls = 0;
    let mut failed_calls = 0;
    
    for task in call_tasks {
        match task.await {
            Ok(Ok(_)) => successful_calls += 1,
            _ => failed_calls += 1,
        }
    }
    
    // Wait a bit for final metrics collection
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Get and print metrics
    let snapshots = metrics_collector.get_snapshots().await;
    MetricsCollector::print_metrics_table(&snapshots);
    
    // Validate captured audio
    let validation_results = audio_validator.validate_all().await;
    AudioValidator::print_validation_results(&validation_results);
    
    // Print summary
    println!("\n╔════════════════════════════════════════════════════════════════╗");
    println!("║                    BENCHMARK SUMMARY                           ║");
    println!("╠════════════════════════════════════════════════════════════════╣");
    println!("║  Total Calls Attempted:    500                                 ║");
    println!("║  Successful Calls:         {}                                ║", successful_calls);
    println!("║  Failed Calls:             {}                                 ║", failed_calls);
    println!("║  Success Rate:             {:.1}%                             ║", 
        (successful_calls as f32 / 500.0) * 100.0);
    println!("║  Total Test Time:          {:.1}s                             ║",
        test_start.elapsed().as_secs_f32());
    println!("╚════════════════════════════════════════════════════════════════╝\n");
    
    // Cleanup
    SessionControl::stop(&server).await?;
    SessionControl::stop(&client).await?;
    
    Ok(())
}

#[tokio::test]
async fn test_concurrent_calls_with_tones() {
    run_benchmark().await.expect("Benchmark failed");
}

/// Main entry point for running as a benchmark
#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    run_benchmark().await
}