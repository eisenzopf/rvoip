//! RFC 3389 Comfort Noise (CN) sender.
//!
//! Emits a single PT 13 packet carrying a one-byte noise-level
//! payload (RFC 3389 §3.1: noise level encoded as -dBov, range
//! 0..=127). Spectral side-information (§3.2) is not generated; the
//! receiver falls back to a flat-spectrum model.
//!
//! Designed to be invoked by the upper layer when its own VAD signals
//! a transition into silence. The single-shot shape mirrors the
//! [`super::dtmf_transmitter::DtmfTransmitter`] surface: construct
//! once per `RtpSession`, call [`CnTransmitter::send`] when silence
//! starts, optionally re-call every 200 ms while silence persists
//! (RFC 3389 §4.1).
//!
//! ## Scope cut for Sprint 3
//!
//! This module owns the wire-side primitive only. Hooking VAD into
//! the audio transmit loop in `audio_generation.rs` to fire CN
//! automatically is left as a follow-up — the loop's existing
//! "skip-send on all-silence" path doesn't generalise to the
//! VAD-on-real-audio case without a deeper refactor. The streampeer
//! example flows continue to send straight audio; deployments that
//! want bandwidth savings can drive `send_comfort_noise` from their
//! own VAD output.

use std::sync::Arc;

use bytes::Bytes;
use tokio::sync::Mutex;
use tracing::{debug, warn};

use rvoip_rtp_core::RtpSession;

use crate::codec::audio::payload_type::COMFORT_NOISE;
use crate::error::{Error, Result};

/// Default noise level emitted when the caller doesn't override.
/// 40 -dBov is "ambient room noise" — quiet enough to be perceptually
/// background, loud enough to mask packet-loss-induced silence gaps
/// in the receiver's plc model.
pub const DEFAULT_NOISE_LEVEL_DBOV: u8 = 40;

/// Single-shot Comfort Noise sender. Construct once per
/// `RtpSession`; each [`Self::send`] call emits exactly one PT 13
/// packet on the audio stream's current timestamp cursor.
pub struct CnTransmitter {
    rtp_session: Arc<Mutex<RtpSession>>,
}

impl CnTransmitter {
    pub fn new(rtp_session: Arc<Mutex<RtpSession>>) -> Self {
        Self { rtp_session }
    }

    /// Send one CN packet at the configured noise level.
    pub async fn send_default(&self) -> Result<()> {
        self.send(DEFAULT_NOISE_LEVEL_DBOV).await
    }

    /// Send one CN packet at the supplied noise level. `level` is
    /// clamped to 0..=127 (RFC 3389 §3.1 — the high bit is reserved).
    pub async fn send(&self, level: u8) -> Result<()> {
        let level = level & 0x7f;
        let payload = Bytes::copy_from_slice(&[level]);

        let timestamp = {
            let session = self.rtp_session.lock().await;
            session.current_timestamp()
        };

        let mut session = self.rtp_session.lock().await;
        session
            .send_packet_with_pt(
                timestamp,
                payload,
                /*marker*/ false,
                /*PT*/ COMFORT_NOISE,
            )
            .await
            .map_err(|e| {
                warn!("RFC 3389 CN send failed: {}", e);
                Error::config(format!("CN send failed: {}", e))
            })?;
        debug!(
            "RFC 3389 Comfort Noise emitted (PT 13, level={} -dBov, ts={})",
            level, timestamp
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_rtp_core::session::{RtpSession, RtpSessionConfig};
    use rvoip_rtp_core::traits::RtpEvent;
    use rvoip_rtp_core::transport::{RtpTransport, RtpTransportConfig, UdpRtpTransport};
    use std::time::Duration;

    /// Construct a sender RtpSession + a peer receiver. Returns the
    /// sender session for `CnTransmitter::new` and the receiver's
    /// event stream so tests can assert on the wire bytes.
    async fn pair() -> (
        Arc<Mutex<RtpSession>>,
        tokio::sync::broadcast::Receiver<RtpEvent>,
    ) {
        let receiver_cfg = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: false,
            rtcp_mux: false,
            session_id: None,
            use_port_allocator: false,
        };
        let receiver_transport = UdpRtpTransport::new(receiver_cfg).await.unwrap();
        let receiver_addr = receiver_transport.local_rtp_addr().unwrap();
        let receiver_events = receiver_transport.subscribe();

        let sender_cfg = RtpSessionConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            remote_addr: Some(receiver_addr),
            ssrc: Some(0xCAFEBABE),
            payload_type: 0, // PCMU; CnTransmitter overrides to PT 13.
            clock_rate: 8000,
            jitter_buffer_size: None,
            max_packet_age_ms: None,
            enable_jitter_buffer: false,
        };
        let sender = RtpSession::new(sender_cfg).await.unwrap();
        (Arc::new(Mutex::new(sender)), receiver_events)
    }

    #[tokio::test]
    async fn send_emits_pt13_with_level_byte() {
        let (sender, mut events) = pair().await;
        let cn = CnTransmitter::new(sender);
        cn.send(40).await.expect("CN send");

        // Drain events until we see the CN packet.
        let timeout = tokio::time::sleep(Duration::from_millis(500));
        tokio::pin!(timeout);
        loop {
            tokio::select! {
                _ = &mut timeout => panic!("no CN packet received within 500 ms"),
                evt = events.recv() => {
                    match evt {
                        Ok(RtpEvent::MediaReceived { payload_type, payload, .. })
                            if payload_type == COMFORT_NOISE =>
                        {
                            assert_eq!(payload.len(), 1, "RFC 3389 CN payload is 1 byte (level only)");
                            assert_eq!(payload[0], 40, "level byte must round-trip on the wire");
                            return;
                        }
                        Ok(other) => {
                            // Skip non-CN events.
                            tracing::debug!("ignoring event: {:?}", other);
                        }
                        Err(_) => continue,
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn send_clamps_high_bit_per_rfc_3389_3_1() {
        let (sender, mut events) = pair().await;
        let cn = CnTransmitter::new(sender);
        // 0xFF — high bit set; RFC 3389 §3.1 reserves it.
        cn.send(0xFF).await.expect("CN send");

        let timeout = tokio::time::sleep(Duration::from_millis(500));
        tokio::pin!(timeout);
        loop {
            tokio::select! {
                _ = &mut timeout => panic!("no CN packet received within 500 ms"),
                evt = events.recv() => {
                    if let Ok(RtpEvent::MediaReceived { payload_type, payload, .. }) = evt {
                        if payload_type == COMFORT_NOISE {
                            assert_eq!(payload[0], 0x7f, "high bit must be cleared (clamp to 0..=127)");
                            return;
                        }
                    }
                }
            }
        }
    }
}
