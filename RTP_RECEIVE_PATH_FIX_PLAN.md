# RTP Receive Path Fix Plan

## Problem Summary
After 4 months of investigation, discovered that the RTP receive path is completely missing. Audio is being sent successfully but received RTP packets are not being decoded and delivered to clients.

## Current Status
- ✅ Client → session-core → media-core → RTP send path works
- ❌ RTP receive → media-core decode → session-core → client path is broken
- Media-core receives RTP packets but doesn't decode them
- Session-core has its own RTP decoder that shouldn't be used

## Architecture Overview

### Intended Flow
```
SEND:   Client --PCM--> session-core --PCM--> media-core --G.711/RTP--> Network
RECEIVE: Client <--PCM-- session-core <--PCM-- media-core <--G.711/RTP-- Network
```

### Current Issue
Media-core receives RTP packets but doesn't decode them or trigger audio frame callbacks.

## Fix Plan

### Phase 1: Remove session-core RTP decoder
**Goal:** Remove duplicate/unused RTP decoder from session-core

1. **Delete file:** `/crates/session-core/src/media/rtp_decoder.rs`
   - Contains unused `RtpPayloadDecoder` that duplicates media-core functionality

2. **Update:** `/crates/session-core/src/media/mod.rs`
   - Remove: `pub mod rtp_decoder;`

3. **Update:** `/crates/session-core/src/media/manager.rs`
   - Remove: `rtp_decoder: Arc<Mutex<RtpPayloadDecoder>>` field
   - Remove: All `self.rtp_decoder` references
   - Remove: `start_rtp_event_processing()` method (lines ~1067-1117)
   - Remove: `create_rtp_event_callback()` method (lines ~1206-1246)
   - Remove: `initialize_rtp_event_integration()` method (lines ~1195-1202)
   - Update: `set_audio_frame_callback()` to only use media-core callback (remove lines 876-879)

### Phase 2: Investigate media-core RTP receive path
**Goal:** Understand how media-core currently handles RTP reception

1. **Examine:** `/crates/media-core/src/relay/controller.rs`
   - Find RTP packet reception code
   - Look for decoding logic
   - Find audio frame callback trigger points

2. **Examine:** `/crates/media-core/src/relay/controller/rtp_management.rs`
   - Check RTP session management
   - Look for receive packet handling

3. **Examine:** `/crates/media-core/src/integration/rtp_bridge.rs`
   - Check if this should bridge RTP to audio frames

### Phase 3: Fix media-core to decode and deliver frames
**Goal:** Make media-core decode received RTP and trigger callbacks

1. **Find:** Where "New RTP stream detected" is logged
   - This is where RTP packets arrive

2. **Add/Fix:** Decoding pipeline
   ```rust
   // Pseudo-code for what needs to happen:
   on_rtp_packet_received(packet) {
       // 1. Extract payload from RTP packet
       let payload = packet.payload();
       
       // 2. Decode based on payload type
       let pcm_samples = match packet.payload_type() {
           0 => decode_g711_ulaw(payload),
           8 => decode_g711_alaw(payload),
           _ => return,
       };
       
       // 3. Create AudioFrame
       let audio_frame = AudioFrame {
           samples: pcm_samples,
           timestamp: packet.timestamp(),
           // ... other fields
       };
       
       // 4. Trigger callback if registered
       if let Some(callback) = self.audio_frame_callback.get(&dialog_id) {
           callback.send(audio_frame).await;
       }
   }
   ```

3. **Ensure:** Callbacks are triggered for each decoded frame

### Phase 4: Test and Verify
**Goal:** Confirm the fix works end-to-end

1. Run the audio exchange test
2. Verify:
   - Output WAV files contain audio (not just headers)
   - Both directions work (A→B and B→A)
   - Frame counts show frames being received
   - No audio quality issues

## Success Criteria
- [x] Session-core RTP decoder removed
- [x] Media-core decodes received RTP packets
- [x] Audio frame callbacks are triggered
- [x] Test shows bidirectional audio exchange
- [x] Output WAV files contain the expected tones

## Implementation Details Found

### The Issue
1. RTP sessions are created in media-core but don't subscribe to PacketReceived events
2. When RTP packets arrive, they're logged ("New RTP stream detected") but not decoded
3. The audio frame callbacks exist but are never triggered with decoded frames

### The Fix Required
In `/crates/media-core/src/relay/controller/mod.rs`:
1. After creating RTP session, subscribe to its events
2. When PacketReceived events arrive:
   - Extract the RTP payload
   - Decode based on payload type (0=PCMU, 8=PCMA)
   - Create AudioFrame with decoded PCM samples
   - Send through registered audio frame callbacks

## Notes
- The sending path works perfectly, so focus only on receive
- Media-core already has G.711 codecs, just need to use them
- The callback mechanism exists, just needs to be triggered

## Fix Summary
The RTP receive path has been successfully implemented. The key changes were:

1. **Added RTP event subscription** in `MediaSessionController::start_media()` to get packet events
2. **Implemented `spawn_rtp_event_handler()`** that:
   - Subscribes to RTP session events
   - Decodes G.711 μ-law/A-law packets to PCM
   - Sends decoded AudioFrames through registered callbacks
3. **Fixed callback timing issue** by checking for callbacks inside the event loop rather than before spawning

The test now successfully exchanges bidirectional audio:
- Session A sends 440Hz and receives 880Hz 
- Session B sends 880Hz and receives 440Hz
- Output WAV files contain actual audio data, not just headers