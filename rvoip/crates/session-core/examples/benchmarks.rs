use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Semaphore};
use tokio::time;
use rand::{rngs::SmallRng, SeedableRng, Rng};
use rvoip_sip_core::{
    Request, Response, Method, StatusCode, Uri, HeaderName, TypedHeader,
    builder::{SimpleRequestBuilder, SimpleResponseBuilder},
    types::{
        call_id::CallId,
        from::From as FromHeader,
        to::To as ToHeader,
        cseq::CSeq,
        address::Address,
        contact::Contact,
    }
};
use rvoip_transaction_core::{
    TransactionManager, TransactionEvent, TransactionKey, TransactionKind
};
use rvoip_session_core::{
    dialog::{Dialog, DialogId, DialogManager, DialogState},
    session::{Session, SessionId, SessionManager, SessionState, SessionConfig, SessionDirection},
    events::{EventBus, SessionEvent, EventHandler},
    Error,
    make_call, end_call, create_dialog_from_invite
};
use uuid::Uuid;
use std::collections::HashMap;
use futures::future::{join_all, try_join_all};
use tokio::task::JoinHandle;

// Mock transport implementation for benchmarking
#[derive(Clone, Debug)]
struct BenchmarkTransport {
    local_addr: std::net::SocketAddr,
}

impl BenchmarkTransport {
    fn new() -> Self {
        Self {
            local_addr: "127.0.0.1:5060".parse().unwrap(),
        }
    }
}

#[async_trait::async_trait]
impl rvoip_sip_transport::Transport for BenchmarkTransport {
    fn local_addr(&self) -> std::result::Result<std::net::SocketAddr, rvoip_sip_transport::error::Error> {
        Ok(self.local_addr)
    }
    
    async fn send_message(&self, _message: rvoip_sip_core::Message, _destination: std::net::SocketAddr) 
        -> std::result::Result<(), rvoip_sip_transport::error::Error> {
        // Mock implementation: don't actually send anything
        Ok(())
    }
    
    async fn close(&self) -> std::result::Result<(), rvoip_sip_transport::error::Error> {
        Ok(())
    }
    
    fn is_closed(&self) -> bool {
        false
    }
}

// Helper to create test SIP messages
fn create_test_invite(call_id: &str, from_tag: &str) -> Request {
    SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
        .unwrap()
        .from("Alice", "sip:alice@example.com", Some(from_tag))
        .to("Bob", "sip:bob@example.com", None)
        .call_id(call_id)
        .cseq(1)
        .contact("sip:alice@192.168.1.1", Some("Alice"))
        .via("192.168.1.1:5060", "UDP", Some(&format!("z9hG4bK-{}", Uuid::new_v4().as_simple())))
        .build()
}

fn create_test_response(request: &Request, status: StatusCode, with_to_tag: bool) -> Response {
    let mut builder = SimpleResponseBuilder::response_from_request(request, status, None);
    
    // Add a Contact header for dialog establishment
    builder = builder.contact("sip:bob@192.168.1.2", Some("Bob"));
    
    // If this is a response that should establish a dialog, add a to-tag
    if with_to_tag {
        let to_tag = format!("bob-tag-{}", Uuid::new_v4().as_simple());
        
        // Get original To header to extract display name and URI
        if let Some(TypedHeader::To(to)) = request.header(&HeaderName::To) {
            let display_name = to.address().display_name().unwrap_or("").to_string();
            let uri = to.address().uri.to_string();
            builder = builder.to(&display_name, &uri, Some(&to_tag));
        }
    }
    
    builder.build()
}

// Session and its associated transaction data
struct SessionData {
    session: Arc<Session>,
    transaction_id: Option<TransactionKey>,
    request: Option<Request>,
    dialog_id: Option<DialogId>,
}

// Send an artificial transaction event
async fn send_artificial_transaction_event(
    tx: &mpsc::Sender<TransactionEvent>,
    transaction_id: TransactionKey,
    request: Request,
    status: StatusCode,
) -> Result<Response, mpsc::error::SendError<TransactionEvent>> {
    let response = create_test_response(&request, status, true);
    
    let event = if status.is_success() {
        TransactionEvent::SuccessResponse {
            transaction_id,
            response: response.clone(),
            source: "127.0.0.1:5060".parse().unwrap(),
            need_ack: status == StatusCode::Ok && request.method() == Method::Invite,
        }
    } else if status.is_provisional() {
        TransactionEvent::ProvisionalResponse {
            transaction_id,
            response: response.clone(),
        }
    } else {
        TransactionEvent::FailureResponse {
            transaction_id,
            response: response.clone(),
        }
    };
    
    // Send without blocking if possible
    match tx.try_send(event) {
        Ok(_) => Ok(response),
        Err(mpsc::error::TrySendError::Full(event)) => {
            // Fall back to blocking send if channel is full
            tx.send(event).await?;
            Ok(response)
        },
        Err(mpsc::error::TrySendError::Closed(event)) => {
            Err(mpsc::error::SendError(event))
        }
    }
}

// Benchmark configuration
struct BenchmarkConfig {
    // Number of concurrent sessions to create
    session_count: usize,
    
    // Duration to run the benchmark for
    duration: Duration,
    
    // Maximum number of dialogs per session
    max_dialogs_per_session: usize,
    
    // Percentage of sessions that will terminate during the test
    termination_percentage: u8,
    
    // Time between cleanup operations
    cleanup_interval: Duration,

    // Maximum concurrent operations
    max_concurrency: usize,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            session_count: 1000,
            duration: Duration::from_secs(60),
            max_dialogs_per_session: 3,
            termination_percentage: 30,
            cleanup_interval: Duration::from_secs(5),
            max_concurrency: 500,
        }
    }
}

// Performance metrics
#[derive(Debug, Default, Clone)]
struct BenchmarkMetrics {
    // Session creation time statistics
    session_creation_total_ns: u64,
    session_creation_count: usize,
    
    // Dialog creation time statistics
    dialog_creation_total_ns: u64,
    dialog_creation_count: usize,
    
    // State transition time statistics
    state_transition_total_ns: u64,
    state_transition_count: usize,
    
    // Cleanup time statistics
    cleanup_total_ns: u64,
    cleanup_count: usize,
    cleanup_items: usize,
    
    // Memory usage statistics
    peak_memory_usage: usize,
    
    // Error counts
    errors: usize,
}

impl BenchmarkMetrics {
    fn record_session_creation(&mut self, duration: Duration) {
        self.session_creation_total_ns += duration.as_nanos() as u64;
        self.session_creation_count += 1;
    }
    
    fn record_dialog_creation(&mut self, duration: Duration) {
        self.dialog_creation_total_ns += duration.as_nanos() as u64;
        self.dialog_creation_count += 1;
    }
    
    fn record_state_transition(&mut self, duration: Duration) {
        self.state_transition_total_ns += duration.as_nanos() as u64;
        self.state_transition_count += 1;
    }
    
    fn record_cleanup(&mut self, duration: Duration, items: usize) {
        self.cleanup_total_ns += duration.as_nanos() as u64;
        self.cleanup_count += 1;
        self.cleanup_items += items;
    }
    
    fn record_error(&mut self) {
        self.errors += 1;
    }
    
    fn avg_session_creation_us(&self) -> f64 {
        if self.session_creation_count == 0 {
            0.0
        } else {
            (self.session_creation_total_ns as f64) / (self.session_creation_count as f64) / 1_000.0
        }
    }
    
    fn avg_dialog_creation_us(&self) -> f64 {
        if self.dialog_creation_count == 0 {
            0.0
        } else {
            (self.dialog_creation_total_ns as f64) / (self.dialog_creation_count as f64) / 1_000.0
        }
    }
    
    fn avg_state_transition_us(&self) -> f64 {
        if self.state_transition_count == 0 {
            0.0
        } else {
            (self.state_transition_total_ns as f64) / (self.state_transition_count as f64) / 1_000.0
        }
    }
    
    fn avg_cleanup_ms(&self) -> f64 {
        if self.cleanup_count == 0 {
            0.0
        } else {
            (self.cleanup_total_ns as f64) / (self.cleanup_count as f64) / 1_000_000.0
        }
    }
    
    fn avg_items_per_cleanup(&self) -> f64 {
        if self.cleanup_count == 0 {
            0.0
        } else {
            (self.cleanup_items as f64) / (self.cleanup_count as f64)
        }
    }
}

// Run the benchmark
async fn run_benchmark(config: BenchmarkConfig) -> BenchmarkMetrics {
    let mut metrics = BenchmarkMetrics::default();
    
    // Create a transport and transaction manager
    let transport = Arc::new(BenchmarkTransport::new());
    let (transport_tx, transport_rx) = mpsc::channel::<rvoip_sip_transport::TransportEvent>(config.session_count * 2);
    let (transaction_manager, event_rx) = TransactionManager::new(transport.clone(), transport_rx, Some(config.session_count)).await.unwrap();
    let transaction_manager = Arc::new(transaction_manager);

    // Create a transaction event channel that the transaction manager will use
    let (tx, mut rx) = mpsc::channel::<rvoip_transaction_core::TransactionEvent>(config.session_count * 2);
    
    // Create a task to forward events to the transaction manager's event subscribers
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            // Forward to any existing subscribers
            if let Err(e) = tx_clone.send(event).await {
                println!("Error forwarding event: {}", e);
            }
        }
    });
    
    // Create an event bus with larger capacity
    let event_bus = EventBus::new(config.session_count * 2);

    // Track dialog creations with a counter
    let dialog_counter = Arc::new(tokio::sync::Mutex::new(0));
    let dialog_counter_clone = dialog_counter.clone();

    // Add an event handler to track dialog creations
    struct DialogTrackingHandler {
        counter: Arc<tokio::sync::Mutex<usize>>,
        metrics: Arc<tokio::sync::Mutex<BenchmarkMetrics>>,
    }

    #[async_trait::async_trait]
    impl EventHandler for DialogTrackingHandler {
        async fn handle_event(&self, event: SessionEvent) {
            match event {
                SessionEvent::DialogUpdated { session_id: _, dialog_id: _ } => {
                    // Record that a dialog was created/updated
                    let mut counter = self.counter.lock().await;
                    *counter += 1;

                    let start = Instant::now();
                    // Record metrics
                    let mut metrics = self.metrics.lock().await;
                    metrics.record_dialog_creation(start.elapsed());
                },
                _ => {}
            }
        }
    }

    let metrics_for_events = Arc::new(tokio::sync::Mutex::new(BenchmarkMetrics::default()));
    let event_handler = Arc::new(DialogTrackingHandler {
        counter: dialog_counter_clone,
        metrics: metrics_for_events.clone(),
    });
    event_bus.register_handler(event_handler).await;
    
    // Create a session manager
    let session_config = SessionConfig {
        local_signaling_addr: "127.0.0.1:5060".parse().unwrap(),
        local_media_addr: "127.0.0.1:10000".parse().unwrap(),
        supported_codecs: vec![],
        display_name: None,
        user_agent: "RVOIP-Benchmark/0.1.0".to_string(),
        max_duration: 0,
        max_sessions: Some(config.session_count),
    };
    let session_manager = Arc::new(SessionManager::new(
        transaction_manager.clone(),
        session_config,
        event_bus.clone()
    ));
    
    // Start the session manager
    let _ = session_manager.start().await;
    
    // Store active session data
    let active_sessions = Arc::new(tokio::sync::Mutex::new(Vec::with_capacity(config.session_count)));
    
    // Start timestamp
    let start_time = Instant::now();
    
    // Create semaphore for controlling concurrency
    let semaphore = Arc::new(Semaphore::new(config.max_concurrency));
    
    // Create initial sessions in parallel
    println!("Creating {} sessions concurrently...", config.session_count);
    
    let session_manager_clone = session_manager.clone();
    let active_sessions_clone = active_sessions.clone();
    let metrics_arc = Arc::new(tokio::sync::Mutex::new(metrics));
    
    let mut session_tasks = Vec::with_capacity(config.session_count);
    
    for i in 0..config.session_count {
        let session_manager = session_manager_clone.clone();
        let active_sessions = active_sessions_clone.clone();
        let metrics = metrics_arc.clone();
        let semaphore = semaphore.clone();
        
        let task = tokio::spawn(async move {
            // Acquire semaphore permit
            let _permit = semaphore.acquire().await.unwrap();
            
            let creation_start = Instant::now();
            
            // Create a destination URI for the call
            let destination = Uri::sip(&format!("bench-user-{}-{}", i, Uuid::new_v4().as_simple()));
            
            // Use make_call helper
            match make_call(&session_manager, destination).await {
                Ok(session) => {
                    // Record metrics
                    metrics.lock().await.record_session_creation(creation_start.elapsed());
                    
                    // Store session data
                    let session_data = SessionData {
                        session,
                        transaction_id: None,
                        request: None,
                        dialog_id: None,
                    };
                    
                    // Add to active sessions
                    active_sessions.lock().await.push(session_data);
                    
                    true
                },
                Err(_) => {
                    metrics.lock().await.record_error();
                    false
                }
            }
        });
        
        session_tasks.push(task);
    }
    
    // Wait for all session creation tasks to complete
    let results = join_all(session_tasks).await;
    let successful_sessions = results.iter().filter(|r| r.as_ref().map_or(false, |&x| x)).count();
    
    println!("Created {} sessions successfully.", successful_sessions);
    
    // Set up periodic cleanup
    let session_manager_clone = session_manager.clone();
    let metrics_cleanup_clone = metrics_arc.clone();
    
    let cleanup_handle = tokio::spawn(async move {
        let mut interval = time::interval(config.cleanup_interval);
        loop {
            interval.tick().await;
            
            // Perform cleanup
            let cleanup_start = Instant::now();
            let session_count = session_manager_clone.cleanup_terminated().await;
            let dialog_count = session_manager_clone.dialog_manager().cleanup_terminated();
            let cleanup_duration = cleanup_start.elapsed();
            
            // Record metrics
            let mut metrics = metrics_cleanup_clone.lock().await;
            metrics.record_cleanup(cleanup_duration, session_count + dialog_count);
        }
    });
    
    // Create dialogs concurrently for all sessions
    println!("Creating dialogs concurrently for all sessions...");
    
    let tx_for_tasks = tx.clone();
    let session_manager_clone = session_manager.clone();
    let metrics_clone = metrics_arc.clone();
    let semaphore = Arc::new(Semaphore::new(config.max_concurrency));
    
    let active_sessions_locked = active_sessions.lock().await;
    let session_count = active_sessions_locked.len();
    drop(active_sessions_locked); // Release the lock
    
    let mut dialog_tasks = Vec::with_capacity(session_count);
    
    for i in 0..session_count {
        let tx = tx_for_tasks.clone();
        let session_manager = session_manager_clone.clone();
        let active_sessions = active_sessions.clone();
        let metrics = metrics_clone.clone();
        let semaphore_clone = semaphore.clone();
        
        let dialog_task = tokio::spawn(async move {
            // Acquire permit to control concurrency
            let _permit = semaphore_clone.acquire().await.unwrap();
            
            // Get the session
            let mut sessions = active_sessions.lock().await;
            if i >= sessions.len() {
                return false;
            }
            
            let session_data = &mut sessions[i];
            let session = session_data.session.clone();
            
            // Release the active_sessions lock early
            drop(sessions);
            
            // Get session ID
            let session_id = session.id.clone();
            
            // Only process session if it's in Initializing or Dialing state
            let session_state = session.state().await;
            if session_state != SessionState::Initializing && session_state != SessionState::Dialing {
                return false;
            }
            
            // Create dialog for this session
            let call_id = format!("bench-{}", Uuid::new_v4().as_simple());
            let from_tag = format!("tag-{}", Uuid::new_v4().as_simple());
            let request = create_test_invite(&call_id, &from_tag);
            
            // Create transaction ID
            let branch = format!("z9hG4bK-{}", Uuid::new_v4().as_simple());
            let transaction_id = TransactionKey::new(
                branch,
                Method::Invite,
                false // Client transaction
            );
            
            // Associate the transaction with the session
            let _ = session.track_transaction(transaction_id.clone(), 
                rvoip_session_core::session::SessionTransactionType::InitialInvite).await;
            
            // Send provisional response
            let response = match send_artificial_transaction_event(
                &tx,
                transaction_id.clone(),
                request.clone(),
                StatusCode::Ringing
            ).await {
                Ok(r) => r,
                Err(_) => return false,
            };
            
            // Change state to Ringing
            let transition_start = Instant::now();
            if let Err(_) = session.set_state(SessionState::Ringing).await {
                metrics.lock().await.record_error();
                return false;
            }
            metrics.lock().await.record_state_transition(transition_start.elapsed());
            
            // Create dialog from the request/response
            let dialog_creation_start = Instant::now();
            let dialog_mgr = session_manager.dialog_manager();
            
            // Create dialog using the transaction key
            let dialog_result = dialog_mgr.create_dialog_from_transaction(
                &transaction_id,
                &request,
                &response,
                true  // We're the initiator (UAC)
            ).await;
            
            if let Some(dialog_id) = dialog_result {
                // Associate with session
                if let Err(_) = dialog_mgr.associate_with_session(&dialog_id, &session_id) {
                    metrics.lock().await.record_error();
                    return false;
                }
                
                // Record dialog creation metrics
                metrics.lock().await.record_dialog_creation(dialog_creation_start.elapsed());
                
                // Update the session_data with transaction and dialog information
                let mut sessions = active_sessions.lock().await;
                if i < sessions.len() {
                    let session_data = &mut sessions[i];
                    session_data.transaction_id = Some(transaction_id);
                    session_data.request = Some(request);
                    session_data.dialog_id = Some(dialog_id);
                }
                
                return true;
            } else {
                metrics.lock().await.record_error();
                return false;
            }
        });
        
        dialog_tasks.push(dialog_task);
    }
    
    // Process dialog creation tasks in batches to see progress
    let batch_size = (1000).min(dialog_tasks.len());
    let mut remaining_tasks = dialog_tasks;
    let mut completed_dialogs = 0;
    let mut batch_num = 0;
    
    // Process multiple batches concurrently to improve throughput
    let max_concurrent_batches = 5;
    
    while !remaining_tasks.is_empty() {
        let mut batch_handles = Vec::new();
        
        // Start up to max_concurrent_batches batches
        for _ in 0..max_concurrent_batches {
            if remaining_tasks.is_empty() {
                break;
            }
            
            let (batch, rest) = if remaining_tasks.len() <= batch_size {
                (remaining_tasks, vec![])
            } else {
                let rest = remaining_tasks.split_off(batch_size);
                (remaining_tasks, rest)
            };
            
            remaining_tasks = rest;
            batch_num += 1;
            
            // Process this batch in a separate task
            let batch_handle = tokio::spawn(async move {
                let batch_size = batch.len();
                let results = join_all(batch).await;
                let successful = results.iter()
                    .filter(|r| r.as_ref().map_or(false, |&success| success))
                    .count();
                (successful, batch_size)
            });
            
            batch_handles.push(batch_handle);
        }
        
        // Wait for all started batches to complete
        let batch_results = join_all(batch_handles).await;
        
        for result in batch_results {
            if let Ok((successful, total)) = result {
                completed_dialogs += successful;
                println!("Batch {}: Created {} of {} dialogs. Total: {} dialogs.", 
                    batch_num, successful, total, completed_dialogs);
            }
        }
    }
    
    println!("Dialog creation complete. Created {} dialogs for {} sessions.", 
        completed_dialogs, session_count);
    
    // Move sessions through their lifecycle concurrently (ringing -> connected -> terminated)
    println!("Running session lifecycle simulation...");
    let remaining_time = config.duration.saturating_sub(start_time.elapsed());
    let end_time = Instant::now() + remaining_time;
    
    let active_sessions_clone = active_sessions.clone();
    let tx_clone = tx.clone();
    let metrics_clone = metrics_arc.clone();
    
    // Create a fixed pool of workers to process sessions
    let worker_count = config.max_concurrency / 10; // More workers for better concurrency
    let mut worker_tasks = Vec::with_capacity(worker_count);
    
    for worker_id in 0..worker_count {
        let active_sessions = active_sessions_clone.clone();
        let tx = tx_clone.clone();
        let metrics = metrics_clone.clone();
        let end = end_time;
        
        let worker = tokio::spawn(async move {
            // Use SmallRng which is Send-compatible
            let mut rng = SmallRng::from_entropy();
            let mut processed_count = 0;
            
            while Instant::now() < end {
                // Get a random session to process
                let mut sessions = active_sessions.lock().await;
                if sessions.is_empty() {
                    break;
                }
                
                let session_idx = rng.gen_range(0..sessions.len());
                let session_data = &mut sessions[session_idx];
                let session = session_data.session.clone();
                
                let session_state = session.state().await;
                let transaction_id = session_data.transaction_id.clone();
                let request = session_data.request.clone();
                
                // Drop lock early
                drop(sessions);
                
                match session_state {
                    SessionState::Ringing => {
                        // Use the transaction associated with this session
                        if let (Some(transaction_id), Some(request)) = (transaction_id, request) {
                            // Send success response
                            let _ = send_artificial_transaction_event(
                                &tx,
                                transaction_id,
                                request,
                                StatusCode::Ok
                            ).await;
                            
                            // Change state to Connected
                            let transition_start = Instant::now();
                            if let Err(_) = session.set_state(SessionState::Connected).await {
                                metrics.lock().await.record_error();
                            } else {
                                metrics.lock().await.record_state_transition(transition_start.elapsed());
                                processed_count += 1;
                            }
                        }
                    },
                    SessionState::Connected => {
                        // Decide whether to terminate
                        let should_terminate = rng.gen_range(0..100) < config.termination_percentage;
                        
                        if should_terminate {
                            // Use end_call helper instead of manual state transitions
                            let transition_start = Instant::now();
                            if let Err(_) = end_call(&session).await {
                                metrics.lock().await.record_error();
                            } else {
                                // Record both state transitions at once
                                metrics.lock().await.record_state_transition(transition_start.elapsed());
                                metrics.lock().await.record_state_transition(Duration::from_nanos(0));
                                processed_count += 1;
                                
                                // Remove from active sessions
                                let mut sessions = active_sessions.lock().await;
                                if session_idx < sessions.len() && sessions[session_idx].session.id == session.id {
                                    sessions.swap_remove(session_idx);
                                }
                            }
                        }
                    },
                    _ => {
                        // For other states, just continue to next session
                    }
                }
                
                // Small delay to prevent CPU spinning
                tokio::task::yield_now().await;
            }
            
            processed_count
        });
        
        worker_tasks.push(worker);
    }
    
    // Wait for all workers to complete
    let worker_results = join_all(worker_tasks).await;
    let total_processed = worker_results.iter()
        .filter_map(|r| r.as_ref().ok())
        .sum::<usize>();
    
    println!("Processed {} state transitions.", total_processed);
    
    // Terminate all sessions
    println!("Terminating all sessions...");
    let _ = session_manager.terminate_all().await;

    // Wait for cleanup task to run a few more times
    time::sleep(config.cleanup_interval.mul_f32(3.0)).await;

    // Cancel the cleanup task and wait for it to complete
    cleanup_handle.abort();
    let _ = cleanup_handle.await;

    // Get dialog count for reporting
    let dialog_count = *dialog_counter.lock().await;
    
    // Retrieve metrics from the Arc
    let final_metrics = match Arc::try_unwrap(metrics_arc) {
        Ok(mutex) => mutex.into_inner(),
        Err(arc) => {
            // If we can't unwrap, just lock and clone the data
            println!("Note: Could not unwrap metrics (still has references), cloning data instead");
            arc.lock().await.clone()
        }
    };

    // Merge in event metrics
    let event_metrics = match Arc::try_unwrap(metrics_for_events) {
        Ok(mutex) => mutex.into_inner(),
        Err(arc) => arc.lock().await.clone(),
    };

    // Combine the metrics
    let mut combined_metrics = final_metrics;
    
    // If event_metrics has more dialog creations, use that count
    if event_metrics.dialog_creation_count > combined_metrics.dialog_creation_count {
        combined_metrics.dialog_creation_count = event_metrics.dialog_creation_count;
        combined_metrics.dialog_creation_total_ns = event_metrics.dialog_creation_total_ns;
    }
    
    // If we have a direct dialog count from the counter, that's most accurate
    if dialog_count > 0 {
        combined_metrics.dialog_creation_count = dialog_count;
    }

    combined_metrics
}

#[tokio::main]
async fn main() {
    // Setup tracing
    tracing_subscriber::fmt::init();
    
    println!("Starting session-core benchmark...");
    
    // Custom benchmark configuration
    let config = BenchmarkConfig {
        session_count: 10000,   // Benchmark with 10,000 concurrent sessions
        duration: Duration::from_secs(30),
        max_dialogs_per_session: 2,
        termination_percentage: 25,
        cleanup_interval: Duration::from_secs(3),
        max_concurrency: 10000,   // Increased to full concurrency
    };
    
    // Run the benchmark
    let start_time = Instant::now();
    let metrics = run_benchmark(config).await;
    let total_duration = start_time.elapsed();
    
    // Print results
    println!("\nBenchmark Results");
    println!("================");
    println!("Total duration: {:.2?}", total_duration);
    println!("");
    println!("Session creation average: {:.2} µs", metrics.avg_session_creation_us());
    println!("Dialog creation average: {:.2} µs", metrics.avg_dialog_creation_us());
    println!("State transition average: {:.2} µs", metrics.avg_state_transition_us());
    println!("Cleanup average: {:.2} ms", metrics.avg_cleanup_ms());
    println!("Items per cleanup: {:.2}", metrics.avg_items_per_cleanup());
    println!("");
    println!("Operation counts:");
    println!("  Session creations: {}", metrics.session_creation_count);
    println!("  Dialog creations: {}", metrics.dialog_creation_count);
    println!("  State transitions: {}", metrics.state_transition_count);
    println!("  Cleanup operations: {}", metrics.cleanup_count);
    println!("");
    println!("Errors: {}", metrics.errors);
} 