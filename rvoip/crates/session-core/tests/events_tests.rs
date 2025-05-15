use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use async_trait::async_trait;
use tokio::time::sleep;
use std::time::Duration;

use rvoip_session_core::{
    events::{EventBus, EventHandler, SessionEvent},
    session::{SessionId, SessionState},
    dialog::DialogId,
};

// Create a test event handler that counts events
struct TestEventHandler {
    event_count: AtomicUsize,
    created_count: AtomicUsize,
    state_changed_count: AtomicUsize,
    terminated_count: AtomicUsize,
}

impl TestEventHandler {
    fn new() -> Self {
        Self {
            event_count: AtomicUsize::new(0),
            created_count: AtomicUsize::new(0),
            state_changed_count: AtomicUsize::new(0),
            terminated_count: AtomicUsize::new(0),
        }
    }
    
    fn event_count(&self) -> usize {
        self.event_count.load(Ordering::SeqCst)
    }
    
    fn created_count(&self) -> usize {
        self.created_count.load(Ordering::SeqCst)
    }
    
    fn state_changed_count(&self) -> usize {
        self.state_changed_count.load(Ordering::SeqCst)
    }
    
    fn terminated_count(&self) -> usize {
        self.terminated_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl EventHandler for TestEventHandler {
    async fn handle_event(&self, event: SessionEvent) {
        self.event_count.fetch_add(1, Ordering::SeqCst);
        
        match event {
            SessionEvent::Created { .. } => {
                self.created_count.fetch_add(1, Ordering::SeqCst);
            },
            SessionEvent::StateChanged { .. } => {
                self.state_changed_count.fetch_add(1, Ordering::SeqCst);
            },
            SessionEvent::Terminated { .. } => {
                self.terminated_count.fetch_add(1, Ordering::SeqCst);
            },
            _ => {}
        }
    }
}

#[tokio::test]
async fn test_event_bus_publish_subscribe() {
    // Create event bus
    let event_bus = EventBus::new(10);
    
    // Subscribe to events
    let mut rx = event_bus.subscribe();
    
    // Create a session ID for testing
    let session_id = SessionId::new();
    
    // Publish an event
    event_bus.publish(SessionEvent::Created { 
        session_id: session_id.clone() 
    });
    
    // Try to receive the event with a timeout
    let received = tokio::time::timeout(
        Duration::from_millis(100),
        rx.recv()
    ).await;
    
    // Check if we received the event
    assert!(received.is_ok(), "Failed to receive event within timeout");
    
    if let Ok(event_result) = received {
        match event_result {
            Ok(SessionEvent::Created { session_id: event_session_id }) => {
                assert_eq!(event_session_id, session_id);
            },
            _ => panic!("Received wrong event type"),
        }
    }
}

#[tokio::test]
async fn test_event_handler_registration() {
    // Create event bus
    let event_bus = EventBus::new(10);
    
    // Create and register handler
    let handler = Arc::new(TestEventHandler::new());
    event_bus.register_handler(handler.clone()).await;
    
    // Create a session ID for testing
    let session_id = SessionId::new();
    let dialog_id = DialogId::new();
    
    // Publish several events
    event_bus.publish(SessionEvent::Created { 
        session_id: session_id.clone() 
    });
    
    event_bus.publish(SessionEvent::StateChanged { 
        session_id: session_id.clone(),
        old_state: SessionState::Initializing,
        new_state: SessionState::Dialing,
    });
    
    event_bus.publish(SessionEvent::DialogUpdated { 
        session_id: session_id.clone(),
        dialog_id,
    });
    
    event_bus.publish(SessionEvent::Terminated { 
        session_id: session_id.clone(),
        reason: "Test termination".to_string(),
    });
    
    // Wait a bit for all events to be processed
    sleep(Duration::from_millis(50)).await;
    
    // Check the counts
    assert_eq!(handler.event_count(), 4);
    assert_eq!(handler.created_count(), 1);
    assert_eq!(handler.state_changed_count(), 1);
    assert_eq!(handler.terminated_count(), 1);
}

#[tokio::test]
async fn test_multiple_subscribers() {
    // Create event bus
    let event_bus = EventBus::new(10);
    
    // Create two subscribers
    let mut rx1 = event_bus.subscribe();
    let mut rx2 = event_bus.subscribe();
    
    // Create a session ID for testing
    let session_id = SessionId::new();
    
    // Publish an event
    event_bus.publish(SessionEvent::Created { 
        session_id: session_id.clone() 
    });
    
    // Try to receive the event on both subscribers
    let received1 = tokio::time::timeout(
        Duration::from_millis(100),
        rx1.recv()
    ).await;
    
    let received2 = tokio::time::timeout(
        Duration::from_millis(100),
        rx2.recv()
    ).await;
    
    // Check if both received the event
    assert!(received1.is_ok(), "Subscriber 1 failed to receive event within timeout");
    assert!(received2.is_ok(), "Subscriber 2 failed to receive event within timeout");
}

#[tokio::test]
async fn test_event_bus_drop_slow_receiver() {
    // Create event bus with small capacity
    let event_bus = EventBus::new(2);
    
    // Create a subscriber but don't read from it
    let _rx = event_bus.subscribe();
    
    // Create a session ID for testing
    let session_id = SessionId::new();
    
    // Publish multiple events (more than the capacity)
    for _ in 0..5 {
        event_bus.publish(SessionEvent::Created { 
            session_id: session_id.clone() 
        });
    }
    
    // This should not block or crash, as the broadcast channel will drop old messages
    // for slow receivers. Just verify we can still publish events.
    event_bus.publish(SessionEvent::Terminated { 
        session_id: session_id.clone(),
        reason: "Final test event".to_string(),
    });
    
    // Test passes if we get here without blocking
}

#[tokio::test]
async fn test_event_bus_default() {
    // Test the default constructor
    let event_bus = EventBus::default();
    
    // Subscribe to events
    let mut rx = event_bus.subscribe();
    
    // Create a session ID for testing
    let session_id = SessionId::new();
    
    // Publish an event
    event_bus.publish(SessionEvent::Created { 
        session_id: session_id.clone() 
    });
    
    // Verify we can receive it
    let received = tokio::time::timeout(
        Duration::from_millis(100),
        rx.recv()
    ).await;
    
    assert!(received.is_ok(), "Failed to receive event within timeout");
} 