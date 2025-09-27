# Media Server Implementation Plan

## Executive Summary

This plan describes `media-server-core`, a standalone media server library that processes RTP, performs mixing, recording, and IVR operations. It's designed to be controlled by b2bua-core via API, handling all media processing separately from SIP signaling.

## Architecture Overview

```
┌──────────────────────────────────────────────────────┐
│                 B2BUA Core                           │
│            (Controls via API)                        │
└─────────────────┬────────────────────────────────────┘
                  │ REST/gRPC API
                  ▼
┌──────────────────────────────────────────────────────┐
│              Media Server Core                       │
├──────────────────────────────────────────────────────┤
│  ┌──────────┐ ┌──────────┐ ┌──────────┐           │
│  │ Endpoint │ │  Mixer   │ │ Recorder │           │
│  │  Pool    │ │  Engine  │ │  Engine  │           │
│  └─────┬────┘ └─────┬────┘ └─────┬────┘           │
│        └────────────┼─────────────┘                 │
│                     ▼                                │
│          ┌──────────────────┐                       │
│          │   Media Core     │                       │
│          │ (RTP Processing) │                       │
│          └──────────────────┘                       │
└──────────────────────────────────────────────────────┘
                  │
                  ▼ RTP/RTCP
            [Network - UDP Ports]
```

## Design Principles

1. **Standalone Server**: Runs independently from B2BUA
2. **API Controlled**: REST/gRPC interface for control
3. **Scalable**: Horizontal scaling via multiple instances
4. **Efficient**: Optimized for high-throughput RTP processing
5. **Modular**: Pluggable engines for different operations

## Core Components

### 1. Media Server Core

```rust
// media-server-core/src/lib.rs
pub struct MediaServerCore {
    // Resource management
    endpoint_pool: Arc<EndpointPool>,

    // Processing engines
    mixer: Arc<MixerEngine>,
    recorder: Arc<RecorderEngine>,
    player: Arc<PlayerEngine>,
    dtmf_detector: Arc<DtmfDetector>,

    // Network layer
    rtp_transport: Arc<RtpTransport>,

    // Control API
    api_server: Arc<ApiServer>,

    // Configuration
    config: MediaServerConfig,

    // Metrics
    metrics: Arc<Metrics>,
}

impl MediaServerCore {
    pub async fn start(&self) -> Result<()> {
        // Start RTP transport
        self.rtp_transport.start().await?;

        // Start processing engines
        self.mixer.start().await?;
        self.recorder.start().await?;
        self.player.start().await?;

        // Start API server
        self.api_server.start().await?;

        info!("Media server started on ports {:?}", self.config.rtp_port_range);
        Ok(())
    }
}
```

### 2. Endpoint Management

```rust
// media-server-core/src/endpoint.rs
pub struct Endpoint {
    pub id: EndpointId,
    pub rtp_port: u16,
    pub rtcp_port: u16,
    pub local_ip: IpAddr,
    pub remote_addr: Option<SocketAddr>,
    pub codecs: Vec<Codec>,
    pub state: EndpointState,
    pub statistics: Arc<RwLock<EndpointStats>>,
}

pub struct EndpointPool {
    endpoints: Arc<DashMap<EndpointId, Arc<Endpoint>>>,
    available_ports: Arc<Mutex<VecDeque<u16>>>,
    max_endpoints: usize,
}

impl EndpointPool {
    pub async fn allocate(&self) -> Result<Endpoint> {
        // Get available port
        let rtp_port = self.available_ports
            .lock().await
            .pop_front()
            .ok_or(Error::NoPortsAvailable)?;

        // Create endpoint
        let endpoint = Endpoint {
            id: EndpointId::new(),
            rtp_port,
            rtcp_port: rtp_port + 1,
            local_ip: self.config.bind_ip,
            remote_addr: None,
            codecs: vec![],
            state: EndpointState::Allocated,
            statistics: Arc::new(RwLock::new(EndpointStats::default())),
        };

        // Store and return
        self.endpoints.insert(endpoint.id, Arc::new(endpoint.clone()));
        Ok(endpoint)
    }

    pub async fn release(&self, id: EndpointId) -> Result<()> {
        if let Some((_, endpoint)) = self.endpoints.remove(&id) {
            // Return port to pool
            self.available_ports.lock().await.push_back(endpoint.rtp_port);

            // Clean up resources
            self.cleanup_endpoint(&endpoint).await?;
        }
        Ok(())
    }
}

pub enum EndpointState {
    Allocated,
    Connected,
    Bridged(EndpointId),
    InConference(ConferenceId),
    Playing,
    Recording,
    OnHold,
}
```

### 3. RTP Transport Layer

```rust
// media-server-core/src/transport.rs
pub struct RtpTransport {
    // Socket management
    sockets: Arc<DashMap<u16, Arc<UdpSocket>>>,

    // Packet routing
    router: Arc<PacketRouter>,

    // Media core integration
    media_core: Arc<MediaCore>,
}

impl RtpTransport {
    pub async fn start(&self) -> Result<()> {
        // Bind sockets for port range
        for port in self.config.port_range.clone() {
            let socket = UdpSocket::bind((self.config.bind_ip, port)).await?;
            self.sockets.insert(port, Arc::new(socket));

            // Spawn receiver task
            self.spawn_receiver(port).await;
        }
        Ok(())
    }

    async fn spawn_receiver(&self, port: u16) {
        let socket = self.sockets.get(&port).unwrap().clone();
        let router = self.router.clone();

        tokio::spawn(async move {
            let mut buf = vec![0u8; 2048];
            loop {
                match socket.recv_from(&mut buf).await {
                    Ok((len, addr)) => {
                        let packet = &buf[..len];
                        router.route_packet(port, addr, packet).await;
                    }
                    Err(e) => {
                        error!("RTP receive error on port {}: {}", port, e);
                    }
                }
            }
        });
    }
}

pub struct PacketRouter {
    endpoints: Arc<EndpointPool>,
    processor: Arc<MediaCore>,
}

impl PacketRouter {
    pub async fn route_packet(
        &self,
        local_port: u16,
        remote_addr: SocketAddr,
        packet: &[u8],
    ) {
        // Find endpoint by port
        if let Some(endpoint) = self.endpoints.find_by_port(local_port).await {
            // Update remote address if needed
            if endpoint.remote_addr.is_none() {
                endpoint.set_remote_addr(remote_addr).await;
            }

            // Process based on state
            match &endpoint.state {
                EndpointState::Bridged(other_id) => {
                    // Forward to bridged endpoint
                    if let Some(other) = self.endpoints.get(other_id).await {
                        self.forward_packet(packet, &other).await;
                    }
                }
                EndpointState::InConference(conf_id) => {
                    // Send to mixer
                    self.mixer.add_packet(*conf_id, endpoint.id, packet).await;
                }
                EndpointState::Recording => {
                    // Send to recorder
                    self.recorder.add_packet(endpoint.id, packet).await;
                }
                _ => {
                    // Process normally
                    self.processor.process_rtp(packet).await;
                }
            }

            // Update statistics
            endpoint.statistics.write().await.packets_received += 1;
        }
    }
}
```

### 4. Mixer Engine (Conferencing)

```rust
// media-server-core/src/mixer.rs
pub struct MixerEngine {
    conferences: Arc<DashMap<ConferenceId, Arc<Conference>>>,
    mixer_threads: Vec<JoinHandle<()>>,
}

pub struct Conference {
    pub id: ConferenceId,
    pub participants: Arc<DashMap<EndpointId, Participant>>,
    pub mixer: Arc<Mutex<AudioMixer>>,
    pub config: ConferenceConfig,
}

pub struct Participant {
    pub endpoint_id: EndpointId,
    pub is_muted: AtomicBool,
    pub is_moderator: bool,
    pub audio_buffer: Arc<Mutex<RingBuffer>>,
    pub last_packet_time: Arc<RwLock<Instant>>,
}

impl MixerEngine {
    pub async fn create_conference(&self, config: ConferenceConfig) -> Result<ConferenceId> {
        let id = ConferenceId::new();

        let conference = Arc::new(Conference {
            id,
            participants: Arc::new(DashMap::new()),
            mixer: Arc::new(Mutex::new(AudioMixer::new(config.sample_rate))),
            config,
        });

        self.conferences.insert(id, conference.clone());

        // Start mixer thread
        self.spawn_mixer_thread(conference).await;

        Ok(id)
    }

    async fn spawn_mixer_thread(&self, conference: Arc<Conference>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(20)); // 50Hz

            loop {
                interval.tick().await;

                // Collect audio from all participants
                let mut audio_streams = Vec::new();

                for entry in conference.participants.iter() {
                    let participant = entry.value();

                    if !participant.is_muted.load(Ordering::Relaxed) {
                        if let Some(audio) = participant.read_audio().await {
                            audio_streams.push((participant.endpoint_id, audio));
                        }
                    }
                }

                // Mix audio
                let mixed = conference.mixer.lock().await.mix(&audio_streams);

                // Send mixed audio to each participant (minus their own)
                for entry in conference.participants.iter() {
                    let participant = entry.value();
                    let custom_mix = conference.mixer.lock().await
                        .mix_excluding(&audio_streams, participant.endpoint_id);

                    participant.send_audio(custom_mix).await;
                }
            }
        });
    }

    pub async fn add_participant(
        &self,
        conf_id: ConferenceId,
        endpoint_id: EndpointId,
    ) -> Result<()> {
        if let Some(conference) = self.conferences.get(&conf_id) {
            let participant = Participant {
                endpoint_id,
                is_muted: AtomicBool::new(false),
                is_moderator: false,
                audio_buffer: Arc::new(Mutex::new(RingBuffer::new(8000))),
                last_packet_time: Arc::new(RwLock::new(Instant::now())),
            };

            conference.participants.insert(endpoint_id, participant);
            Ok(())
        } else {
            Err(Error::ConferenceNotFound)
        }
    }
}

pub struct AudioMixer {
    sample_rate: u32,
    mixing_buffer: Vec<i16>,
}

impl AudioMixer {
    pub fn mix(&mut self, streams: &[(EndpointId, Vec<i16>)]) -> Vec<i16> {
        // Reset buffer
        self.mixing_buffer.clear();
        self.mixing_buffer.resize(160, 0); // 20ms at 8kHz

        // Mix all streams
        for (_, samples) in streams {
            for (i, &sample) in samples.iter().enumerate() {
                if i < self.mixing_buffer.len() {
                    // Saturating addition to prevent overflow
                    let mixed = self.mixing_buffer[i].saturating_add(sample);
                    self.mixing_buffer[i] = mixed;
                }
            }
        }

        // Normalize
        let num_streams = streams.len() as i16;
        if num_streams > 1 {
            for sample in &mut self.mixing_buffer {
                *sample /= num_streams;
            }
        }

        self.mixing_buffer.clone()
    }
}
```

### 5. Recording Engine

```rust
// media-server-core/src/recorder.rs
pub struct RecorderEngine {
    recordings: Arc<DashMap<RecordingId, Arc<Recording>>>,
    storage: Arc<StorageBackend>,
}

pub struct Recording {
    pub id: RecordingId,
    pub endpoint_id: EndpointId,
    pub file_path: PathBuf,
    pub format: RecordingFormat,
    pub state: RecordingState,
    pub writer: Arc<Mutex<WavWriter>>,
    pub start_time: Instant,
    pub duration: Arc<RwLock<Duration>>,
}

impl RecorderEngine {
    pub async fn start_recording(
        &self,
        endpoint_id: EndpointId,
        format: RecordingFormat,
    ) -> Result<RecordingId> {
        let id = RecordingId::new();
        let file_path = self.storage.generate_path(&id, &format).await?;

        let writer = match format {
            RecordingFormat::Wav => WavWriter::create(&file_path)?,
            RecordingFormat::Mp3 => Mp3Writer::create(&file_path)?,
            RecordingFormat::Opus => OpusWriter::create(&file_path)?,
        };

        let recording = Arc::new(Recording {
            id,
            endpoint_id,
            file_path,
            format,
            state: RecordingState::Recording,
            writer: Arc::new(Mutex::new(writer)),
            start_time: Instant::now(),
            duration: Arc::new(RwLock::new(Duration::ZERO)),
        });

        self.recordings.insert(id, recording);
        Ok(id)
    }

    pub async fn add_packet(&self, endpoint_id: EndpointId, packet: &[u8]) {
        // Find active recordings for this endpoint
        for entry in self.recordings.iter() {
            let recording = entry.value();

            if recording.endpoint_id == endpoint_id
                && recording.state == RecordingState::Recording {
                // Decode RTP
                if let Ok(rtp) = RtpPacket::parse(packet) {
                    // Extract audio payload
                    let audio = self.decode_payload(&rtp).await;

                    // Write to file
                    recording.writer.lock().await.write_samples(&audio).await;

                    // Update duration
                    let duration = recording.start_time.elapsed();
                    *recording.duration.write().await = duration;
                }
            }
        }
    }

    pub async fn stop_recording(&self, id: RecordingId) -> Result<RecordingFile> {
        if let Some((_, recording)) = self.recordings.remove(&id) {
            // Finalize file
            recording.writer.lock().await.finalize().await?;

            // Upload to storage
            let url = self.storage.upload(&recording.file_path).await?;

            Ok(RecordingFile {
                id: recording.id,
                url,
                format: recording.format,
                duration: *recording.duration.read().await,
            })
        } else {
            Err(Error::RecordingNotFound)
        }
    }
}
```

### 6. IVR Player Engine

```rust
// media-server-core/src/player.rs
pub struct PlayerEngine {
    play_sessions: Arc<DashMap<PlaybackId, Arc<PlaybackSession>>>,
    prompt_cache: Arc<PromptCache>,
}

pub struct PlaybackSession {
    pub id: PlaybackId,
    pub endpoint_id: EndpointId,
    pub file_path: PathBuf,
    pub reader: Arc<Mutex<AudioReader>>,
    pub state: PlaybackState,
    pub loop_count: Option<u32>,
    pub current_loop: AtomicU32,
}

impl PlayerEngine {
    pub async fn play_file(
        &self,
        endpoint_id: EndpointId,
        file: &str,
        options: PlaybackOptions,
    ) -> Result<PlaybackId> {
        let id = PlaybackId::new();

        // Load file (check cache first)
        let audio_data = self.prompt_cache.get_or_load(file).await?;

        let session = Arc::new(PlaybackSession {
            id,
            endpoint_id,
            file_path: PathBuf::from(file),
            reader: Arc::new(Mutex::new(AudioReader::from_data(audio_data))),
            state: PlaybackState::Playing,
            loop_count: options.loop_count,
            current_loop: AtomicU32::new(0),
        });

        self.play_sessions.insert(id, session.clone());

        // Start playback task
        self.spawn_playback_task(session).await;

        Ok(id)
    }

    async fn spawn_playback_task(&self, session: Arc<PlaybackSession>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(20));

            loop {
                interval.tick().await;

                if session.state != PlaybackState::Playing {
                    break;
                }

                // Read next audio chunk
                let audio = session.reader.lock().await.read_chunk(160).await;

                match audio {
                    Some(samples) => {
                        // Send to endpoint
                        session.send_audio_to_endpoint(samples).await;
                    }
                    None => {
                        // End of file
                        if let Some(max_loops) = session.loop_count {
                            let current = session.current_loop.fetch_add(1, Ordering::SeqCst);
                            if current < max_loops {
                                // Restart playback
                                session.reader.lock().await.seek_to_start().await;
                            } else {
                                // Playback complete
                                session.state = PlaybackState::Completed;
                                break;
                            }
                        } else {
                            // Single play complete
                            session.state = PlaybackState::Completed;
                            break;
                        }
                    }
                }
            }
        });
    }
}

pub struct PromptCache {
    cache: Arc<DashMap<String, Arc<Vec<i16>>>>,
    max_size: usize,
}

impl PromptCache {
    pub async fn get_or_load(&self, file: &str) -> Result<Arc<Vec<i16>>> {
        if let Some(cached) = self.cache.get(file) {
            return Ok(cached.clone());
        }

        // Load from disk
        let audio = self.load_audio_file(file).await?;
        let audio_arc = Arc::new(audio);

        // Cache it
        self.cache.insert(file.to_string(), audio_arc.clone());

        Ok(audio_arc)
    }
}
```

### 7. DTMF Detection

```rust
// media-server-core/src/dtmf.rs
pub struct DtmfDetector {
    detectors: Arc<DashMap<EndpointId, Arc<GoertzelDetector>>>,
}

pub struct GoertzelDetector {
    endpoint_id: EndpointId,
    sample_rate: u32,
    detectors: [GoertzelFilter; 8], // DTMF frequencies
    buffer: Vec<f32>,
    last_digit: Option<char>,
    digit_callback: Arc<dyn Fn(char) + Send + Sync>,
}

impl DtmfDetector {
    pub async fn start_detection(
        &self,
        endpoint_id: EndpointId,
        callback: impl Fn(char) + Send + Sync + 'static,
    ) {
        let detector = Arc::new(GoertzelDetector::new(
            endpoint_id,
            8000,
            Arc::new(callback),
        ));

        self.detectors.insert(endpoint_id, detector);
    }

    pub async fn process_audio(&self, endpoint_id: EndpointId, samples: &[i16]) {
        if let Some(detector) = self.detectors.get(&endpoint_id) {
            detector.process(samples).await;
        }
    }
}

impl GoertzelDetector {
    pub async fn process(&self, samples: &[i16]) {
        // Convert to float
        let float_samples: Vec<f32> = samples.iter()
            .map(|&s| s as f32 / 32768.0)
            .collect();

        // Run Goertzel algorithm
        let mut powers = [0.0; 8];
        for (i, detector) in self.detectors.iter().enumerate() {
            powers[i] = detector.process(&float_samples);
        }

        // Detect DTMF digit
        if let Some(digit) = self.detect_digit(&powers) {
            if self.last_digit != Some(digit) {
                (self.digit_callback)(digit);
                self.last_digit = Some(digit);
            }
        } else {
            self.last_digit = None;
        }
    }

    fn detect_digit(&self, powers: &[f32; 8]) -> Option<char> {
        // DTMF detection logic
        const DTMF_CHARS: [[char; 4]; 4] = [
            ['1', '2', '3', 'A'],
            ['4', '5', '6', 'B'],
            ['7', '8', '9', 'C'],
            ['*', '0', '#', 'D'],
        ];

        // Find strongest low and high frequencies
        let (low_idx, _) = powers[0..4].iter().enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())?;
        let (high_idx, _) = powers[4..8].iter().enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())?;

        // Threshold check
        if powers[low_idx] > 0.1 && powers[4 + high_idx] > 0.1 {
            Some(DTMF_CHARS[low_idx][high_idx])
        } else {
            None
        }
    }
}
```

### 8. Control API

```rust
// media-server-core/src/api/mod.rs
pub struct ApiServer {
    rest_server: Option<RestApiServer>,
    grpc_server: Option<GrpcApiServer>,
    media_server: Arc<MediaServerCore>,
}

// REST API
impl RestApiServer {
    pub fn routes(&self) -> Router {
        Router::new()
            .route("/endpoints", post(Self::allocate_endpoint))
            .route("/endpoints/:id", delete(Self::release_endpoint))
            .route("/bridges", post(Self::create_bridge))
            .route("/conferences", post(Self::create_conference))
            .route("/recordings", post(Self::start_recording))
            .route("/playback", post(Self::play_file))
    }

    async fn allocate_endpoint(
        State(server): State<Arc<MediaServerCore>>,
        Json(req): Json<AllocateEndpointRequest>,
    ) -> Result<Json<EndpointResponse>> {
        let endpoint = server.endpoint_pool.allocate().await?;

        Ok(Json(EndpointResponse {
            id: endpoint.id,
            rtp_port: endpoint.rtp_port,
            rtcp_port: endpoint.rtcp_port,
            ip: endpoint.local_ip,
        }))
    }
}

// gRPC API
#[tonic::async_trait]
impl MediaService for GrpcApiServer {
    async fn allocate_endpoint(
        &self,
        request: Request<AllocateRequest>,
    ) -> Result<Response<EndpointInfo>, Status> {
        let endpoint = self.media_server.endpoint_pool.allocate().await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(EndpointInfo {
            id: endpoint.id.to_string(),
            rtp_port: endpoint.rtp_port as u32,
            rtcp_port: endpoint.rtcp_port as u32,
            ip: endpoint.local_ip.to_string(),
        }))
    }
}
```

## Integration with b2bua-core

### b2bua-core Client Implementation

```rust
// In b2bua-core
pub struct MediaServerClient {
    base_url: Url,
    client: reqwest::Client,
}

impl MediaServerController for MediaServerClient {
    async fn allocate_endpoint(&self) -> Result<MediaEndpoint> {
        let response = self.client
            .post(format!("{}/endpoints", self.base_url))
            .json(&AllocateEndpointRequest::default())
            .send()
            .await?;

        let endpoint: EndpointResponse = response.json().await?;

        Ok(MediaEndpoint {
            id: endpoint.id,
            sdp: self.generate_sdp(&endpoint),
        })
    }

    async fn bridge(&self, a: EndpointId, b: EndpointId) -> Result<BridgeId> {
        let response = self.client
            .post(format!("{}/bridges", self.base_url))
            .json(&BridgeRequest {
                endpoint_a: a,
                endpoint_b: b,
            })
            .send()
            .await?;

        let bridge: BridgeResponse = response.json().await?;
        Ok(bridge.id)
    }
}
```

## Deployment Architecture

### Standalone Deployment

```yaml
# docker-compose.yml
version: '3.8'

services:
  media-server-1:
    image: rvoip/media-server:latest
    ports:
      - "8080:8080"  # API
      - "10000-10999:10000-10999/udp"  # RTP
    environment:
      - RTP_PORT_START=10000
      - RTP_PORT_END=10999
      - API_PORT=8080

  media-server-2:
    image: rvoip/media-server:latest
    ports:
      - "8081:8080"  # API
      - "11000-11999:11000-11999/udp"  # RTP
    environment:
      - RTP_PORT_START=11000
      - RTP_PORT_END=11999
      - API_PORT=8080

  load-balancer:
    image: nginx
    ports:
      - "80:80"
    volumes:
      - ./nginx.conf:/etc/nginx/nginx.conf
```

### Kubernetes Deployment

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: media-server
spec:
  replicas: 5
  selector:
    matchLabels:
      app: media-server
  template:
    metadata:
      labels:
        app: media-server
    spec:
      containers:
      - name: media-server
        image: rvoip/media-server:latest
        ports:
        - containerPort: 8080
          name: api
        - containerPort: 10000-10999
          protocol: UDP
          name: rtp
        resources:
          requests:
            memory: "1Gi"
            cpu: "2"
          limits:
            memory: "2Gi"
            cpu: "4"
```

## Performance Optimization

### 1. Zero-Copy RTP Processing

```rust
impl RtpTransport {
    async fn process_packet_zero_copy(&self, packet: &[u8]) {
        // Parse in place
        let rtp = RtpPacketRef::from_slice(packet)?;

        // Process without copying
        match self.endpoints.get_by_ssrc(rtp.ssrc()) {
            Some(endpoint) => {
                // Direct forward without copy
                self.forward_packet_ref(rtp, endpoint).await;
            }
            None => {
                // Unknown SSRC
                self.handle_unknown_ssrc(rtp).await;
            }
        }
    }
}
```

### 2. SIMD Audio Mixing

```rust
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

impl AudioMixer {
    unsafe fn mix_simd(&mut self, streams: &[Vec<i16>]) -> Vec<i16> {
        // Use AVX2 for parallel mixing
        let mut result = vec![0i16; 160];

        for stream in streams {
            for i in (0..160).step_by(16) {
                let a = _mm256_loadu_si256(stream[i..].as_ptr() as *const __m256i);
                let b = _mm256_loadu_si256(result[i..].as_ptr() as *const __m256i);
                let sum = _mm256_adds_epi16(a, b);
                _mm256_storeu_si256(result[i..].as_mut_ptr() as *mut __m256i, sum);
            }
        }

        result
    }
}
```

### 3. Lock-Free Structures

```rust
use crossbeam::queue::ArrayQueue;

pub struct LockFreeEndpointPool {
    available: ArrayQueue<EndpointId>,
    endpoints: Arc<DashMap<EndpointId, Endpoint>>,
}

impl LockFreeEndpointPool {
    pub fn allocate(&self) -> Option<EndpointId> {
        self.available.pop()
    }

    pub fn release(&self, id: EndpointId) {
        let _ = self.available.push(id);
    }
}
```

## Testing Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn test_endpoint_allocation() {
        let pool = EndpointPool::new(10000..11000);

        let endpoint = pool.allocate().await.unwrap();
        assert!(endpoint.rtp_port >= 10000 && endpoint.rtp_port < 11000);

        pool.release(endpoint.id).await.unwrap();
    }

    #[test]
    fn test_audio_mixing() {
        let mut mixer = AudioMixer::new(8000);

        let stream1 = vec![100i16; 160];
        let stream2 = vec![200i16; 160];

        let mixed = mixer.mix(&[
            (EndpointId::new(), stream1),
            (EndpointId::new(), stream2),
        ]);

        assert_eq!(mixed[0], 150); // Average
    }

    #[test]
    fn test_dtmf_detection() {
        let detector = GoertzelDetector::new(8000);

        // Generate DTMF tone for '5'
        let samples = generate_dtmf_tone('5', 8000, 100);

        let digit = detector.detect(&samples);
        assert_eq!(digit, Some('5'));
    }
}
```

### Load Testing

```rust
#[tokio::test]
async fn test_concurrent_conferences() {
    let server = MediaServerCore::new_test();

    // Create 100 conferences with 10 participants each
    let mut conferences = Vec::new();

    for _ in 0..100 {
        let conf_id = server.create_conference().await.unwrap();

        for _ in 0..10 {
            let endpoint = server.allocate_endpoint().await.unwrap();
            server.add_to_conference(conf_id, endpoint.id).await.unwrap();
        }

        conferences.push(conf_id);
    }

    // Verify all conferences are active
    assert_eq!(server.active_conferences(), 100);
    assert_eq!(server.active_endpoints(), 1000);
}
```

## Timeline

### Week 1-2: Core Infrastructure
- Endpoint pool management
- RTP transport layer
- Basic packet routing

### Week 3-4: Processing Engines
- Mixer engine
- Recording engine
- Player engine

### Week 5-6: Advanced Features
- DTMF detection
- Conference management
- IVR prompts

### Week 7-8: API & Integration
- REST API
- gRPC API
- b2bua-core integration

### Week 9-10: Optimization & Testing
- Performance optimization
- Load testing
- Documentation

## Success Metrics

1. **Performance**
   - Support 10,000+ concurrent endpoints
   - < 5ms processing latency
   - < 1% packet loss

2. **Scalability**
   - Horizontal scaling to N instances
   - Load balancing across instances
   - Graceful failover

3. **Reliability**
   - 99.99% uptime
   - Automatic recovery
   - Health monitoring

4. **Features**
   - All B2BUA media requirements met
   - Clean API for control
   - Comprehensive documentation