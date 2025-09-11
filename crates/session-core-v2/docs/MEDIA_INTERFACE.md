# Media Interface

This document describes the interface between session-core-v2 and media-core, clarifying the boundaries and communication patterns in the GlobalEventCoordinator-based architecture.

## Overview

The media interface manages audio/video streams and RTP sessions through:
1. **Direct Calls** - Synchronous method calls from session-core to media-core
2. **Events** - Asynchronous events from media-core to session-core via GlobalEventCoordinator

## Direct Calls (Session → Media)

These are methods that session-core calls directly on the MediaAdapter, which internally communicates with media-core:

### Session Management

#### `create_session(session_id, role)`
- **Purpose**: Create a new media session
- **When Used**: CreateMediaSession action in state machine
- **Parameters**:
  - `session_id`: Unique session identifier
  - `role`: UAC or UAS
- **Returns**: Result<MediaSessionId>
- **Side Effects**: Allocates RTP port, creates UDP socket

#### `stop_session(session_id)`
- **Purpose**: Terminate media session and cleanup
- **When Used**: StartMediaCleanup action
- **Parameters**:
  - `session_id`: Session to stop
- **Returns**: Result<()>
- **Side Effects**: Closes sockets, frees ports

### SDP Operations

#### `generate_sdp_offer(session_id)`
- **Purpose**: Create SDP offer for outbound calls
- **When Used**: GenerateLocalSDP action (UAC)
- **Parameters**:
  - `session_id`: Session needing SDP
- **Returns**: Result<String> - SDP offer
- **Details**: Includes local IP, RTP port, supported codecs

#### `negotiate_sdp_as_uac(session_id, remote_sdp)`
- **Purpose**: Process SDP answer from remote
- **When Used**: NegotiateSDPAsUAC action
- **Parameters**:
  - `session_id`: Session to update
  - `remote_sdp`: Remote party's SDP answer
- **Returns**: Result<NegotiatedConfig>
- **Side Effects**: Updates remote RTP endpoint

#### `negotiate_sdp_as_uas(session_id, remote_sdp)`
- **Purpose**: Process SDP offer and generate answer
- **When Used**: NegotiateSDPAsUAS action
- **Parameters**:
  - `session_id`: Session to update
  - `remote_sdp`: Remote party's SDP offer
- **Returns**: Result<(String, NegotiatedConfig)> - (SDP answer, config)
- **Side Effects**: Updates remote RTP endpoint

### Media Control

#### `start_media_flow(session_id)`
- **Purpose**: Begin RTP packet processing
- **When Used**: StartMediaFlow action
- **Parameters**:
  - `session_id`: Session to start
- **Returns**: Result<()>
- **Side Effects**: Starts RTP event loop

#### `stop_media_flow(session_id)`
- **Purpose**: Pause RTP processing
- **When Used**: Media hold scenarios
- **Parameters**:
  - `session_id`: Session to pause
- **Returns**: Result<()>

#### `mute_audio(session_id, muted)`
- **Purpose**: Control audio transmission
- **When Used**: MuteCall/UnmuteCall events
- **Parameters**:
  - `session_id`: Session to control
  - `muted`: true to mute, false to unmute
- **Returns**: Result<()>

### Audio Operations

#### `send_audio_frame(session_id, samples)`
- **Purpose**: Send audio data over RTP
- **When Used**: Application audio generation
- **Parameters**:
  - `session_id`: Target session
  - `samples`: PCM audio samples
- **Returns**: Result<()>
- **Side Effects**: Encodes and sends RTP packet

#### `subscribe_to_audio_frames(session_id)`
- **Purpose**: Receive decoded audio from RTP
- **When Used**: Application needs received audio
- **Parameters**:
  - `session_id`: Session to subscribe to
- **Returns**: Result<AudioFrameReceiver>
- **Details**: Returns channel for receiving PCM frames

### Statistics & Monitoring

#### `get_statistics(session_id)`
- **Purpose**: Get RTP statistics
- **When Used**: Quality monitoring
- **Parameters**:
  - `session_id`: Session to query
- **Returns**: Result<RtpStatistics>
- **Details**: Packet counts, jitter, loss percentage

## Events (Media → Session)

These events are published by media-core through the GlobalEventCoordinator and handled by session-core's SessionCrossCrateEventHandler:

### Stream Lifecycle Events

#### `MediaToSessionEvent::MediaStreamStarted`
- **When**: Media session successfully created
- **Fields**:
  - `session_id`: Session identifier
  - `local_port`: Allocated RTP port
  - `codec`: Selected codec (e.g., "PCMU")
- **Expected Action**: Update MediaSessionReady condition

#### `MediaToSessionEvent::MediaStreamStopped`
- **When**: Media session terminated
- **Fields**:
  - `session_id`: Session identifier
  - `reason`: Stop reason
- **Expected Action**: Complete cleanup

#### `MediaToSessionEvent::MediaFlowEstablished`
- **When**: Bidirectional RTP detected
- **Fields**:
  - `session_id`: Session identifier
  - `local_addr`: Local RTP address
  - `remote_addr`: Remote RTP address
- **Expected Action**: Confirm media connectivity

### Quality & Error Events

#### `MediaToSessionEvent::MediaError`
- **When**: Media processing error
- **Fields**:
  - `session_id`: Session identifier
  - `error`: Error description
  - `error_code`: Error category
- **Expected Action**: Handle error, possibly terminate

#### `MediaToSessionEvent::MediaQualityDegraded`
- **When**: Quality falls below threshold
- **Fields**:
  - `session_id`: Session identifier
  - `packet_loss`: Loss percentage
  - `jitter`: Jitter in ms
  - `severity`: Warning/Critical
- **Expected Action**: Notify user, possibly adapt

#### `MediaToSessionEvent::RtpTimeout`
- **When**: No RTP packets received
- **Fields**:
  - `session_id`: Session identifier
  - `last_packet_time`: When last packet received
- **Expected Action**: Check connectivity, possibly terminate

#### `MediaToSessionEvent::PacketLossThresholdExceeded`
- **When**: Packet loss exceeds limit
- **Fields**:
  - `session_id`: Session identifier
  - `loss_percentage`: Current loss rate
- **Expected Action**: Quality adaptation

### DTMF Events

#### `MediaToSessionEvent::DtmfDetected`
- **When**: DTMF tone in RTP (RFC2833)
- **Fields**:
  - `session_id`: Session identifier
  - `digit`: DTMF digit ('0'-'9', '*', '#')
  - `duration_ms`: Tone duration
- **Expected Action**: Process DTMF input

## Important Design Principles

1. **Codec Agnostic**: Interface doesn't expose codec details
2. **Transport Agnostic**: RTP/SRTP handled internally
3. **No Direct Callbacks**: All notifications via events
4. **Clean Separation**: Media-core owns RTP, session-core owns call logic
5. **Resource Safety**: Automatic cleanup on session drop

## SDP Negotiation Flow

### UAC (Caller) Flow:
```
1. Session-Core: create_session() → Media-Core
2. Session-Core: generate_sdp_offer() → Media-Core
3. Session-Core: (sends INVITE with SDP)
4. Session-Core: (receives 200 OK with SDP answer)
5. Session-Core: negotiate_sdp_as_uac(answer) → Media-Core
6. Media-Core: → MediaFlowEstablished → Session-Core
```

### UAS (Callee) Flow:
```
1. Session-Core: (receives INVITE with SDP offer)
2. Session-Core: create_session() → Media-Core
3. Session-Core: negotiate_sdp_as_uas(offer) → Media-Core
4. Session-Core: (sends 200 OK with SDP answer)
5. Media-Core: → MediaFlowEstablished → Session-Core
```

## Audio Flow Example

### Sending Audio:
```rust
// In application code
let samples = generate_audio_frame(); // 160 samples @ 8kHz
media_adapter.send_audio_frame(&session_id, samples)?;
```

### Receiving Audio:
```rust
// Setup
let mut audio_rx = media_adapter.subscribe_to_audio_frames(&session_id)?;

// In audio processing loop
while let Some(frame) = audio_rx.recv().await {
    process_received_audio(frame.samples);
}
```

## Error Handling

- Synchronous errors returned as `Result<T>`
- Asynchronous errors via `MediaError` events
- Network issues trigger `RtpTimeout` events
- Quality issues trigger degradation events

## Resource Management

- Ports allocated from configured range
- Automatic cleanup on session termination
- Graceful handling of port exhaustion
- Socket cleanup on error conditions
