//! RTP session management functionality
//!
//! This module handles all RTP-related operations including session management,
//! packet transmission, remote address updates, and media flow control.
//!
//! # Muting Behavior
//!
//! Audio muting is implemented by sending silence packets rather than stopping
//! RTP transmission. This approach:
//! - Maintains continuous RTP flow preventing NAT timeouts
//! - Preserves sequence numbers and timestamps
//! - Ensures compatibility with all SIP endpoints
//! - Provides instant mute/unmute without renegotiation
//!
//! When `set_audio_muted(true)` is called, subsequent audio frames are replaced
//! with silence (PCM zeros) before encoding and transmission.

use bytes::Bytes;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::codec::audio::common::AudioCodec;
use crate::error::{CodecError, Error, Result};
use crate::types::DialogId;
use rvoip_rtp_core::RtpSession;

use super::{
    audio_generation::{AudioSource, AudioTransmitter, AudioTransmitterConfig},
    MediaSessionController,
};

impl MediaSessionController {
    /// Get RTP session for a dialog (for packet transmission)
    pub async fn get_rtp_session(
        &self,
        dialog_id: &DialogId,
    ) -> Option<Arc<tokio::sync::Mutex<RtpSession>>> {
        let rtp_sessions = self.rtp_sessions.read().await;
        rtp_sessions
            .get(dialog_id)
            .map(|wrapper| wrapper.session.clone())
    }

    /// Send RTP packet for a dialog
    pub async fn send_rtp_packet(
        &self,
        dialog_id: &DialogId,
        payload: Vec<u8>,
        timestamp: u32,
    ) -> Result<()> {
        let rtp_session = self
            .get_rtp_session(dialog_id)
            .await
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;

        let mut session = rtp_session.lock().await;
        let payload_len = payload.len();
        session
            .send_packet(timestamp, Bytes::from(payload), false)
            .await
            .map_err(|e| Error::config(format!("Failed to send RTP packet: {}", e)))?;

        info!(
            "📤 Sent RTP packet for dialog: {} (timestamp: {}, payload: {} bytes)",
            dialog_id, timestamp, payload_len
        );
        Ok(())
    }

    /// Send a DTMF digit per RFC 4733 §2.5.
    ///
    /// Spawns a [`DtmfTransmitter`] task that emits the full packet
    /// schedule — start (E=0, marker=1) + 20 ms continuations
    /// (incrementing duration, fixed timestamp) + three E=1
    /// retransmits (RFC 4733 §2.5.1.3). The receive-side dedup at
    /// `rtp-core::transport::udp` collapses the three retransmits
    /// into one logical digit downstream.
    ///
    /// Fire-and-forget: the spawned task is dropped, so the caller
    /// returns as soon as the schedule is armed — critical for
    /// softphone UX where a key-down handler should not block on the
    /// full tone duration.
    ///
    /// Unknown digits fall back to event code 0 (DTMF '0' interpretation
    /// is implementation-defined for non-DTMF events; receivers MAY
    /// ignore).
    pub async fn send_dtmf_packet(
        &self,
        dialog_id: &DialogId,
        digit: char,
        duration_ms: u32,
    ) -> Result<()> {
        use super::dtmf_transmitter::DtmfTransmitter;

        let rtp_session = self
            .get_rtp_session(dialog_id)
            .await
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;

        let transmitter = DtmfTransmitter::new(rtp_session);
        let _handle = transmitter.send_digit(digit, duration_ms);
        // Drop the handle — fire-and-forget. The schedule runs to
        // completion in the background and logs any wire-level send
        // failures via the transmitter's tracing instrumentation.
        info!(
            "☎️  RFC 4733 DTMF '{}' scheduled (duration={}ms) for dialog {}",
            digit, duration_ms, dialog_id
        );
        Ok(())
    }

    /// Update remote address for RTP session
    pub async fn update_rtp_remote_addr(
        &self,
        dialog_id: &DialogId,
        remote_addr: SocketAddr,
    ) -> Result<()> {
        let rtp_session = self
            .get_rtp_session(dialog_id)
            .await
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;

        let mut session = rtp_session.lock().await;
        session.set_remote_addr(remote_addr).await;

        // Update wrapper info
        {
            let mut rtp_sessions = self.rtp_sessions.write().await;
            if let Some(wrapper) = rtp_sessions.get_mut(dialog_id) {
                wrapper.remote_addr = Some(remote_addr);
            }
        }

        info!(
            "✅ Updated RTP remote address for dialog: {} -> {}",
            dialog_id, remote_addr
        );
        Ok(())
    }

    /// Set remote address and start audio transmission (called when call is established)
    pub async fn establish_media_flow(
        &self,
        dialog_id: &DialogId,
        remote_addr: SocketAddr,
    ) -> Result<()> {
        info!(
            "🔗 Establishing media flow for dialog: {} -> {}",
            dialog_id, remote_addr
        );

        // Update remote address
        self.update_rtp_remote_addr(dialog_id, remote_addr).await?;

        // Start audio transmission in pass-through mode by default
        self.start_audio_transmission(dialog_id).await?;

        info!("✅ Media flow established for dialog: {}", dialog_id);
        Ok(())
    }

    /// Terminate media flow (called when call ends)
    pub async fn terminate_media_flow(&self, dialog_id: &DialogId) -> Result<()> {
        info!("🛑 Terminating media flow for dialog: {}", dialog_id);

        // Stop audio transmission
        self.stop_audio_transmission(dialog_id).await?;

        // Clean up advanced processors if they exist
        {
            let mut processors = self.advanced_processors.write().await;
            if processors.remove(dialog_id).is_some() {
                info!(
                    "🧹 Cleaned up advanced processors for dialog: {}",
                    dialog_id
                );
            }
        }

        info!("✅ Media flow terminated for dialog: {}", dialog_id);
        Ok(())
    }

    /// Start audio transmission for a dialog with default configuration (pass-through mode)
    pub async fn start_audio_transmission(&self, dialog_id: &DialogId) -> Result<()> {
        let config = AudioTransmitterConfig::default(); // Uses pass-through mode
        self.start_audio_transmission_with_config(dialog_id, config)
            .await
    }

    /// Start audio transmission for a dialog with tone generation (for backward compatibility)
    pub async fn start_audio_transmission_with_tone(&self, dialog_id: &DialogId) -> Result<()> {
        let config = AudioTransmitterConfig {
            source: AudioSource::Tone {
                frequency: 440.0,
                amplitude: 0.5,
            },
            ..Default::default()
        };
        self.start_audio_transmission_with_config(dialog_id, config)
            .await
    }

    /// Start audio transmission for a dialog with custom configuration
    pub async fn start_audio_transmission_with_config(
        &self,
        dialog_id: &DialogId,
        config: AudioTransmitterConfig,
    ) -> Result<()> {
        info!("🎵 Starting audio transmission for dialog: {}", dialog_id);

        let mut rtp_sessions = self.rtp_sessions.write().await;
        let wrapper = rtp_sessions
            .get_mut(dialog_id)
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;

        if wrapper.transmission_enabled {
            return Ok(()); // Already started
        }

        // Create audio transmitter with custom configuration
        let mut audio_transmitter =
            AudioTransmitter::new_with_config(wrapper.session.clone(), config);
        audio_transmitter.start().await;

        wrapper.audio_transmitter = Some(audio_transmitter);
        wrapper.transmission_enabled = true;

        info!("✅ Audio transmission started for dialog: {}", dialog_id);
        Ok(())
    }

    /// Stop audio transmission for a dialog
    pub async fn stop_audio_transmission(&self, dialog_id: &DialogId) -> Result<()> {
        info!("🛑 Stopping audio transmission for dialog: {}", dialog_id);

        let mut rtp_sessions = self.rtp_sessions.write().await;
        let wrapper = rtp_sessions
            .get_mut(dialog_id)
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;

        if let Some(transmitter) = &wrapper.audio_transmitter {
            transmitter.stop().await;
        }

        wrapper.audio_transmitter = None;
        wrapper.transmission_enabled = false;

        info!("✅ Audio transmission stopped for dialog: {}", dialog_id);
        Ok(())
    }

    /// Set audio muted state for a dialog (send silence when muted)
    ///
    /// When muted, the RTP stream continues but audio frames are replaced with
    /// silence before encoding. This maintains RTP flow and prevents issues with
    /// NAT traversal, session timers, and remote endpoint timeout detection.
    ///
    /// # Arguments
    ///
    /// * `dialog_id` - The dialog/session to mute or unmute
    /// * `muted` - `true` to mute (send silence), `false` to unmute (send actual audio)
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the mute state was successfully updated, or an error if
    /// the dialog was not found.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use rvoip_media_core::relay::controller::MediaSessionController;
    /// # use rvoip_media_core::types::DialogId;
    /// # async fn example(controller: &MediaSessionController) -> Result<(), Box<dyn std::error::Error>> {
    /// let dialog_id = DialogId::new("call-123");
    ///
    /// // Mute the microphone (start sending silence)
    /// controller.set_audio_muted(&dialog_id, true).await?;
    ///
    /// // Later, unmute to resume normal audio
    /// controller.set_audio_muted(&dialog_id, false).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn set_audio_muted(&self, dialog_id: &DialogId, muted: bool) -> Result<()> {
        info!("🔇 Setting audio muted={} for dialog: {}", muted, dialog_id);

        let mut rtp_sessions = self.rtp_sessions.write().await;
        let wrapper = rtp_sessions
            .get_mut(dialog_id)
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;

        wrapper.is_muted = muted;

        info!("✅ Audio muted={} set for dialog: {}", muted, dialog_id);
        Ok(())
    }

    /// Check if audio transmission is active for a dialog
    pub async fn is_audio_transmission_active(&self, dialog_id: &DialogId) -> bool {
        let rtp_sessions = self.rtp_sessions.read().await;
        if let Some(wrapper) = rtp_sessions.get(dialog_id) {
            if let Some(transmitter) = &wrapper.audio_transmitter {
                return transmitter.is_active().await;
            }
        }
        false
    }

    /// Set custom audio samples for transmission
    pub async fn set_custom_audio(
        &self,
        dialog_id: &DialogId,
        samples: Vec<u8>,
        repeat: bool,
    ) -> Result<()> {
        info!(
            "🎵 Setting custom audio for dialog: {} ({} samples, repeat: {})",
            dialog_id,
            samples.len(),
            repeat
        );

        let rtp_sessions = self.rtp_sessions.read().await;
        let wrapper = rtp_sessions
            .get(dialog_id)
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;

        if let Some(transmitter) = &wrapper.audio_transmitter {
            transmitter.set_custom_audio(samples, repeat).await;
            info!("✅ Custom audio set for dialog: {}", dialog_id);
        } else {
            return Err(Error::config(
                "Audio transmission not active for dialog".to_string(),
            ));
        }

        Ok(())
    }

    /// Set tone generation parameters for a dialog
    pub async fn set_tone_generation(
        &self,
        dialog_id: &DialogId,
        frequency: f64,
        amplitude: f64,
    ) -> Result<()> {
        info!(
            "🎵 Setting tone generation for dialog: {} ({}Hz, amplitude: {})",
            dialog_id, frequency, amplitude
        );

        let rtp_sessions = self.rtp_sessions.read().await;
        let wrapper = rtp_sessions
            .get(dialog_id)
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;

        if let Some(transmitter) = &wrapper.audio_transmitter {
            transmitter.set_tone(frequency, amplitude).await;
            info!("✅ Tone generation set for dialog: {}", dialog_id);
        } else {
            return Err(Error::config(
                "Audio transmission not active for dialog".to_string(),
            ));
        }

        Ok(())
    }

    /// Set an arbitrary [`AudioSource`] on the running transmitter for this
    /// dialog. Used by session-core early-media flows to swap silence
    /// for a caller-chosen ringback tone / hold announcement after
    /// `start_audio_transmission_with_config` has already established the
    /// transmitter.
    ///
    /// Errors if no transmitter is active for the dialog — call
    /// [`start_audio_transmission`](Self::start_audio_transmission) or
    /// [`establish_media_flow`](Self::establish_media_flow) first.
    pub async fn set_audio_source(&self, dialog_id: &DialogId, source: AudioSource) -> Result<()> {
        info!("🎵 Setting audio source for dialog: {}", dialog_id);

        let rtp_sessions = self.rtp_sessions.read().await;
        let wrapper = rtp_sessions
            .get(dialog_id)
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;

        if let Some(transmitter) = &wrapper.audio_transmitter {
            transmitter.set_audio_source(source).await;
            debug!("✅ Audio source updated for dialog: {}", dialog_id);
            Ok(())
        } else {
            Err(Error::config(format!(
                "Audio transmission not active for dialog {} — call start_audio_transmission first",
                dialog_id
            )))
        }
    }

    /// Enable pass-through mode for a dialog (no audio generation)
    pub async fn set_pass_through_mode(&self, dialog_id: &DialogId) -> Result<()> {
        info!("🔄 Setting pass-through mode for dialog: {}", dialog_id);

        let rtp_sessions = self.rtp_sessions.read().await;
        let wrapper = rtp_sessions
            .get(dialog_id)
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;

        if let Some(transmitter) = &wrapper.audio_transmitter {
            transmitter.set_pass_through().await;
            info!("✅ Pass-through mode enabled for dialog: {}", dialog_id);
        } else {
            return Err(Error::config(
                "Audio transmission not active for dialog".to_string(),
            ));
        }

        Ok(())
    }

    /// Start audio transmission with custom audio samples
    pub async fn start_audio_transmission_with_custom_audio(
        &self,
        dialog_id: &DialogId,
        samples: Vec<u8>,
        repeat: bool,
    ) -> Result<()> {
        let config = AudioTransmitterConfig {
            source: AudioSource::CustomSamples { samples, repeat },
            ..Default::default()
        };
        self.start_audio_transmission_with_config(dialog_id, config)
            .await
    }

    /// Encode and send audio frame (for session-core to delegate encoding)
    ///
    /// This method accepts raw PCM audio, encodes it using the configured codec,
    /// and sends it via RTP. If the session is muted, the audio samples are replaced
    /// with silence before encoding to maintain continuous RTP flow.
    ///
    /// # Arguments
    ///
    /// * `dialog_id` - The dialog/session to send audio for
    /// * `pcm_samples` - Raw 16-bit PCM audio samples
    /// * `timestamp` - RTP timestamp for the audio frame
    ///
    /// # Behavior
    ///
    /// - If `transmission_enabled` is false, the frame is dropped entirely
    /// - If `is_muted` is true, the PCM samples are replaced with zeros (silence)
    /// - The (possibly silenced) audio is then encoded according to the session's codec
    /// - The encoded packet is sent via RTP
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use rvoip_media_core::relay::controller::MediaSessionController;
    /// # use rvoip_media_core::types::DialogId;
    /// # async fn example(controller: &MediaSessionController) -> Result<(), Box<dyn std::error::Error>> {
    /// let dialog_id = DialogId::new("call-123");
    /// let audio_samples = vec![0i16; 160]; // 20ms of audio at 8kHz
    /// let timestamp = 12345u32;
    ///
    /// // This will send silence if muted, or the actual audio if not muted
    /// controller.encode_and_send_audio_frame(&dialog_id, audio_samples, timestamp).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn encode_and_send_audio_frame(
        &self,
        dialog_id: &DialogId,
        pcm_samples: Vec<i16>,
        timestamp: u32,
    ) -> Result<()> {
        info!(
            "🎯 encode_and_send_audio_frame called for dialog: {} with {} samples",
            dialog_id,
            pcm_samples.len()
        );

        // Check if transmission is enabled and if audio is muted
        let (is_muted, is_enabled) = {
            let rtp_sessions = self.rtp_sessions.read().await;
            if let Some(wrapper) = rtp_sessions.get(dialog_id) {
                info!(
                    "✅ Found RTP session for dialog: {}, muted={}, enabled={}",
                    dialog_id, wrapper.is_muted, wrapper.transmission_enabled
                );
                (wrapper.is_muted, wrapper.transmission_enabled)
            } else {
                warn!(
                    "⚠️ No RTP session found for dialog: {} - using defaults",
                    dialog_id
                );
                (false, true)
            }
        };

        if !is_enabled {
            // Transmission is disabled, don't send anything
            info!(
                "🔇 Audio transmission disabled for dialog: {}, dropping frame",
                dialog_id
            );
            return Ok(());
        }

        // Replace with silence if muted
        let pcm_samples = if is_muted {
            debug!("🔇 Audio muted for dialog: {}, sending silence", dialog_id);
            vec![0i16; pcm_samples.len()] // PCM silence is zero
        } else {
            pcm_samples
        };

        // Sprint 3.6 C1 follow-up — RFC 3389 Comfort Noise gating.
        // When CN is enabled at the controller level, run the
        // per-dialog VAD over the outgoing PCM frame and decide
        // whether to send the audio normally, suppress it (a recent
        // CN packet already covers this silence run), or emit one PT
        // 13 CN packet now and then suppress.
        if self
            .comfort_noise_enabled
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            // Build (or retrieve) the per-dialog gate. The gate's
            // CnTransmitter shares this dialog's RtpSession arc so PT
            // 13 packets ride the existing SSRC + timestamp cursor.
            let gate_arc = if let Some(existing) = self.cn_gate_state.get(dialog_id) {
                existing.value().clone()
            } else {
                let session_arc = {
                    let rtp_sessions = self.rtp_sessions.read().await;
                    rtp_sessions
                        .get(dialog_id)
                        .map(|w| w.session.clone())
                        .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?
                };
                let gate = crate::relay::controller::cn_gate::CnGate::new(session_arc)?;
                let gate_arc = Arc::new(tokio::sync::Mutex::new(gate));
                self.cn_gate_state
                    .insert(dialog_id.clone(), gate_arc.clone());
                gate_arc
            };

            let frame = crate::types::AudioFrame::new(pcm_samples.clone(), 8000, 1, timestamp);
            let decision = {
                let mut gate = gate_arc.lock().await;
                gate.process_frame(&frame)
            };
            use crate::relay::controller::cn_gate::CnGateDecision;
            match decision {
                CnGateDecision::SendAudio => {
                    // Fall through to normal encode-and-send.
                }
                CnGateDecision::SuppressAudio => {
                    debug!(
                        "RFC 3389 CN gate: suppressing audio for dialog {} (silence ongoing)",
                        dialog_id
                    );
                    return Ok(());
                }
                CnGateDecision::EmitCnThenSuppress { level } => {
                    debug!(
                        "RFC 3389 CN gate: emitting CN packet for dialog {} (level={} -dBov)",
                        dialog_id, level
                    );
                    let gate = gate_arc.lock().await;
                    if let Err(e) = gate.emit_cn_now(level).await {
                        warn!(
                            "RFC 3389 CN gate: emit_cn_now failed for dialog {}: {}",
                            dialog_id, e
                        );
                    }
                    return Ok(());
                }
            }
        }

        // Get session info to determine codec
        let codec_payload_type = {
            let sessions = self.sessions.read().await;
            info!("🔍 Looking for session for dialog: {}", dialog_id);
            let session = sessions.get(dialog_id).ok_or_else(|| {
                error!("❌ Session not found for dialog: {}", dialog_id);
                Error::session_not_found(dialog_id.as_str())
            })?;
            info!("✅ Found session for dialog: {}", dialog_id);

            // Determine payload type from configured codec
            let pt = session
                .config
                .preferred_codec
                .as_ref()
                .and_then(|codec| self.codec_mapper.codec_to_payload(codec))
                .unwrap_or(0); // Default to PCMU
            info!("📝 Using payload type {} for dialog: {}", pt, dialog_id);
            pt
        };

        // Create AudioFrame for codec interface
        let audio_frame = crate::types::AudioFrame::new(
            pcm_samples,
            8000, // Default for G.711
            1,    // Default mono
            timestamp,
        );

        // Encode based on payload type
        let encoded_payload = match codec_payload_type {
            0 => {
                // PCMU encoding using media-core's G711Codec
                let mut codec = self.g711_codec.lock().await;
                codec.encode(&audio_frame)?
            }
            8 => {
                // PCMA encoding - create temporary codec
                use crate::codec::audio::G711Codec;
                let mut codec = G711Codec::a_law(8000, 1)?;
                codec.encode(&audio_frame)?
            }
            _ => {
                // For other codecs, we would need to instantiate them here
                // For now, return an error
                return Err(Error::unsupported_payload_type(codec_payload_type));
            }
        };

        // Send the encoded packet via RTP
        info!(
            "📡 About to send RTP packet for dialog: {} with {} bytes payload",
            dialog_id,
            encoded_payload.len()
        );
        self.send_rtp_packet(dialog_id, encoded_payload, timestamp)
            .await?;

        info!(
            "✅ Encoded and sent audio frame for dialog: {} (codec PT: {}, timestamp: {})",
            dialog_id, codec_payload_type, timestamp
        );
        Ok(())
    }
}
