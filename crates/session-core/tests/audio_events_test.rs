//! Test audio event system in session-core
//!
//! This test verifies that:
//! 1. Audio events can be published and received
//! 2. Audio event helper methods work correctly
//! 3. Audio events contain the correct data
//! 4. Event system handles audio events properly

use rvoip_session_core::manager::events::{SessionEvent, SessionEventProcessor, MediaFlowDirection};
use rvoip_session_core::api::types::{SessionId, AudioFrame, AudioStreamConfig};
use tokio::time::{timeout, Duration};

#[tokio::test]
async fn test_audio_frame_received_event() {
    // Create event processor and start it
    let processor = SessionEventProcessor::new();
    processor.start().await.unwrap();
    
    // Subscribe to events
    let mut subscriber = processor.subscribe().await.unwrap();
    
    // Create test data
    let session_id = SessionId::new();
    let audio_frame = AudioFrame::new(vec![1, 2, 3, 4], 8000, 1, 100);
    let stream_id = Some("stream-1".to_string());
    
    // Publish audio frame received event
    processor.publish_audio_frame_received(
        session_id.clone(),
        audio_frame.clone(),
        stream_id.clone(),
    ).await.unwrap();
    
    // Receive the event
    let event = timeout(Duration::from_millis(100), subscriber.receive())
        .await
        .expect("Timeout waiting for event")
        .expect("Failed to receive event");
    
    // Verify the event
    match event {
        SessionEvent::AudioFrameReceived { session_id: recv_session_id, audio_frame: recv_frame, stream_id: recv_stream_id } => {
            assert_eq!(recv_session_id, session_id);
            assert_eq!(recv_frame.samples, audio_frame.samples);
            assert_eq!(recv_frame.sample_rate, audio_frame.sample_rate);
            assert_eq!(recv_frame.channels, audio_frame.channels);
            assert_eq!(recv_frame.timestamp, audio_frame.timestamp);
            assert_eq!(recv_stream_id, stream_id);
        }
        _ => panic!("Expected AudioFrameReceived event"),
    }
    
    processor.stop().await.unwrap();
    println!("✅ AudioFrameReceived event works correctly");
}

#[tokio::test]
async fn test_audio_frame_requested_event() {
    // Create event processor and start it
    let processor = SessionEventProcessor::new();
    processor.start().await.unwrap();
    
    // Subscribe to events
    let mut subscriber = processor.subscribe().await.unwrap();
    
    // Create test data
    let session_id = SessionId::new();
    let config = AudioStreamConfig::wideband();
    let stream_id = Some("stream-2".to_string());
    
    // Publish audio frame requested event
    processor.publish_audio_frame_requested(
        session_id.clone(),
        config.clone(),
        stream_id.clone(),
    ).await.unwrap();
    
    // Receive the event
    let event = timeout(Duration::from_millis(100), subscriber.receive())
        .await
        .expect("Timeout waiting for event")
        .expect("Failed to receive event");
    
    // Verify the event
    match event {
        SessionEvent::AudioFrameRequested { session_id: recv_session_id, config: recv_config, stream_id: recv_stream_id } => {
            assert_eq!(recv_session_id, session_id);
            assert_eq!(recv_config.sample_rate, config.sample_rate);
            assert_eq!(recv_config.channels, config.channels);
            assert_eq!(recv_config.codec, config.codec);
            assert_eq!(recv_stream_id, stream_id);
        }
        _ => panic!("Expected AudioFrameRequested event"),
    }
    
    processor.stop().await.unwrap();
    println!("✅ AudioFrameRequested event works correctly");
}

#[tokio::test]
async fn test_audio_stream_config_changed_event() {
    // Create event processor and start it
    let processor = SessionEventProcessor::new();
    processor.start().await.unwrap();
    
    // Subscribe to events
    let mut subscriber = processor.subscribe().await.unwrap();
    
    // Create test data
    let session_id = SessionId::new();
    let old_config = AudioStreamConfig::telephony();
    let new_config = AudioStreamConfig::wideband();
    let stream_id = Some("stream-3".to_string());
    
    // Publish audio stream config changed event
    processor.publish_audio_stream_config_changed(
        session_id.clone(),
        old_config.clone(),
        new_config.clone(),
        stream_id.clone(),
    ).await.unwrap();
    
    // Receive the event
    let event = timeout(Duration::from_millis(100), subscriber.receive())
        .await
        .expect("Timeout waiting for event")
        .expect("Failed to receive event");
    
    // Verify the event
    match event {
        SessionEvent::AudioStreamConfigChanged { 
            session_id: recv_session_id, 
            old_config: recv_old_config, 
            new_config: recv_new_config,
            stream_id: recv_stream_id 
        } => {
            assert_eq!(recv_session_id, session_id);
            assert_eq!(recv_old_config.sample_rate, old_config.sample_rate);
            assert_eq!(recv_old_config.codec, old_config.codec);
            assert_eq!(recv_new_config.sample_rate, new_config.sample_rate);
            assert_eq!(recv_new_config.codec, new_config.codec);
            assert_eq!(recv_stream_id, stream_id);
        }
        _ => panic!("Expected AudioStreamConfigChanged event"),
    }
    
    processor.stop().await.unwrap();
    println!("✅ AudioStreamConfigChanged event works correctly");
}

#[tokio::test]
async fn test_audio_stream_lifecycle_events() {
    // Create event processor and start it
    let processor = SessionEventProcessor::new();
    processor.start().await.unwrap();
    
    // Subscribe to events
    let mut subscriber = processor.subscribe().await.unwrap();
    
    // Create test data
    let session_id = SessionId::new();
    let config = AudioStreamConfig::high_quality();
    let stream_id = "stream-4".to_string();
    let direction = MediaFlowDirection::Both;
    let stop_reason = "Call ended".to_string();
    
    // Publish audio stream started event
    processor.publish_audio_stream_started(
        session_id.clone(),
        config.clone(),
        stream_id.clone(),
        direction,
    ).await.unwrap();
    
    // Receive the started event
    let start_event = timeout(Duration::from_millis(100), subscriber.receive())
        .await
        .expect("Timeout waiting for start event")
        .expect("Failed to receive start event");
    
    // Verify the start event
    match start_event {
        SessionEvent::AudioStreamStarted { 
            session_id: recv_session_id, 
            config: recv_config, 
            stream_id: recv_stream_id,
            direction: recv_direction 
        } => {
            assert_eq!(recv_session_id, session_id);
            assert_eq!(recv_config.sample_rate, config.sample_rate);
            assert_eq!(recv_config.channels, config.channels);
            assert_eq!(recv_config.codec, config.codec);
            assert_eq!(recv_stream_id, stream_id);
            assert_eq!(recv_direction, direction);
        }
        _ => panic!("Expected AudioStreamStarted event"),
    }
    
    // Publish audio stream stopped event
    processor.publish_audio_stream_stopped(
        session_id.clone(),
        stream_id.clone(),
        stop_reason.clone(),
    ).await.unwrap();
    
    // Receive the stopped event
    let stop_event = timeout(Duration::from_millis(100), subscriber.receive())
        .await
        .expect("Timeout waiting for stop event")
        .expect("Failed to receive stop event");
    
    // Verify the stop event
    match stop_event {
        SessionEvent::AudioStreamStopped { 
            session_id: recv_session_id, 
            stream_id: recv_stream_id,
            reason: recv_reason 
        } => {
            assert_eq!(recv_session_id, session_id);
            assert_eq!(recv_stream_id, stream_id);
            assert_eq!(recv_reason, stop_reason);
        }
        _ => panic!("Expected AudioStreamStopped event"),
    }
    
    processor.stop().await.unwrap();
    println!("✅ Audio stream lifecycle events work correctly");
}

#[tokio::test]
async fn test_multiple_audio_events() {
    // Test that multiple audio events can be published and received in order
    let processor = SessionEventProcessor::new();
    processor.start().await.unwrap();
    
    let mut subscriber = processor.subscribe().await.unwrap();
    
    let session_id = SessionId::new();
    let config = AudioStreamConfig::telephony();
    let stream_id = "multi-stream".to_string();
    
    // Publish multiple events
    processor.publish_audio_stream_started(
        session_id.clone(),
        config.clone(),
        stream_id.clone(),
        MediaFlowDirection::Both,
    ).await.unwrap();
    
    let audio_frame = AudioFrame::new(vec![100, 200], 8000, 1, 200);
    processor.publish_audio_frame_received(
        session_id.clone(),
        audio_frame.clone(),
        Some(stream_id.clone()),
    ).await.unwrap();
    
    processor.publish_audio_frame_requested(
        session_id.clone(),
        config.clone(),
        Some(stream_id.clone()),
    ).await.unwrap();
    
    processor.publish_audio_stream_stopped(
        session_id.clone(),
        stream_id.clone(),
        "Test complete".to_string(),
    ).await.unwrap();
    
    // Receive all events and verify order
    let events = vec![
        timeout(Duration::from_millis(100), subscriber.receive()).await.unwrap().unwrap(),
        timeout(Duration::from_millis(100), subscriber.receive()).await.unwrap().unwrap(),
        timeout(Duration::from_millis(100), subscriber.receive()).await.unwrap().unwrap(),
        timeout(Duration::from_millis(100), subscriber.receive()).await.unwrap().unwrap(),
    ];
    
    // Verify event types in order
    assert!(matches!(events[0], SessionEvent::AudioStreamStarted { .. }));
    assert!(matches!(events[1], SessionEvent::AudioFrameReceived { .. }));
    assert!(matches!(events[2], SessionEvent::AudioFrameRequested { .. }));
    assert!(matches!(events[3], SessionEvent::AudioStreamStopped { .. }));
    
    processor.stop().await.unwrap();
    println!("✅ Multiple audio events work correctly");
}

#[tokio::test]
async fn test_audio_events_with_no_stream_id() {
    // Test audio events work correctly when stream_id is None
    let processor = SessionEventProcessor::new();
    processor.start().await.unwrap();
    
    let mut subscriber = processor.subscribe().await.unwrap();
    
    let session_id = SessionId::new();
    let audio_frame = AudioFrame::new(vec![50, 100, 150], 16000, 1, 300);
    let config = AudioStreamConfig::wideband();
    
    // Publish events with no stream_id
    processor.publish_audio_frame_received(
        session_id.clone(),
        audio_frame.clone(),
        None,
    ).await.unwrap();
    
    processor.publish_audio_frame_requested(
        session_id.clone(),
        config.clone(),
        None,
    ).await.unwrap();
    
    processor.publish_audio_stream_config_changed(
        session_id.clone(),
        AudioStreamConfig::telephony(),
        config.clone(),
        None,
    ).await.unwrap();
    
    // Receive and verify events
    let events = vec![
        timeout(Duration::from_millis(100), subscriber.receive()).await.unwrap().unwrap(),
        timeout(Duration::from_millis(100), subscriber.receive()).await.unwrap().unwrap(),
        timeout(Duration::from_millis(100), subscriber.receive()).await.unwrap().unwrap(),
    ];
    
    // Verify all events have None for stream_id
    match &events[0] {
        SessionEvent::AudioFrameReceived { stream_id, .. } => assert_eq!(*stream_id, None),
        _ => panic!("Expected AudioFrameReceived"),
    }
    
    match &events[1] {
        SessionEvent::AudioFrameRequested { stream_id, .. } => assert_eq!(*stream_id, None),
        _ => panic!("Expected AudioFrameRequested"),
    }
    
    match &events[2] {
        SessionEvent::AudioStreamConfigChanged { stream_id, .. } => assert_eq!(*stream_id, None),
        _ => panic!("Expected AudioStreamConfigChanged"),
    }
    
    processor.stop().await.unwrap();
    println!("✅ Audio events with no stream_id work correctly");
}

#[tokio::test]
async fn test_audio_events_serialization() {
    // Test that audio events can be serialized and deserialized
    let session_id = SessionId::new();
    let audio_frame = AudioFrame::new(vec![1, 2, 3], 8000, 1, 400);
    let config = AudioStreamConfig::telephony();
    
    // Test AudioFrameReceived event
    let frame_event = SessionEvent::AudioFrameReceived {
        session_id: session_id.clone(),
        audio_frame: audio_frame.clone(),
        stream_id: Some("test-stream".to_string()),
    };
    
    let serialized = serde_json::to_string(&frame_event).expect("Failed to serialize");
    let deserialized: SessionEvent = serde_json::from_str(&serialized).expect("Failed to deserialize");
    
    match deserialized {
        SessionEvent::AudioFrameReceived { audio_frame: deser_frame, .. } => {
            assert_eq!(deser_frame.samples, audio_frame.samples);
            assert_eq!(deser_frame.sample_rate, audio_frame.sample_rate);
        }
        _ => panic!("Deserialized wrong event type"),
    }
    
    // Test AudioStreamStarted event
    let start_event = SessionEvent::AudioStreamStarted {
        session_id: session_id.clone(),
        config: config.clone(),
        stream_id: "test-stream".to_string(),
        direction: MediaFlowDirection::Send,
    };
    
    let serialized = serde_json::to_string(&start_event).expect("Failed to serialize");
    let deserialized: SessionEvent = serde_json::from_str(&serialized).expect("Failed to deserialize");
    
    match deserialized {
        SessionEvent::AudioStreamStarted { config: deser_config, direction, .. } => {
            assert_eq!(deser_config.sample_rate, config.sample_rate);
            assert_eq!(deser_config.codec, config.codec);
            assert_eq!(direction, MediaFlowDirection::Send);
        }
        _ => panic!("Deserialized wrong event type"),
    }
    
    println!("✅ Audio events serialization works correctly");
}

#[tokio::test]
async fn test_audio_event_processor_lifecycle() {
    // Test that audio events work correctly through processor start/stop cycles
    let processor = SessionEventProcessor::new();
    
    // Verify processor starts and stops cleanly
    assert!(!processor.is_running().await);
    
    processor.start().await.unwrap();
    assert!(processor.is_running().await);
    
    // Test publishing events while running
    let session_id = SessionId::new();
    let config = AudioStreamConfig::telephony();
    
    processor.publish_audio_stream_started(
        session_id.clone(),
        config.clone(),
        "lifecycle-test".to_string(),
        MediaFlowDirection::Receive,
    ).await.unwrap();
    
    processor.stop().await.unwrap();
    assert!(!processor.is_running().await);
    
    // Test that events are dropped when processor is stopped
    processor.publish_audio_stream_stopped(
        session_id,
        "lifecycle-test".to_string(),
        "Processor stopped".to_string(),
    ).await.unwrap(); // Should not panic, but event will be dropped
    
    println!("✅ Audio event processor lifecycle works correctly");
}

#[tokio::test]
async fn test_realistic_audio_streaming_scenario() {
    // Test a realistic scenario of audio streaming events
    let processor = SessionEventProcessor::new();
    processor.start().await.unwrap();
    
    let mut subscriber = processor.subscribe().await.unwrap();
    
    let session_id = SessionId::new();
    let mut config = AudioStreamConfig::telephony();
    let stream_id = "realistic-stream".to_string();
    
    // 1. Start audio stream
    processor.publish_audio_stream_started(
        session_id.clone(),
        config.clone(),
        stream_id.clone(),
        MediaFlowDirection::Both,
    ).await.unwrap();
    
    // 2. Simulate receiving several audio frames
    for i in 0..3 {
        let samples = vec![i, i+1, i+2, i+3];
        let frame = AudioFrame::new(samples, 8000, 1, (i * 160) as u32);
        processor.publish_audio_frame_received(
            session_id.clone(),
            frame,
            Some(stream_id.clone()),
        ).await.unwrap();
    }
    
    // 3. Request audio frames for sending
    for _ in 0..2 {
        processor.publish_audio_frame_requested(
            session_id.clone(),
            config.clone(),
            Some(stream_id.clone()),
        ).await.unwrap();
    }
    
    // 4. Change configuration (upgrade quality)
    let old_config = config.clone();
    config = AudioStreamConfig::wideband();
    processor.publish_audio_stream_config_changed(
        session_id.clone(),
        old_config,
        config.clone(),
        Some(stream_id.clone()),
    ).await.unwrap();
    
    // 5. Stop the stream
    processor.publish_audio_stream_stopped(
        session_id.clone(),
        stream_id.clone(),
        "Call ended".to_string(),
    ).await.unwrap();
    
    // Receive all events (1 start + 3 received + 2 requested + 1 config change + 1 stop = 8 events)
    let mut events = Vec::new();
    for _ in 0..8 {
        let event = timeout(Duration::from_millis(100), subscriber.receive())
            .await
            .expect("Timeout waiting for event")
            .expect("Failed to receive event");
        events.push(event);
    }
    
    // Verify event sequence
    assert!(matches!(events[0], SessionEvent::AudioStreamStarted { .. }));
    assert!(matches!(events[1], SessionEvent::AudioFrameReceived { .. }));
    assert!(matches!(events[2], SessionEvent::AudioFrameReceived { .. }));
    assert!(matches!(events[3], SessionEvent::AudioFrameReceived { .. }));
    assert!(matches!(events[4], SessionEvent::AudioFrameRequested { .. }));
    assert!(matches!(events[5], SessionEvent::AudioFrameRequested { .. }));
    assert!(matches!(events[6], SessionEvent::AudioStreamConfigChanged { .. }));
    assert!(matches!(events[7], SessionEvent::AudioStreamStopped { .. }));
    
    processor.stop().await.unwrap();
    println!("✅ Realistic audio streaming scenario works correctly");
} 