# SIP Hold/Resume Implementation Plan with Music-on-Hold

## Overview

This document outlines the implementation plan for SIP hold/resume functionality with music-on-hold support in the rvoip stack, following RFC 3264 and RFC 6337 requirements.

## Current State

The current implementation:
- Sends placeholder "SDP with hold attributes" text
- Updates call state but does NOT control media
- Audio continues to flow during hold (non-compliant)

## RFC Requirements

According to RFC 3264 (SDP Offer/Answer) and RFC 6337 (SIP Usage):

1. **To place a call on hold:**
   - Send re-INVITE with `a=sendonly` in SDP
   - Stop sending microphone audio to the remote party
   - Typically play music-on-hold to the remote party
   - Remote party MUST respond with `a=recvonly` or `a=inactive`

2. **To resume from hold:**
   - Send re-INVITE with `a=sendrecv` in SDP
   - Resume sending microphone audio

3. **Important rules:**
   - Each media stream is held independently
   - Don't automatically reciprocate hold (breaks third-party call control)
   - RTP packets continue to flow (with music instead of microphone audio)

## Implementation Plan

### Phase 1: Music-on-Hold Configuration

#### 1.1 Add MoH configuration
**Location:** `/rvoip/crates/session-core/src/config.rs` (update existing or create)

```rust
#[derive(Debug, Clone)]
pub struct MediaConfig {
    // Existing fields...
    
    /// Path to music-on-hold WAV file
    /// If None, silence will be sent during hold
    pub music_on_hold_path: Option<PathBuf>,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            // Default MoH file in resources or configurable path
            music_on_hold_path: Some(PathBuf::from("resources/music_on_hold.wav")),
        }
    }
}
```

#### 1.2 Add WAV file loader
**Location:** `/rvoip/crates/media-core/src/audio/wav_loader.rs` (new file)

```rust
use std::path::Path;
use hound; // Add to Cargo.toml

pub struct WavAudio {
    pub samples: Vec<i16>,
    pub sample_rate: u32,
    pub channels: u16,
}

pub fn load_wav_file(path: &Path) -> Result<WavAudio> {
    let reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    
    // Convert to i16 PCM samples
    let samples: Vec<i16> = reader.into_samples::<i16>()
        .collect::<Result<Vec<_>, _>>()?;
    
    Ok(WavAudio {
        samples,
        sample_rate: spec.sample_rate,
        channels: spec.channels,
    })
}

/// Convert WAV audio to G.711 µ-law for RTP transmission
pub fn wav_to_ulaw(wav: &WavAudio) -> Result<Vec<u8>> {
    // Resample to 8kHz if needed
    // Convert to mono if needed
    // Encode as G.711 µ-law
}
```

### Phase 2: SDP Generation and Parsing

#### 2.1 Create SDP utilities module
**Location:** `/rvoip/crates/session-core/src/media/sdp_utils.rs` (new file)

```rust
pub enum MediaDirection {
    SendRecv,
    SendOnly,
    RecvOnly,
    Inactive,
}

pub fn generate_hold_sdp(current_sdp: &str) -> Result<String> {
    // Parse current SDP
    // For each media line (m=):
    //   - Replace or add "a=sendonly" attribute
    //   - Remove any existing direction attributes
    // Return modified SDP
}

pub fn generate_resume_sdp(current_sdp: &str) -> Result<String> {
    // Parse current SDP
    // For each media line (m=):
    //   - Replace with "a=sendrecv" or remove direction attribute
    // Return modified SDP
}
```

### Phase 3: Media Control Integration with MoH

#### 3.1 Update session control for hold with music
**Location:** `/rvoip/crates/session-core/src/api/control.rs`

```rust
async fn hold_session(&self, session_id: &SessionId) -> Result<()> {
    // Existing validation...
    
    // Step 1: Load and start music-on-hold
    if let Some(moh_path) = &self.config.media_config.music_on_hold_path {
        self.start_music_on_hold(session_id, moh_path).await?;
    } else {
        // Fallback to silence if no MoH file configured
        self.media_manager.set_audio_muted(session_id, true).await?;
    }
    
    // Step 2: Send SIP re-INVITE with hold SDP
    self.dialog_manager.hold_session(session_id).await?;
    
    // Step 3: Update session state
    // ... existing code ...
    
    Ok(())
}

async fn resume_session(&self, session_id: &SessionId) -> Result<()> {
    // Existing validation...
    
    // Step 1: Send SIP re-INVITE with resume SDP
    self.dialog_manager.resume_session(session_id).await?;
    
    // Step 2: Stop MoH and resume microphone audio
    self.stop_music_on_hold(session_id).await?;
    
    // Step 3: Update session state
    // ... existing code ...
    
    Ok(())
}

// Private helper methods
impl SessionCoordinator {
    async fn start_music_on_hold(&self, session_id: &SessionId, moh_path: &Path) -> Result<()> {
        // Load WAV file
        let wav_audio = wav_loader::load_wav_file(moh_path)
            .map_err(|e| SessionError::MediaIntegration { 
                message: format!("Failed to load MoH file: {}", e) 
            })?;
        
        // Convert to G.711 µ-law
        let ulaw_samples = wav_loader::wav_to_ulaw(&wav_audio)
            .map_err(|e| SessionError::MediaIntegration { 
                message: format!("Failed to convert MoH to µ-law: {}", e) 
            })?;
        
        // Start transmitting MoH
        self.media_manager.start_audio_transmission_with_custom_audio(
            session_id,
            ulaw_samples,
            true  // repeat the music
        ).await?;
        
        Ok(())
    }
    
    async fn stop_music_on_hold(&self, session_id: &SessionId) -> Result<()> {
        // Resume normal microphone audio
        self.media_manager.start_audio_transmission(session_id).await?;
        Ok(())
    }
}
```

### Phase 4: Configuration Options

#### 4.1 Multiple configuration approaches

**Option A: Global configuration in SessionManagerBuilder**
```rust
let coordinator = SessionManagerBuilder::new()
    .with_music_on_hold_file("/path/to/music.wav")
    .build()
    .await?;
```

**Option B: Per-session configuration**
```rust
// Add to SessionParams
pub struct SessionParams {
    // Existing fields...
    pub music_on_hold_path: Option<PathBuf>,
}
```

**Option C: Runtime configuration via API**
```rust
// Add to SessionControl trait
async fn set_music_on_hold_file(&self, path: Option<PathBuf>) -> Result<()>;
```

**Recommendation**: Start with Option A (global configuration) for simplicity, with the ability to override per-session later.

### Phase 5: Error Handling

#### 5.1 Graceful fallback
```rust
async fn start_music_on_hold(&self, session_id: &SessionId, moh_path: &Path) -> Result<()> {
    match wav_loader::load_wav_file(moh_path) {
        Ok(wav_audio) => {
            // Use music-on-hold
            let ulaw_samples = wav_loader::wav_to_ulaw(&wav_audio)?;
            self.media_manager.start_audio_transmission_with_custom_audio(
                session_id, ulaw_samples, true
            ).await?;
        }
        Err(e) => {
            // Log warning and fall back to silence
            warn!("Failed to load MoH file {}: {}, using silence", moh_path.display(), e);
            self.media_manager.set_audio_muted(session_id, true).await?;
        }
    }
    Ok(())
}
```

### Phase 6: Testing

#### 6.1 Unit tests
- Test WAV file loading
- Test WAV to µ-law conversion
- Test SDP generation with sendonly/sendrecv

#### 6.2 Integration tests
- Test hold with valid MoH file
- Test hold with missing MoH file (fallback to silence)
- Test hold/resume cycle
- Verify RTP packets contain music during hold

#### 6.3 Test resources
Create test WAV files:
- `/test/resources/test_music_8khz_mono.wav` (ideal format)
- `/test/resources/test_music_44khz_stereo.wav` (needs conversion)
- `/test/resources/invalid.wav` (corrupted for error testing)

## Implementation Order

1. **Week 1**: 
   - Implement WAV loader and µ-law converter
   - Create SDP utilities
   - Add configuration structure

2. **Week 2**: 
   - Integrate MoH with session control
   - Update dialog manager for proper SDP
   - Implement error handling and fallback

3. **Week 3**: 
   - Testing and bug fixes
   - Documentation
   - Example MoH file creation

## Success Criteria

- [ ] Hold sends proper `a=sendonly` in SDP
- [ ] Music-on-hold plays from configured WAV file
- [ ] RTP packets continue during hold (with music)
- [ ] Resume sends proper `a=sendrecv` in SDP
- [ ] Microphone audio resumes after hold
- [ ] Graceful fallback to silence if MoH file is unavailable
- [ ] Music loops if shorter than hold duration
- [ ] No glitches during hold/resume transitions

## Configuration Example

```toml
# rvoip.toml or similar config file
[media]
music_on_hold = "/usr/share/rvoip/music/default_hold.wav"

# Or via environment variable
RVOIP_MUSIC_ON_HOLD=/path/to/music.wav
```

## Notes

- WAV file should ideally be 8kHz mono to avoid conversion overhead
- The implementation reuses existing `start_audio_transmission_with_custom_audio()`
- RTP stream never stops, maintaining NAT bindings and compatibility
- Consider caching the loaded/converted audio to avoid repeated file I/O