# Media Core Library Design - SIP Best Practices & Clean Architecture

## 🎯 **Vision & Scope**

**media-core** is the media processing engine for the RVOIP stack. It focuses exclusively on media processing, codec management, and media session coordination while integrating cleanly with:

- **session-core**: SIP signaling and dialog management  
- **rtp-core**: RTP transport and packet handling

### **Core Responsibilities**
✅ **Media Processing**: Codec encode/decode, audio processing (AEC, AGC, VAD, NS)  
✅ **Media Session Management**: Coordinate media flows for SIP dialogs  
✅ **Quality Management**: Monitor and adapt media quality  
✅ **Format Conversion**: Sample rate conversion, channel mixing  
✅ **Codec Management**: Registry, negotiation, transcoding  

### **NOT Responsibilities** (Delegated)
❌ **RTP Transport**: Handled by rtp-core  
❌ **SIP Signaling**: Handled by session-core  
❌ **Network I/O**: Handled by rtp-core  
❌ **SDP Negotiation**: Handled by session-core (media-core provides capabilities)  

---

## 🏗️ **Architecture Overview**

```
┌─────────────────┐    Media Capabilities    ┌─────────────────┐
│                 │ ◄──────────────────────── │                 │
│  session-core   │                           │   media-core    │
│ (SIP Signaling) │ ──────────────────────► │ (Processing)    │
│                 │    Media Session Mgmt     │                 │
└─────────────────┘                           └─────────────────┘
                                                       │
                                                       │ Media Streams
                                                       ▼
                                              ┌─────────────────┐
                                              │    rtp-core     │
                                              │  (Transport)    │
                                              └─────────────────┘
```

### **Integration Patterns**

1. **session-core → media-core**: Request media capabilities, create/destroy media sessions
2. **media-core → session-core**: Provide codec capabilities, report media session events  
3. **rtp-core → media-core**: Deliver incoming media packets for processing
4. **media-core → rtp-core**: Send processed media packets for transmission

---

## 📁 **Directory Structure**

```
src/
├── lib.rs                     # Public API, re-exports, and documentation
├── error.rs                   # Comprehensive error types
├── types.rs                   # Common types, constants, and utilities
│
├── engine/                    # Core Media Engine
│   ├── mod.rs                 # Module exports
│   ├── media_engine.rs        # Central MediaEngine orchestrator
│   ├── config.rs              # Engine configuration and settings
│   └── lifecycle.rs           # Engine startup/shutdown management
│
├── session/                   # Media Session Management  
│   ├── mod.rs                 # Module exports
│   ├── media_session.rs       # MediaSession per SIP dialog
│   ├── session_manager.rs     # Manages multiple MediaSessions
│   ├── events.rs              # Media session event types
│   ├── state.rs               # Session state management
│   └── coordinator.rs         # Coordinates media flows
│
├── codec/                     # Codec Framework
│   ├── mod.rs                 # Module exports and traits
│   ├── manager.rs             # CodecManager - central codec orchestration
│   ├── registry.rs            # CodecRegistry - available codecs
│   ├── traits.rs              # Codec traits (AudioCodec, VideoCodec)
│   ├── negotiation.rs         # Codec negotiation and capability matching
│   ├── transcoding.rs         # Cross-codec transcoding
│   ├── audio/                 # Audio Codec Implementations
│   │   ├── mod.rs
│   │   ├── g711.rs            # G.711 μ-law/A-law (PCMU/PCMA)
│   │   ├── opus.rs            # Opus codec (wideband/fullband)
│   │   ├── g722.rs            # G.722 wideband codec
│   │   └── dtmf.rs            # DTMF/telephone-event (RFC 4733)
│   └── video/                 # Video Codec Implementations (future)
│       ├── mod.rs
│       └── h264.rs            # H.264 codec (future)
│
├── processing/                # Media Signal Processing
│   ├── mod.rs                 # Module exports
│   ├── pipeline.rs            # Processing pipeline orchestration
│   ├── audio/                 # Audio Processing Components
│   │   ├── mod.rs
│   │   ├── processor.rs       # AudioProcessor - main audio processing
│   │   ├── aec.rs             # Acoustic Echo Cancellation
│   │   ├── agc.rs             # Automatic Gain Control
│   │   ├── vad.rs             # Voice Activity Detection
│   │   ├── ns.rs              # Noise Suppression
│   │   ├── plc.rs             # Packet Loss Concealment
│   │   └── dtmf_detector.rs   # DTMF tone detection
│   ├── format/                # Format Conversion
│   │   ├── mod.rs
│   │   ├── converter.rs       # FormatConverter - main conversion
│   │   ├── resampler.rs       # Sample rate conversion
│   │   ├── channel_mixer.rs   # Channel layout conversion
│   │   └── bit_depth.rs       # Bit depth conversion
│   └── effects/               # Audio Effects (optional)
│       ├── mod.rs
│       ├── equalizer.rs       # Audio EQ
│       └── compressor.rs      # Dynamic range compression
│
├── quality/                   # Quality Monitoring & Adaptation
│   ├── mod.rs                 # Module exports
│   ├── monitor.rs             # QualityMonitor - real-time monitoring
│   ├── metrics.rs             # Quality metrics collection
│   ├── adaptation.rs          # Quality adaptation strategies
│   ├── analyzer.rs            # Media quality analysis
│   └── reporter.rs            # Quality reporting to session-core
│
├── buffer/                    # Media Buffering
│   ├── mod.rs                 # Module exports
│   ├── jitter.rs              # JitterBuffer - adaptive jitter buffering
│   ├── adaptive.rs            # AdaptiveBuffer - dynamic buffer sizing
│   ├── frame_buffer.rs        # FrameBuffer - frame-based buffering
│   └── ring_buffer.rs         # RingBuffer - circular buffer utilities
│
├── integration/               # Integration Bridges
│   ├── mod.rs                 # Module exports
│   ├── rtp_bridge.rs          # RtpBridge - integration with rtp-core
│   ├── session_bridge.rs      # SessionBridge - integration with session-core
│   └── events.rs              # Cross-crate event handling
│
└── examples/                  # Usage Examples
    ├── basic_session.rs       # Basic media session setup
    ├── codec_transcoding.rs   # Codec transcoding example
    └── quality_monitoring.rs  # Quality monitoring example
```

---

## 🏛️ **Core Architecture Components**

### **1. MediaEngine** - Central Orchestrator
```rust
pub struct MediaEngine {
    codec_manager: Arc<CodecManager>,
    session_manager: Arc<SessionManager>,
    quality_monitor: Arc<QualityMonitor>,
    audio_processor: Arc<AudioProcessor>,
    format_converter: Arc<FormatConverter>,
    config: MediaEngineConfig,
}

impl MediaEngine {
    // Core lifecycle
    pub async fn new(config: MediaEngineConfig) -> Result<Self>;
    pub async fn start(&self) -> Result<()>;
    pub async fn stop(&self) -> Result<()>;
    
    // Session management
    pub async fn create_media_session(&self, dialog_id: DialogId, params: MediaSessionParams) -> Result<MediaSessionHandle>;
    pub async fn destroy_media_session(&self, dialog_id: DialogId) -> Result<()>;
    
    // Capability discovery
    pub fn get_supported_codecs(&self) -> Vec<CodecCapability>;
    pub fn get_media_capabilities(&self) -> MediaCapabilities;
}
```

### **2. MediaSession** - Per-Dialog Media Management
```rust
pub struct MediaSession {
    dialog_id: DialogId,
    state: RwLock<MediaSessionState>,
    audio_codec: RwLock<Option<Box<dyn AudioCodec>>>,
    video_codec: RwLock<Option<Box<dyn VideoCodec>>>,
    jitter_buffer: Arc<JitterBuffer>,
    quality_metrics: Arc<RwLock<QualityMetrics>>,
    event_tx: mpsc::UnboundedSender<MediaSessionEvent>,
}

impl MediaSession {
    // Media processing
    pub async fn process_incoming_media(&self, packet: MediaPacket) -> Result<()>;
    pub async fn send_outgoing_media(&self, frame: MediaFrame) -> Result<()>;
    
    // Codec management
    pub async fn set_audio_codec(&self, codec: Box<dyn AudioCodec>) -> Result<()>;
    pub async fn set_video_codec(&self, codec: Box<dyn VideoCodec>) -> Result<()>;
    
    // Quality management
    pub async fn get_quality_metrics(&self) -> QualityMetrics;
    pub async fn adjust_quality(&self, adjustment: QualityAdjustment) -> Result<()>;
}
```

### **3. CodecManager** - Codec Orchestration
```rust
pub struct CodecManager {
    registry: Arc<CodecRegistry>,
    transcoder: Arc<Transcoder>,
    negotiator: Arc<CodecNegotiator>,
}

impl CodecManager {
    // Codec lifecycle
    pub fn create_audio_codec(&self, payload_type: u8, params: &CodecParams) -> Result<Box<dyn AudioCodec>>;
    pub fn create_video_codec(&self, payload_type: u8, params: &CodecParams) -> Result<Box<dyn VideoCodec>>;
    
    // Capability management
    pub fn get_supported_audio_codecs(&self) -> Vec<AudioCodecCapability>;
    pub fn get_supported_video_codecs(&self) -> Vec<VideoCodecCapability>;
    
    // Negotiation
    pub fn negotiate_codecs(&self, local_caps: &[CodecCapability], remote_caps: &[CodecCapability]) -> Result<CodecNegotiationResult>;
}
```

### **4. AudioProcessor** - Audio Processing Pipeline
```rust
pub struct AudioProcessor {
    aec: Option<Box<dyn AcousticEchoCanceller>>,
    agc: Option<Box<dyn AutomaticGainControl>>,
    vad: Option<Box<dyn VoiceActivityDetector>>,
    ns: Option<Box<dyn NoiseSuppressor>>,
    config: AudioProcessingConfig,
}

impl AudioProcessor {
    // Processing pipeline
    pub fn process_capture_audio(&self, input: &AudioFrame) -> Result<AudioFrame>;
    pub fn process_playback_audio(&self, input: &AudioFrame) -> Result<AudioFrame>;
    
    // Component management
    pub fn enable_aec(&mut self, config: AecConfig) -> Result<()>;
    pub fn enable_agc(&mut self, config: AgcConfig) -> Result<()>;
    pub fn enable_vad(&mut self, config: VadConfig) -> Result<()>;
}
```

### **5. QualityMonitor** - Real-time Quality Management
```rust
pub struct QualityMonitor {
    metrics_collector: Arc<MetricsCollector>,
    adaptation_engine: Arc<AdaptationEngine>,
    thresholds: QualityThresholds,
}

impl QualityMonitor {
    // Quality monitoring
    pub async fn analyze_media_quality(&self, session_id: &MediaSessionId, packet: &MediaPacket) -> QualityMetrics;
    pub async fn suggest_quality_adjustments(&self, session_id: &MediaSessionId) -> Vec<QualityAdjustment>;
    
    // Metrics
    pub async fn get_session_metrics(&self, session_id: &MediaSessionId) -> Result<SessionMetrics>;
    pub async fn get_overall_metrics(&self) -> OverallMetrics;
}
```

---

## 🔗 **Integration Interfaces**

### **session-core Integration**
```rust
// Media capabilities for SDP negotiation
pub trait MediaCapabilityProvider {
    fn get_audio_capabilities(&self) -> Vec<AudioCapability>;
    fn get_video_capabilities(&self) -> Vec<VideoCapability>;
    fn negotiate_media(&self, local_sdp: &Sdp, remote_sdp: &Sdp) -> Result<MediaNegotiationResult>;
}

// Media session lifecycle
pub trait MediaSessionProvider {
    async fn create_media_session(&self, dialog_id: DialogId, params: MediaSessionParams) -> Result<MediaSessionHandle>;
    async fn update_media_session(&self, dialog_id: DialogId, params: MediaSessionParams) -> Result<()>;
    async fn destroy_media_session(&self, dialog_id: DialogId) -> Result<()>;
}
```

### **rtp-core Integration**
```rust
// Media packet handling
pub trait MediaPacketHandler {
    async fn handle_incoming_packet(&self, session_id: &MediaSessionId, packet: RtpPacket) -> Result<()>;
    async fn send_outgoing_packet(&self, session_id: &MediaSessionId, packet: RtpPacket) -> Result<()>;
}

// RTP session coordination
pub trait RtpSessionCoordinator {
    async fn register_media_session(&self, session_id: MediaSessionId, rtp_session: Arc<RtpSession>) -> Result<()>;
    async fn unregister_media_session(&self, session_id: &MediaSessionId) -> Result<()>;
}
```

---

## 📋 **Implementation Phases**

### **Phase 1: Core Foundation** (2-3 weeks)
- [ ] **Basic Types & Errors** (`types.rs`, `error.rs`)
- [ ] **MediaEngine Structure** (`engine/media_engine.rs`)  
- [ ] **MediaSession Basic** (`session/media_session.rs`)
- [ ] **Simple CodecRegistry** (`codec/registry.rs`)
- [ ] **G.711 Implementation** (`codec/audio/g711.rs`)
- [ ] **Integration Stubs** (`integration/`)

### **Phase 2: Processing Pipeline** (2-3 weeks)  
- [ ] **AudioProcessor Framework** (`processing/audio/processor.rs`)
- [ ] **Basic VAD** (`processing/audio/vad.rs`)
- [ ] **FormatConverter** (`processing/format/converter.rs`)
- [ ] **JitterBuffer** (`buffer/jitter.rs`)
- [ ] **Quality Monitoring** (`quality/monitor.rs`)

### **Phase 3: Advanced Features** (3-4 weeks)
- [ ] **AEC Implementation** (`processing/audio/aec.rs`)
- [ ] **AGC Implementation** (`processing/audio/agc.rs`)  
- [ ] **Opus Codec** (`codec/audio/opus.rs`)
- [ ] **Codec Transcoding** (`codec/transcoding.rs`)
- [ ] **Quality Adaptation** (`quality/adaptation.rs`)

### **Phase 4: Production Ready** (2-3 weeks)
- [ ] **Comprehensive Testing**
- [ ] **Performance Optimization**  
- [ ] **Documentation & Examples**
- [ ] **Integration Testing with session-core & rtp-core**

---

## 🎯 **Success Criteria**

### **Phase 1 Complete**
- [ ] `cargo check` passes without errors
- [ ] Basic media session creation/destruction works
- [ ] G.711 codec encode/decode functional
- [ ] Integration stubs allow session-core/rtp-core to compile against media-core

### **Final Success**
- [ ] Two SIP clients can make calls through the server with high-quality audio
- [ ] Opus and G.711 codecs work seamlessly
- [ ] Audio processing (AEC, AGC, VAD) enhances call quality
- [ ] Quality monitoring provides actionable insights
- [ ] Clean separation of concerns with other crates
- [ ] Comprehensive test coverage (>80%)
- [ ] Production-ready performance and stability

---

## 🔄 **Next Steps**

1. **Review & Approve Architecture** - Get feedback on this design
2. **Create Basic Project Structure** - Set up the directory structure  
3. **Implement Phase 1** - Start with core foundation
4. **Integration Testing** - Test with session-core and rtp-core early and often
5. **Iterative Development** - Build incrementally with continuous testing

**Target**: Production-ready media-core within 10-12 weeks, fully integrated with the RVOIP stack. 