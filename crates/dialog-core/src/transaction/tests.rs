use super::*;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::sync::RwLock;
use crate::transaction::runner::{AsRefState, AsRefKey, HasTransactionEvents, HasTransport, HasCommandSender};
use crate::transaction::logic::TransactionLogic;
use rvoip_sip_transport::{Transport, Error as TransportError};
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};

#[tokio::test]
async fn test_graceful_shutdown_when_receiver_dropped() {
    // Test that the transaction runner gracefully shuts down when the event receiver is dropped
    
    // Setup mock components
    let state = Arc::new(AtomicTransactionState::new());
    let key = TransactionKey::new("test-branch".to_string(), Method::Invite, false);
    let addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
    
    // Create a mock transport
    let mock_transport = Arc::new(MockTransport::new());
    
    // Create a channel that we'll drop to simulate shutdown
    let (event_tx, event_rx) = mpsc::channel::<TransactionEvent>(10);
    
    // Create a command channel
    let (cmd_tx, cmd_rx) = mpsc::channel(10);
    
    // Create transaction data
    let data = Arc::new(MockData {
        state,
        key,
        events_tx: event_tx,
        transport: mock_transport,
        cmd_tx: cmd_tx.clone(),
        shutdown_handled: Arc::new(AtomicBool::new(false)),
    });
    
    // Create mock transaction logic
    let logic = Arc::new(MockLogic::new());
    
    // First test: Without RVOIP_TEST environment variable set
    {
        // Make sure RVOIP_TEST is not set
        std::env::remove_var("RVOIP_TEST");
        
        let data_clone = data.clone();
        let logic_clone = logic.clone();
        
        // Reset shutdown flag
        data.shutdown_handled.store(false, Ordering::SeqCst);
        
        // Spawn transaction runner in a separate task
        let runner_task = tokio::spawn(async move {
            // Drop event_rx here to simulate the receiver being dropped
            drop(event_rx);
            
            // Run the transaction loop
            super::runner::run_transaction_loop(data_clone, logic_clone, cmd_rx).await;
        });
        
        // Send a state transition command to trigger shutdown handling
        cmd_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Trying)).await.unwrap();
        
        // Wait for runner to exit
        tokio::time::timeout(Duration::from_secs(1), runner_task).await
            .expect("Transaction runner should exit within 1 second")
            .expect("Transaction runner should exit cleanly");
        
        // Check that shutdown was handled gracefully
        assert!(data.shutdown_handled.load(Ordering::SeqCst), 
            "The transaction should have handled shutdown gracefully without RVOIP_TEST set");
    }
    
    // Second test: With RVOIP_TEST environment variable set
    {
        // Set RVOIP_TEST environment variable
        std::env::set_var("RVOIP_TEST", "1");
        
        // Create a new command channel and event channel
        let (new_event_tx, new_event_rx) = mpsc::channel::<TransactionEvent>(10);
        let (new_cmd_tx, new_cmd_rx) = mpsc::channel(10);
        
        // Create new transaction data
        let test_data = Arc::new(MockData {
            state: Arc::new(AtomicTransactionState::new()),
            key: key.clone(),
            events_tx: new_event_tx,
            transport: mock_transport.clone(),
            cmd_tx: new_cmd_tx.clone(),
            shutdown_handled: Arc::new(AtomicBool::new(false)),
        });
        
        // Spawn transaction runner with RVOIP_TEST set
        let runner_task = tokio::spawn({
            let test_data = test_data.clone();
            let logic_clone = logic.clone();
            async move {
                // Drop event receiver to simulate closed channel
                drop(new_event_rx);
                
                // Run the transaction loop with test env var set
                super::runner::run_transaction_loop(test_data, logic_clone, new_cmd_rx).await;
            }
        });
        
        // Send a command to trigger the processing of a dropped channel
        new_cmd_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Trying)).await.unwrap();
        
        // Wait a bit to ensure command is processed
        tokio::time::sleep(Duration::from_millis(100)).await;
        
        // Send another command - this should still work if runner didn't terminate
        let send_result = new_cmd_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Proceeding)).await;
        assert!(send_result.is_ok(), "Should be able to send commands with RVOIP_TEST set even with closed event channel");
        
        // Send a terminate command to end the test
        new_cmd_tx.send(InternalTransactionCommand::Terminate).await.unwrap();
        
        // Wait for the runner to exit
        tokio::time::timeout(Duration::from_secs(1), runner_task).await
            .expect("Transaction runner should exit within 1 second")
            .expect("Transaction runner should exit cleanly");
        
        // Clean up
        std::env::remove_var("RVOIP_TEST");
    }
}

// Mock implementations needed for the test
#[derive(Debug)]
struct MockData {
    state: Arc<AtomicTransactionState>,
    key: TransactionKey,
    events_tx: mpsc::Sender<TransactionEvent>,
    transport: Arc<MockTransport>,
    cmd_tx: mpsc::Sender<InternalTransactionCommand>,
    shutdown_handled: Arc<AtomicBool>,
}

impl AsRefState for MockData {
    fn as_ref_state(&self) -> &Arc<AtomicTransactionState> {
        &self.state
    }
}

impl AsRefKey for MockData {
    fn as_ref_key(&self) -> &TransactionKey {
        &self.key
    }
}

impl HasTransactionEvents for MockData {
    fn get_tu_event_sender(&self) -> mpsc::Sender<TransactionEvent> {
        self.events_tx.clone()
    }
}

impl HasTransport for MockData {
    fn get_transport_layer(&self) -> Arc<dyn Transport> {
        self.transport.clone()
    }
}

impl HasCommandSender for MockData {
    fn get_self_command_sender(&self) -> mpsc::Sender<InternalTransactionCommand> {
        self.cmd_tx.clone()
    }
}

#[derive(Debug)]
struct MockTransport {
    is_closed: Arc<AtomicBool>,
}

impl MockTransport {
    fn new() -> Self {
        Self {
            is_closed: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[async_trait::async_trait]
impl Transport for MockTransport {
    fn local_addr(&self) -> std::result::Result<SocketAddr, TransportError> {
        Ok(SocketAddr::from_str("127.0.0.1:5060").unwrap())
    }
    
    async fn send_message(&self, _message: rvoip_sip_core::Message, _destination: SocketAddr) -> std::result::Result<(), TransportError> {
        Ok(())
    }
    
    async fn close(&self) -> std::result::Result<(), TransportError> {
        self.is_closed.store(true, Ordering::SeqCst);
        Ok(())
    }
    
    fn is_closed(&self) -> bool {
        self.is_closed.load(Ordering::SeqCst)
    }
}

// Simple empty implementation of timer handle for tests
#[derive(Debug, Default)]
struct MockTimerHandles { }

// Simple transaction logic implementation
#[derive(Debug)]
struct MockLogic { 
    timer_cancel_called: Arc<AtomicBool>,
}

impl MockLogic {
    fn new() -> Self {
        Self {
            timer_cancel_called: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[async_trait::async_trait]
impl TransactionLogic<MockData, MockTimerHandles> for MockLogic {
    fn kind(&self) -> TransactionKind {
        TransactionKind::ClientInvite
    }
    
    async fn process_message(&self, _data: &MockData, _message: Message, _current_state: TransactionState) -> super::Result<Option<TransactionState>> {
        Ok(None)
    }
    
    async fn handle_timer(&self, _data: &MockData, _timer_name: &str, _current_state: TransactionState, _timer_handles: &mut MockTimerHandles) -> super::Result<Option<TransactionState>> {
        Ok(None)
    }
    
    async fn on_enter_state(
        &self,
        data: &MockData,
        _new_state: TransactionState,
        _previous_state: TransactionState,
        _timer_handles: &mut MockTimerHandles,
        _cmd_sender: mpsc::Sender<InternalTransactionCommand>,
    ) -> super::Result<()> {
        // Mark shutdown as handled when we enter Terminated state
        if _new_state == TransactionState::Terminated {
            data.shutdown_handled.store(true, Ordering::SeqCst);
        }
        Ok(())
    }
    
    fn cancel_all_specific_timers(&self, _timer_handles: &mut MockTimerHandles) {
        self.timer_cancel_called.store(true, Ordering::SeqCst);
    }
} 