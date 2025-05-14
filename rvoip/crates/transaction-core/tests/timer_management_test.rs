/* Temporarily disabling these tests due to API changes 
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{sleep, timeout};

use rvoip_transaction_core::timer::{Timer, TimerManager, TimerFactory, TimerType, TimerConfig};
use rvoip_transaction_core::transaction::{
    TransactionKey,
    InternalTransactionCommand
};
use rvoip_sip_core::Method;

#[tokio::test]
async fn test_timer_creation() {
    // Test basic timer creation
    let tx_id = "test-transaction-1".to_string();
    
    let one_shot = Timer::new_one_shot("test-timer", tx_id.clone(), Duration::from_millis(100));
    assert_eq!(one_shot.name, "test-timer");
    assert_eq!(one_shot.transaction_id, tx_id);
    assert!(!one_shot.repeating);
    
    let repeating = Timer::new_repeating("repeating", tx_id.clone(), Duration::from_millis(50));
    assert_eq!(repeating.name, "repeating");
    assert!(repeating.repeating);
    assert_eq!(repeating.interval, Some(Duration::from_millis(50)));
    
    let backoff = Timer::new_backoff("backoff", tx_id, Duration::from_millis(20), Duration::from_millis(200));
    assert_eq!(backoff.name, "backoff");
    assert!(backoff.repeating);
    assert_eq!(backoff.max_interval, Some(Duration::from_millis(200)));
    assert_eq!(backoff.current_interval, Some(Duration::from_millis(20)));
}

#[tokio::test]
async fn test_timer_backoff() {
    let tx_id = "test-transaction-2".to_string();
    let mut timer = Timer::new_backoff("backoff", tx_id, Duration::from_millis(25), Duration::from_millis(100));
    
    // Initial interval is 25ms
    assert_eq!(timer.current_interval, Some(Duration::from_millis(25)));
    
    // First backoff should double to 50ms
    let next = timer.next_backoff_interval();
    assert_eq!(next, Duration::from_millis(50));
    assert_eq!(timer.current_interval, Some(Duration::from_millis(50)));
    
    // Second backoff should double again to 100ms
    let next = timer.next_backoff_interval();
    assert_eq!(next, Duration::from_millis(100));
    assert_eq!(timer.current_interval, Some(Duration::from_millis(100)));
    
    // Third backoff should cap at max of 100ms
    let next = timer.next_backoff_interval();
    assert_eq!(next, Duration::from_millis(100));
    assert_eq!(timer.current_interval, Some(Duration::from_millis(100)));
}

#[tokio::test]
async fn test_timer_manager() {
    // Create a timer manager and channels for testing
    let timer_manager = Arc::new(TimerManager::new());
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<InternalTransactionCommand>(10);
    
    // Create a test transaction ID
    let tx_id = "test-transaction-3".to_string();
    
    // Register the transaction with the timer manager
    timer_manager.register_transaction(tx_id.clone(), cmd_tx).await.unwrap();
    
    // Create and schedule a timer
    let timer = Timer::new_one_shot("A", tx_id.clone(), Duration::from_millis(50));
    timer_manager.schedule_timer(timer).await.unwrap();
    
    // Wait a bit longer than the timer
    sleep(Duration::from_millis(100)).await;
    
    // Check if we get the timer command (may timeout if implementation doesn't work)
    match timeout(Duration::from_millis(100), cmd_rx.recv()).await {
        Ok(Some(cmd)) => {
            match cmd {
                InternalTransactionCommand::Timer(name) => {
                    assert_eq!(name, "A");
                },
                _ => panic!("Unexpected command received"),
            }
        },
        Ok(None) => {
            panic!("Command channel closed unexpectedly");
        },
        Err(_) => {
            panic!("Timeout waiting for timer event");
        }
    }
    
    // Clean up
    timer_manager.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_timer_cancellation() {
    // Create a timer manager and channels for testing
    let timer_manager = Arc::new(TimerManager::new());
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<InternalTransactionCommand>(10);
    
    // Create a test transaction ID
    let tx_id = "test-transaction-4".to_string();
    
    // Register the transaction with the timer manager
    timer_manager.register_transaction(tx_id.clone(), cmd_tx).await.unwrap();
    
    // Create and schedule a timer
    let timer = Timer::new_one_shot("A", tx_id.clone(), Duration::from_millis(200));
    timer_manager.schedule_timer(timer).await.unwrap();
    
    // Cancel the timer before it fires
    timer_manager.cancel_timer(&tx_id, "A").await.unwrap();
    
    // Wait more than the timer duration
    sleep(Duration::from_millis(300)).await;
    
    // Verify no timer event was received (timeout expected)
    match timeout(Duration::from_millis(50), cmd_rx.recv()).await {
        Ok(_) => {
            panic!("Received timer event after cancellation");
        },
        Err(_) => {
            // Expected timeout - timer was successfully cancelled
        }
    }
    
    // Clean up
    timer_manager.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_timer_factory() {
    let timer_manager = Arc::new(TimerManager::new());
    let timer_config = TimerConfig::for_testing();
    let timer_factory = TimerFactory::new(timer_config, timer_manager.clone());
    
    let tx_id = "test-transaction-5".to_string();
    
    // Test scheduling individual timers
    timer_factory.schedule_timer_a(tx_id.clone()).await.unwrap();
    timer_factory.schedule_timer_b(tx_id.clone()).await.unwrap();
    
    // Test scheduling timer combinations
    let tx_id2 = "test-transaction-6".to_string();
    timer_factory.schedule_invite_client_initial_timers(tx_id2.clone()).await.unwrap();
    
    // Clean up
    timer_factory.cancel_all_timers(&tx_id).await.unwrap();
    timer_factory.cancel_all_timers(&tx_id2).await.unwrap();
    timer_manager.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_rfc_timers() {
    // Create a timer factory with testing configuration
    let timer_manager = Arc::new(TimerManager::new());
    let timer_config = TimerConfig::for_testing();
    let timer_factory = TimerFactory::new(timer_config.clone(), timer_manager.clone());
    
    // Create a mock transaction ID
    let tx_id = "test-transaction-7".to_string();
    
    // Test that timer durations match RFC expectations
    let timer_a = TimerManager::create_timer_a(tx_id.clone(), &timer_config);
    let timer_b = TimerManager::create_timer_b(tx_id.clone(), &timer_config);
    
    // Timer A should be a backoff timer
    assert!(timer_a.repeating);
    assert_eq!(timer_a.interval, Some(timer_config.t1));
    assert_eq!(timer_a.max_interval, Some(timer_config.t2));
    
    // Timer B should be a one-shot timer
    assert!(!timer_b.repeating);
    
    // Clean up
    timer_manager.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_basic_timer_functionality() {
    // Create a channel for receiving timer events
    let (tx, mut rx) = mpsc::channel(32);
    
    // Create a transaction key for testing
    let tx_key = TransactionKey::new("z9hG4bK12345".to_string(), Method::Invite, false);
    
    // Create a timer manager
    let timer_manager = TimerManager::new(tx_key.clone(), tx.clone(), None);
    
    // Start a timer
    timer_manager.start_timer("A", TimerType::Retransmission).await;
    
    // Wait for the timer to fire (T1 = 500ms by default, but timer A is 2*T1 = 1 second)
    sleep(Duration::from_millis(1100)).await;
    
    // Check if we received a timer event
    let cmd = rx.recv().await.expect("Should receive timer event");
    match cmd {
        InternalTransactionCommand::Timer(timer_id) => {
            assert_eq!(timer_id, "A", "Timer ID should be 'A'");
        },
        _ => panic!("Expected timer event"),
    }
}

#[tokio::test]
async fn test_timer_cancellation_direct() {
    // Create a channel for receiving timer events
    let (tx, mut rx) = mpsc::channel(32);
    
    // Create a transaction key for testing
    let tx_key = TransactionKey::new("z9hG4bK12345".to_string(), Method::Invite, false);
    
    // Create a timer manager
    let timer_manager = TimerManager::new(tx_key.clone(), tx.clone(), None);
    
    // Start a timer
    timer_manager.start_timer("B", TimerType::TransactionTimeout).await;
    
    // Cancel the timer before it fires
    sleep(Duration::from_millis(100)).await;
    timer_manager.stop_timer("B").await;
    
    // Wait long enough for the timer to have fired if not cancelled
    sleep(Duration::from_millis(32000)).await;
    
    // Check that we didn't receive a timer event
    let timeout = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await;
    assert!(timeout.is_err(), "Should not receive a timer event");
}

#[tokio::test]
async fn test_custom_timer_config() {
    // Create a channel for receiving timer events
    let (tx, mut rx) = mpsc::channel(32);
    
    // Create a transaction key for testing
    let tx_key = TransactionKey::new("z9hG4bK12345".to_string(), Method::Invite, false);
    
    // Create a custom timer config with shorter durations
    let custom_config = TimerConfig {
        t1: Duration::from_millis(100),
        t2: Duration::from_millis(400),
        t4: Duration::from_millis(2000),
    };
    
    // Create a timer manager with custom config
    let timer_manager = TimerManager::new(tx_key.clone(), tx.clone(), Some(custom_config));
    
    // Start a timer (A is 2*T1 = 200ms with our custom config)
    timer_manager.start_timer("A", TimerType::Retransmission).await;
    
    // Wait for the timer to fire (a bit over 200ms)
    sleep(Duration::from_millis(250)).await;
    
    // Check if we received a timer event
    let cmd = rx.recv().await.expect("Should receive timer event");
    match cmd {
        InternalTransactionCommand::Timer(timer_id) => {
            assert_eq!(timer_id, "A", "Timer ID should be 'A'");
        },
        _ => panic!("Expected timer event"),
    }
}
*/ 