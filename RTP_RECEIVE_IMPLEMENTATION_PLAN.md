# RTP Receive Path Implementation Plan

## Executive Summary

After 4 months of configuration fixes, the core issue is clear: **the RTP receive path is completely missing**. While the send path works perfectly (microphone → G.711 encode → RTP transmission), there is no implementation for the receive path (RTP reception → G.711 decode → speaker playback).

This document provides a comprehensive implementation plan to enable bidirectional voice conversation in the sip-client-p2p example.

## Root Cause Analysis

### ✅ Working Components
- **SIP Signaling**: Call setup, SDP negotiation, media flow establishment
- **RTP Transmission**: Microphone audio encoded and sent via UDP
- **Audio Infrastructure**: CPAL-based device management, audio pipelines
- **Codec Support**: G.711 μ-law/A-law encoding and decoding libraries

### ❌ Missing Components  
- **RTP Reception Processing**: Received packets are logged but never decoded
- **Audio Frame Routing**: No path from decoded audio to playback pipeline
- **Client Integration**: Missing `subscribe_to_audio_frames()` method in client-core

## Current Architecture Gap

```text
SEND PATH (Working):
Microphone → AudioPipeline → client.send_audio_frame() → G.711 Encode → RTP → Network

RECEIVE PATH (Missing):
Network → RTP ❌ NO DECODE ❌ NO ROUTING ❌ NO PLAYBACK → Speaker
```

## Comprehensive Implementation Plan

### Phase 1: RTP Payload Decoder (Critical Path)

#### 1.1 Create RTP Payload Decoder
**New File**: `session-core/src/media/rtp_decoder.rs`

```rust
use codec_core::{CodecRegistry, AudioFrame};
use std::collections::HashMap;
use tokio::sync::mpsc;

pub struct RtpPayloadDecoder {
    codec_registry: Arc<CodecRegistry>,
    audio_frame_senders: HashMap<SessionId, mpsc::Sender<AudioFrame>>,
}

impl RtpPayloadDecoder {
    pub async fn process_rtp_event(&mut self, event: RtpEvent, session_id: &SessionId) -> Result<()> {
        match event {
            RtpEvent::MediaReceived { payload_type, payload, timestamp, .. } => {
                // Map payload type to codec
                let codec_name = match payload_type {
                    0 => "PCMU",  // G.711 μ-law
                    8 => "PCMA",  // G.711 A-law  
                    _ => return Ok(()), // Unsupported
                };
                
                // Decode using codec-core
                let codec = self.codec_registry.get_codec(codec_name)?;
                let pcm_samples = codec.decode(&payload)?;
                
                // Create AudioFrame
                let audio_frame = AudioFrame {
                    samples: pcm_samples,
                    sample_rate: 8000, // G.711 is always 8kHz
                    channels: 1,       // G.711 is always mono
                    timestamp,
                };
                
                // Forward to subscribers
                if let Some(sender) = self.audio_frame_senders.get(session_id) {
                    let _ = sender.send(audio_frame).await;
                }
            }
            _ => {}
        }
        Ok(())
    }
    
    pub fn add_subscriber(&mut self, session_id: SessionId, sender: mpsc::Sender<AudioFrame>) {
        self.audio_frame_senders.insert(session_id, sender);
    }
}
```

#### 1.2 Integrate Decoder into MediaManager  
**Modify**: `session-core/src/media/manager.rs`

```rust
impl MediaManager {
    pub fn new(config: MediaConfig, local_bind_addr: SocketAddr) -> Self {
        // Initialize codec registry with G.711 support
        let mut codec_registry = CodecRegistry::new();
        codec_registry.register("PCMU", Box::new(G711Codec::new(G711Variant::MuLaw)));
        codec_registry.register("PCMA", Box::new(G711Codec::new(G711Variant::ALaw)));
        
        let rtp_decoder = RtpPayloadDecoder {
            codec_registry: Arc::new(codec_registry),
            audio_frame_senders: HashMap::new(),
        };
        
        Self {
            // ... existing fields
            rtp_decoder: Arc::new(Mutex::new(rtp_decoder)),
            rtp_processing_active: HashSet::new(),
        }
    }
    
    pub async fn set_audio_frame_callback(
        &mut self, 
        session_id: &SessionId, 
        sender: mpsc::Sender<AudioFrame>
    ) -> Result<()> {
        // Register sender with decoder
        {
            let mut decoder = self.rtp_decoder.lock().await;
            decoder.add_subscriber(session_id.clone(), sender);
        }
        
        // Start RTP event processing task
        if !self.rtp_processing_active.contains(session_id) {
            self.start_rtp_event_processing(session_id).await?;
            self.rtp_processing_active.insert(session_id.clone());
        }
        
        Ok(())
    }
    
    async fn start_rtp_event_processing(&self, session_id: &SessionId) -> Result<()> {
        // Get RTP transport for this session
        let mut rtp_events = self.get_rtp_transport(session_id)?.subscribe();
        let decoder = self.rtp_decoder.clone();
        let session_id = session_id.clone();
        
        tokio::spawn(async move {
            while let Ok(event) = rtp_events.recv().await {
                let mut decoder = decoder.lock().await;
                if let Err(e) = decoder.process_rtp_event(event, &session_id).await {
                    tracing::error!("RTP decoding failed for {}: {}", session_id, e);
                }
            }
        });
        
        Ok(())
    }
}
```

### Phase 2: Audio Frame Subscription System

#### 2.1 Implement AudioFrameSubscriber
**Modify**: `session-core/src/api/types.rs`

```rust
pub struct AudioFrameSubscriber {
    session_id: SessionId,
    receiver: std::sync::mpsc::Receiver<AudioFrame>,
}

impl AudioFrameSubscriber {
    pub fn new(session_id: SessionId, receiver: std::sync::mpsc::Receiver<AudioFrame>) -> Self {
        Self { session_id, receiver }
    }
    
    pub fn recv(&self) -> Result<AudioFrame, std::sync::mpsc::RecvError> {
        self.receiver.recv()
    }
    
    pub fn try_recv(&self) -> Result<AudioFrame, std::sync::mpsc::TryRecvError> {
        self.receiver.try_recv()
    }
    
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }
}
```

#### 2.2 Complete MediaControl Implementation
**Modify**: `session-core/src/api/media.rs`

```rust
impl MediaControl for Arc<SessionCoordinator> {    
    async fn subscribe_to_audio_frames(&self, session_id: &SessionId) -> Result<AudioFrameSubscriber> {
        // Validate session exists
        if SessionControl::get_session(self, session_id).await?.is_none() {
            return Err(SessionError::SessionNotFound(session_id.to_string()));
        }
        
        // Create channel bridge: tokio mpsc (MediaManager) → std mpsc (subscriber)
        let (tokio_sender, mut tokio_receiver) = tokio::sync::mpsc::channel::<AudioFrame>(100);
        let (std_sender, std_receiver) = std::sync::mpsc::channel::<AudioFrame>();
        
        // Register callback with MediaManager
        let media_manager = &self.media_manager;
        media_manager.set_audio_frame_callback(session_id, tokio_sender).await
            .map_err(|e| SessionError::MediaIntegration { 
                message: format!("Failed to set audio frame callback: {}", e) 
            })?;
        
        // Bridge async → sync channels
        let session_id_clone = session_id.clone();
        tokio::spawn(async move {
            while let Some(frame) = tokio_receiver.recv().await {
                if let Err(e) = std_sender.send(frame) {
                    tracing::warn!("Failed to forward audio frame for session {}: {}", session_id_clone, e);
                    break;
                }
            }
        });
        
        Ok(AudioFrameSubscriber::new(session_id.clone(), std_receiver))
    }
}
```

### Phase 3: Client-Core Integration

#### 3.1 Add Audio Streaming Methods
**Modify**: `client-core/src/client/manager.rs`

```rust
impl ClientManager {
    pub async fn subscribe_to_audio_frames(
        &self, 
        call_id: &CallId
    ) -> ClientResult<AudioFrameSubscriber> {
        // Map call_id to session_id
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
        
        // Delegate to session-core
        MediaControl::subscribe_to_audio_frames(&self.coordinator, &session_id)
            .await
            .map_err(|e| ClientError::InternalError { 
                message: format!("Failed to subscribe to audio frames: {}", e) 
            })
    }
    
    pub async fn send_audio_frame(
        &self, 
        call_id: &CallId, 
        audio_frame: AudioFrame
    ) -> ClientResult<()> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
        
        MediaControl::send_audio_frame(&self.coordinator, &session_id, audio_frame)
            .await
            .map_err(|e| ClientError::InternalError { 
                message: format!("Failed to send audio frame: {}", e) 
            })
    }
}
```

#### 3.2 Re-export Audio Types
**Modify**: `client-core/src/lib.rs`

```rust
// Re-export audio streaming types
pub use rvoip_session_core::api::types::{
    AudioFrame, 
    AudioFrameSubscriber, 
    AudioStreamConfig
};
```

### Phase 4: RTP Event Processing Integration

#### 4.1 Connect RTP Transport to MediaManager
**Modify**: `session-core/src/media/manager.rs`

```rust
impl MediaManager {
    async fn get_rtp_transport(&self, session_id: &SessionId) -> Result<RtpEventReceiver> {
        // Get dialog ID from session mapping
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
                .ok_or_else(|| MediaError::SessionNotFound { 
                    session_id: session_id.to_string() 
                })?
        };
        
        // Get RTP session from controller
        let rtp_sessions = self.controller.rtp_sessions.read().await;
        let rtp_wrapper = rtp_sessions.get(&dialog_id)
            .ok_or_else(|| MediaError::SessionNotFound { 
                session_id: format!("No RTP session for dialog {}", dialog_id) 
            })?;
        
        // Subscribe to RTP events
        let rtp_session = rtp_wrapper.session.lock().await;
        Ok(rtp_session.subscribe_events())
    }
}
```

### Phase 5: SIP Client Integration (Already Mostly Implemented)

The sip-client already has the correct structure in `setup_audio_pipeline()`:

```rust
// Subscribe to incoming audio frames (line 672-678)
let mut audio_subscriber = self.inner.client
    .subscribe_to_audio_frames(&call.id)  // ← This method was missing!
    .await?;

// Playback task (line 702-747) - Already implemented correctly
let playback_task = {
    let pipeline = playback_pipeline.clone();
    tokio::spawn(async move {
        while let Ok(audio_frame) = audio_subscriber.recv() {
            // Convert format and play
            let converted_frame = convert_audio_frame(audio_frame, &playback_format);
            if let Err(e) = pipeline.playback_frame(converted_frame).await {
                tracing::error!("Playback failed: {}", e);
                break;
            }
        }
    })
};
```

## Data Flow Architecture

```text
┌─────────────────┐    ┌──────────────────┐    ┌─────────────────┐
│  Network/UDP    │───▶│   rtp-core       │───▶│  RtpEvent       │
│  RTP Packets    │    │  RtpSession      │    │  MediaReceived  │
└─────────────────┘    └──────────────────┘    └─────────────────┘
                                                        │
                                                        ▼
┌─────────────────┐    ┌──────────────────┐    ┌─────────────────┐
│   sip-client    │◀───│   client-core    │◀───│  session-core   │
│  Playback Task  │    │subscribe_to_     │    │  RtpPayload     │
│                 │    │audio_frames()    │    │  Decoder        │
└─────────────────┘    └──────────────────┘    └─────────────────┘
        │                                              │
        ▼                                              ▼
┌─────────────────┐    ┌──────────────────┐    ┌─────────────────┐
│   audio-core    │    │ AudioFrame       │◀───│   codec-core    │
│  AudioPipeline  │◀───│ Subscriber       │    │  G.711 Decode   │
│  Speakers       │    │ (std::mpsc)      │    │                 │
└─────────────────┘    └──────────────────┘    └─────────────────┘
```

## Implementation Timeline

### Week 1: Core RTP Processing
- **Day 1-2**: Implement RtpPayloadDecoder and codec integration
- **Day 3**: Integrate decoder into MediaManager with RTP event processing
- **Day 4**: Unit testing and codec validation

### Week 2: Client Integration  
- **Day 1**: Implement AudioFrameSubscriber system
- **Day 2**: Add missing client-core methods (subscribe_to_audio_frames, send_audio_frame)
- **Day 3**: Integration testing with sip-client
- **Day 4**: End-to-end bidirectional voice testing

### Week 3: Polish & Performance
- **Day 1-2**: Performance optimization and latency reduction
- **Day 3**: Error handling and edge case testing  
- **Day 4**: Documentation and example updates

## Files Requiring Changes

### New Files:
1. `session-core/src/media/rtp_decoder.rs` - RTP payload decoding logic
2. `session-core/src/media/codec_integration.rs` - Codec registry setup

### Modified Files:
1. `session-core/src/media/manager.rs` - RTP event processing integration
2. `session-core/src/api/types.rs` - AudioFrameSubscriber implementation
3. `session-core/src/api/media.rs` - Complete MediaControl implementation
4. `client-core/src/client/manager.rs` - Audio streaming method additions
5. `client-core/src/lib.rs` - Type re-exports
6. `session-core/src/media/mod.rs` - Module declarations

## Dependencies and Prerequisites

### Crate Dependencies:
- **codec-core**: Already has G.711 encode/decode implementation ✅
- **rtp-core**: Already has event-driven RTP packet reception ✅
- **audio-core**: Already has playback pipeline infrastructure ✅

### Critical Interfaces:
- `RtpEvent::MediaReceived` from rtp-core ✅
- `CodecRegistry` and G.711 codec from codec-core ✅  
- `AudioPipeline::playback_frame()` from audio-core ✅

## Testing Strategy

### Unit Tests:
- RtpPayloadDecoder with sample G.711 payloads
- AudioFrameSubscriber channel bridging
- Codec integration with various payload types

### Integration Tests:
- End-to-end RTP packet → decoded AudioFrame pipeline
- Client-core method integration with session-core
- Bidirectional audio flow with real network packets

### Performance Tests:
- Frame delivery latency (target: <20ms)
- Memory usage with continuous audio streams
- Error recovery and packet loss handling

## Risk Assessment

### High Risk:
- **RTP Event Processing**: Complex async integration between layers
- **Codec Integration**: Ensuring proper G.711 decode quality

### Medium Risk:  
- **Channel Bridging**: tokio::mpsc ↔ std::mpsc reliability
- **Session Mapping**: Call ID → Session ID → Dialog ID lookups

### Low Risk:
- **Client Integration**: Straightforward method additions
- **Audio Playback**: Pipeline already implemented and tested

## Success Criteria

1. **Primary Goal**: Bidirectional voice conversation in sip-client-p2p example
2. **Audio Quality**: Clear G.711 decode quality matching encode
3. **Latency**: End-to-end audio latency <100ms  
4. **Reliability**: Handles packet loss and network jitter gracefully
5. **Performance**: Minimal CPU overhead for audio processing

This comprehensive plan addresses the 4-month audio issue by implementing the missing RTP receive path while leveraging all the existing working infrastructure.