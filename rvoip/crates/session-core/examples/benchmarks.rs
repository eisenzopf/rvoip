use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time;
use rand::{thread_rng, Rng};
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

// Send an artificial transaction event
async fn send_artificial_transaction_event(
    tx: &mpsc::Sender<TransactionEvent>,
    transaction_id: TransactionKey,
    request: Request,
    status: StatusCode,
) -> Result<(), mpsc::error::SendError<TransactionEvent>> {
    let response = create_test_response(&request, status, true);
    
    let event = if status.is_success() {
        TransactionEvent::SuccessResponse {
            transaction_id,
            response,
            source: "127.0.0.1:5060".parse().unwrap(),
            need_ack: status == StatusCode::Ok && request.method() == Method::Invite,
        }
    } else if status.is_provisional() {
        TransactionEvent::ProvisionalResponse {
            transaction_id,
            response,
        }
    } else {
        TransactionEvent::FailureResponse {
            transaction_id,
            response,
        }
    };
    
    tx.send(event).await
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
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            session_count: 1000,
            duration: Duration::from_secs(60),
            max_dialogs_per_session: 3,
            termination_percentage: 30,
            cleanup_interval: Duration::from_secs(5),
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
    let (transport_tx, transport_rx) = mpsc::channel::<rvoip_sip_transport::TransportEvent>(1000);
    let (transaction_manager, event_rx) = TransactionManager::new(transport.clone(), transport_rx, Some(100)).await.unwrap();
    let transaction_manager = Arc::new(transaction_manager);

    // Create a transaction event channel that the transaction manager will use
    let (tx, mut rx) = mpsc::channel::<rvoip_transaction_core::TransactionEvent>(1000);

    // Create a task to forward events to the transaction manager's event subscribers
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        let mut event_rx = event_rx;
        while let Some(event) = rx.recv().await {
            // Forward to any existing subscribers
            if let Err(e) = tx_clone.send(event).await {
                println!("Error forwarding event: {}", e);
            }
        }
    });

    // Create an event bus
    let event_bus = EventBus::new(1000);

    // Add an event handler to track dialog creations
    let metrics_for_events = Arc::new(tokio::sync::Mutex::new(BenchmarkMetrics::default()));
    let metrics_for_events_clone = metrics_for_events.clone();

    struct DialogTrackingHandler {
        metrics: Arc<tokio::sync::Mutex<BenchmarkMetrics>>,
    }

    #[async_trait::async_trait]
    impl EventHandler for DialogTrackingHandler {
        async fn handle_event(&self, event: SessionEvent) {
            match event {
                SessionEvent::DialogUpdated { session_id: _, dialog_id: _ } => {
                    let start = Instant::now();
                    // Record the dialog creation
                    let mut metrics = self.metrics.lock().await;
                    metrics.dialog_creation_count += 1;
                    metrics.dialog_creation_total_ns += start.elapsed().as_nanos() as u64;
                },
                _ => {}
            }
        }
    }

    let event_handler = Arc::new(DialogTrackingHandler {
        metrics: metrics_for_events_clone,
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
    
    // Store active sessions and their associated transactions
    let mut active_sessions = Vec::with_capacity(config.session_count);
    let mut active_transactions = Vec::new();
    
    // Start timestamp
    let start_time = Instant::now();
    
    // Create initial sessions
    println!("Creating {} sessions...", config.session_count);
    for _ in 0..config.session_count {
        let creation_start = Instant::now();
        
        // Create a destination URI for the call
        let destination = Uri::sip(&format!("bench-user-{}@example.com", Uuid::new_v4().as_simple()));
        
        // Use make_call helper instead of direct session creation
        let session = match make_call(&session_manager, destination).await {
            Ok(s) => s,
            Err(_) => {
                metrics.record_error();
                continue;
            }
        };
        metrics.record_session_creation(creation_start.elapsed());
        
        // The helper already sets the state to Dialing, so we don't need to do it again
        active_sessions.push(session);
    }
    
    // Set up periodic cleanup
    let session_manager_clone = session_manager.clone();
    let metrics_arc = Arc::new(tokio::sync::Mutex::new(metrics));
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
    
    // Create random client transactions and state transitions
    let mut rng = thread_rng();
    
    // Main benchmark loop
    while start_time.elapsed() < config.duration {
        // Randomly select a session
        if active_sessions.is_empty() {
            break;
        }
        
        let session_idx = rng.gen_range(0..active_sessions.len());
        let session = active_sessions[session_idx].clone();
        
        // Random operation based on session state
        let session_state = session.state().await;
        
        match session_state {
            SessionState::Initializing | SessionState::Dialing => {
                // Simulate dialog creation with the benchmark
                let session_id = session.id.clone();
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

                // Create a provisional response
                let response = create_test_response(&request, StatusCode::Ringing, true);

                // Track transaction
                active_transactions.push((transaction_id.clone(), request.clone()));

                // Associate the transaction with the session
                let _ = session.track_transaction(transaction_id.clone(), 
                    rvoip_session_core::session::SessionTransactionType::InitialInvite).await;

                // Create dialog from the request/response
                let dialog_creation_start = Instant::now();
                let dialog_mgr = session_manager.dialog_manager();

                // Create dialog using the public API
                if let Some(dialog_id) = dialog_mgr.create_dialog_from_transaction(
                    &transaction_id,
                    &request,
                    &response,
                    true
                ).await {
                    // Associate with session
                    dialog_mgr.associate_with_session(&dialog_id, &session_id).unwrap_or_default();
                    
                    // Record dialog creation metrics
                    metrics_arc.lock().await.record_dialog_creation(dialog_creation_start.elapsed());
                }

                // Send provisional response
                let _ = send_artificial_transaction_event(
                    &tx,
                    transaction_id.clone(),
                    request.clone(),
                    StatusCode::Ringing
                ).await;
                
                // Change state to Ringing
                let transition_start = Instant::now();
                if let Err(_) = session.set_state(SessionState::Ringing).await {
                    metrics_arc.lock().await.record_error();
                } else {
                    metrics_arc.lock().await.record_state_transition(transition_start.elapsed());
                }
                
                // Add artificial delay
                time::sleep(Duration::from_millis(rng.gen_range(10..50))).await;
            },
            SessionState::Ringing => {
                // Find a transaction for this session
                if let Some((transaction_id, request)) = active_transactions.first().cloned() {
                    // Send success response
                    let _ = send_artificial_transaction_event(
                        &tx,
                        transaction_id.clone(),
                        request,
                        StatusCode::Ok
                    ).await;
                    
                    // Change state to Connected
                    let transition_start = Instant::now();
                    if let Err(_) = session.set_state(SessionState::Connected).await {
                        metrics_arc.lock().await.record_error();
                    } else {
                        metrics_arc.lock().await.record_state_transition(transition_start.elapsed());
                    }
                }
                
                // Add artificial delay
                time::sleep(Duration::from_millis(rng.gen_range(10..50))).await;
            },
            SessionState::Connected => {
                // Decide whether to terminate
                let should_terminate = rng.gen_range(0..100) < config.termination_percentage;
                
                if should_terminate {
                    // Use end_call helper instead of manual state transitions
                    let transition_start = Instant::now();
                    if let Err(_) = end_call(&session).await {
                        metrics_arc.lock().await.record_error();
                    } else {
                        // Record both state transitions at once since end_call makes two transitions
                        metrics_arc.lock().await.record_state_transition(transition_start.elapsed());
                        metrics_arc.lock().await.record_state_transition(Duration::from_nanos(0)); // Second transition
                    }
                    
                    // Remove from active sessions
                    if session_idx < active_sessions.len() {
                        active_sessions.swap_remove(session_idx);
                    }
                }
                
                // Add artificial delay
                time::sleep(Duration::from_millis(rng.gen_range(10..100))).await;
            },
            _ => {
                // For other states, just wait
                time::sleep(Duration::from_millis(rng.gen_range(10..50))).await;
            }
        }
    }
    
    // Terminate all sessions
    println!("Terminating all sessions...");
    let _ = session_manager.terminate_all().await;

    // Wait for cleanup task to run a few more times
    time::sleep(config.cleanup_interval.mul_f32(3.0)).await;

    // Cancel the cleanup task and wait for it to complete
    cleanup_handle.abort();
    let _ = cleanup_handle.await;

    // Retrieve metrics from the Arc - if we can't unwrap it, just lock and clone the data
    let final_metrics = match Arc::try_unwrap(metrics_arc) {
        Ok(mutex) => mutex.into_inner(),
        Err(arc) => {
            // If we can't unwrap, just lock and clone the data
            println!("Note: Could not unwrap metrics (still has references), cloning data instead");
            arc.lock().await.clone()
        }
    };

    // Merge in dialog metrics
    let dialog_metrics = match Arc::try_unwrap(metrics_for_events) {
        Ok(mutex) => mutex.into_inner(),
        Err(arc) => arc.lock().await.clone(),
    };

    // Combine the metrics
    let mut combined_metrics = final_metrics;
    combined_metrics.dialog_creation_count = dialog_metrics.dialog_creation_count;
    combined_metrics.dialog_creation_total_ns = dialog_metrics.dialog_creation_total_ns;

    combined_metrics
}

#[tokio::main]
async fn main() {
    // Setup tracing
    tracing_subscriber::fmt::init();
    
    println!("Starting session-core benchmark...");
    
    // Custom benchmark configuration
    let config = BenchmarkConfig {
        session_count: 10_000,   // Benchmark with 10,000 concurrent sessions
        duration: Duration::from_secs(30),
        max_dialogs_per_session: 2,
        termination_percentage: 25,
        cleanup_interval: Duration::from_secs(3),
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