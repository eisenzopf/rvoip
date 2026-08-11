//! Transparent RTP bridge between two media sessions.
//!
//! Two bridged sessions exchange RTP packet payloads directly without
//! traversing the AudioFrame decode path. Used by b2bua-style consumers that
//! need to forward media between two SIP legs without transcoding.
//!
//! Requirements enforced at [`MediaSessionController::bridge_sessions`]:
//!
//! - Both sessions must already have a remote RTP address (media flow ready).
//! - Both sessions must have negotiated the same RTP payload type. Mismatches
//!   return [`BridgeError::CodecMismatch`] — no transcoding is performed.
//! - Both sessions must agree on any format parameters that change the wire
//!   bytes. A matching payload type is **not** sufficient for every codec:
//!   two AMR legs can share a payload type while one uses RFC 4867
//!   bandwidth-efficient framing and the other octet-aligned, which are
//!   different bit layouts. Forwarding between them delivers payloads the far
//!   end cannot parse — silent audio failure rather than a clean error — so it
//!   returns [`BridgeError::FormatMismatch`].
//! - Neither session may already be bridged to another session.
//!
//! DTMF (RFC 2833) packets ride the same stream and are forwarded
//! transparently. RTCP is not bridged — each leg keeps generating its own
//! reports (RFC 3550 §7.2 compliance).
//!
//! The returned [`BridgeHandle`] tears the bridge down on drop: the cancel
//! gate flips synchronously and partner entries are removed, with forwarder
//! tasks aborted asynchronously.
//!
//! See `crates/session-core/docs/PRE_B2BUA_ROADMAP.md` Item 2 for the
//! b2bua use case driving this primitive.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use thiserror::Error;
use tokio::sync::{broadcast, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::error::Error;
use crate::types::DialogId;
use rvoip_rtp_core::session::RtpSessionEvent;
use rvoip_rtp_core::RtpSession;

use super::types::NEGOTIATED_FMTP_PARAMETER;
use super::MediaSessionController;

/// Errors specific to bridge creation and teardown.
#[derive(Debug, Error)]
pub enum BridgeError {
    #[error("media session not found: {0}")]
    SessionNotFound(String),

    /// The session exists but has no remote RTP address yet. Callers should
    /// bridge only after both legs reach the `Active` state.
    #[error("session {0} has no remote RTP address — not ready to bridge")]
    SessionNotActive(String),

    /// Format parameters differ in a way that changes the wire bytes, so the
    /// payloads are not interchangeable even though the payload type matches.
    #[error(
        "codec format mismatch on PT={payload_type}: session {a} negotiated {a_fmtp:?}, \
         session {b} negotiated {b_fmtp:?}; relaying between them would deliver \
         unparseable payloads"
    )]
    FormatMismatch {
        /// First session.
        a: String,
        /// Second session.
        b: String,
        /// The payload type both sessions share.
        payload_type: u8,
        /// First session's negotiated fmtp parameters.
        a_fmtp: String,
        /// Second session's negotiated fmtp parameters.
        b_fmtp: String,
    },

    /// Negotiated payload types differ. Transparent relay can't re-encode.
    #[error("codec payload-type mismatch: session {a} uses PT={a_pt}, session {b} uses PT={b_pt}")]
    CodecMismatch {
        a: String,
        b: String,
        a_pt: u8,
        b_pt: u8,
    },

    #[error("session {0} is already bridged to another session")]
    AlreadyBridged(String),

    #[error("cannot bridge a session to itself: {0}")]
    SameSession(String),
}

impl From<BridgeError> for Error {
    fn from(e: BridgeError) -> Self {
        Error::Config(e.to_string())
    }
}

/// Handle representing an active bridge between two media sessions.
///
/// Dropping this handle tears the bridge down: the cancel gate flips
/// synchronously, partner map entries are removed immediately, and the
/// background forwarder tasks are aborted asynchronously.
pub struct BridgeHandle {
    session_a: DialogId,
    session_b: DialogId,
    partner_map: Arc<DashMap<DialogId, DialogId>>,
    cancel: Arc<AtomicBool>,
    tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl BridgeHandle {
    /// Return the two session IDs involved in this bridge.
    pub fn sessions(&self) -> (&DialogId, &DialogId) {
        (&self.session_a, &self.session_b)
    }
}

impl Drop for BridgeHandle {
    fn drop(&mut self) {
        // Synchronously stop accepting new forwarded packets. Forwarder tasks
        // observe this gate on their next loop iteration or via task abort.
        self.cancel.store(true, Ordering::SeqCst);
        self.partner_map.remove(&self.session_a);
        self.partner_map.remove(&self.session_b);

        let tasks = self.tasks.clone();
        let a = self.session_a.clone();
        let b = self.session_b.clone();
        tokio::spawn(async move {
            let mut guard = tasks.lock().await;
            for task in guard.drain(..) {
                task.abort();
            }
            debug!("🔗 bridge {} <-> {} forwarder tasks aborted", a, b);
        });
    }
}

impl MediaSessionController {
    /// Bridge two existing media sessions at the RTP packet level.
    ///
    /// Both sessions must be ready (have a remote address) and must have
    /// negotiated the same payload type. While bridged, inbound RTP from
    /// session A is forwarded as outbound RTP on session B (and vice versa)
    /// without decoding.
    ///
    /// The returned [`BridgeHandle`] owns the bridge lifetime — dropping it
    /// restores normal per-session behavior.
    pub async fn bridge_sessions(
        &self,
        a: DialogId,
        b: DialogId,
    ) -> std::result::Result<BridgeHandle, BridgeError> {
        if a == b {
            return Err(BridgeError::SameSession(a.to_string()));
        }

        // Preflight: both sessions exist, have remote addresses, and use
        // matching payload types.
        let (a_session_arc, a_pt, a_fmtp) = self.read_bridge_preconditions(&a).await?;
        let (b_session_arc, b_pt, b_fmtp) = self.read_bridge_preconditions(&b).await?;

        if a_pt != b_pt {
            return Err(BridgeError::CodecMismatch {
                a: a.to_string(),
                b: b.to_string(),
                a_pt,
                b_pt,
            });
        }

        // A matching payload type does not imply interchangeable payloads.
        // AMR is the case that forces this: the same payload type carries
        // either bandwidth-efficient or octet-aligned framing depending on
        // `octet-align`, and those are different bit layouts. Relaying across
        // the boundary would hand the far end bytes it cannot parse.
        if !wire_formats_are_interchangeable(&a_fmtp, &b_fmtp) {
            return Err(BridgeError::FormatMismatch {
                a: a.to_string(),
                b: b.to_string(),
                payload_type: a_pt,
                a_fmtp,
                b_fmtp,
            });
        }

        // Register partnership (atomic via DashMap). Error out on double-
        // bridge before subscribing to events to avoid resource leaks.
        if self.bridge_partners.contains_key(&a) {
            return Err(BridgeError::AlreadyBridged(a.to_string()));
        }
        if self.bridge_partners.contains_key(&b) {
            return Err(BridgeError::AlreadyBridged(b.to_string()));
        }
        self.bridge_partners.insert(a.clone(), b.clone());
        self.bridge_partners.insert(b.clone(), a.clone());

        // Subscribe to each session's RTP event broadcast. Subscribing
        // early (before spawning) ensures no packets are lost between
        // handshake and the forwarder task starting to poll.
        let a_subscriber = {
            let guard = a_session_arc.lock().await;
            guard.subscribe()
        };
        let b_subscriber = {
            let guard = b_session_arc.lock().await;
            guard.subscribe()
        };

        let cancel = Arc::new(AtomicBool::new(false));

        // Pre-snapshot the lock-free send handles for both directions
        // so the forwarder tasks never need to lock the destination
        // session per packet — see Phase C16 + `RtpSession::send_handle`.
        let a_send_handle = {
            let guard = a_session_arc.lock().await;
            guard.send_handle()
        };
        let b_send_handle = {
            let guard = b_session_arc.lock().await;
            guard.send_handle()
        };

        let task_ab = tokio::spawn(forward_rtp(
            a.clone(),
            b.clone(),
            a_subscriber,
            b_session_arc.clone(),
            b_send_handle,
            cancel.clone(),
        ));
        let task_ba = tokio::spawn(forward_rtp(
            b.clone(),
            a.clone(),
            b_subscriber,
            a_session_arc.clone(),
            a_send_handle,
            cancel.clone(),
        ));

        info!("🔗 Bridged RTP sessions: {} <-> {} (PT={})", a, b, a_pt);

        Ok(BridgeHandle {
            session_a: a,
            session_b: b,
            partner_map: self.bridge_partners.clone(),
            cancel,
            tasks: Arc::new(Mutex::new(vec![task_ab, task_ba])),
        })
    }

    /// Return true if the given dialog is currently bridged.
    pub fn is_bridged(&self, dialog: &DialogId) -> bool {
        self.bridge_partners.contains_key(dialog)
    }

    /// Return the partner dialog for a bridged session, if any.
    pub fn bridge_partner(&self, dialog: &DialogId) -> Option<DialogId> {
        self.bridge_partners.get(dialog).map(|e| e.value().clone())
    }

    /// Internal cleanup invoked when a session is stopped while bridged.
    /// Removes partner-map entries so a stale partner can't be forwarded to.
    pub(super) fn clear_bridge_partner(&self, dialog: &DialogId) {
        if let Some((_, partner)) = self.bridge_partners.remove(dialog) {
            self.bridge_partners.remove(&partner);
            debug!(
                "🔗 Cleared bridge partnership for stopped session: {}",
                dialog
            );
        }
    }

    /// Read the RTP session Arc, negotiated payload type, and negotiated
    /// format parameters for `id`.
    ///
    /// Returns [`BridgeError::SessionNotFound`] if the session is missing
    /// and [`BridgeError::SessionNotActive`] if it has no remote address.
    async fn read_bridge_preconditions(
        &self,
        id: &DialogId,
    ) -> std::result::Result<(Arc<Mutex<RtpSession>>, u8, String), BridgeError> {
        // Snapshot the session Arc + remote-addr check from the
        // DashMap shard. The shard guard drops at the end of the
        // closure — no await held while a shard is locked.
        let session_arc = self
            .rtp_sessions
            .get(id)
            .map(|r| {
                let w = r.value();
                if w.remote_addr.is_none() {
                    Err(BridgeError::SessionNotActive(id.to_string()))
                } else {
                    Ok(w.session.clone())
                }
            })
            .ok_or_else(|| BridgeError::SessionNotFound(id.to_string()))??;

        let (pt, fmtp) = self
            .sessions
            .get(id)
            .map(|r| {
                let config = &r.value().config;
                let pt = config
                    .preferred_codec
                    .as_ref()
                    .and_then(|codec| self.codec_mapper.codec_to_payload(codec))
                    .unwrap_or(0);
                let fmtp = config
                    .parameters
                    .get(NEGOTIATED_FMTP_PARAMETER)
                    .cloned()
                    .unwrap_or_default();
                (pt, fmtp)
            })
            .ok_or_else(|| BridgeError::SessionNotFound(id.to_string()))?;

        Ok((session_arc, pt, fmtp))
    }
}

/// Whether two negotiated `a=fmtp` strings describe the same wire format.
///
/// Only parameters that change the bytes on the wire matter here. For AMR,
/// RFC 4867 §8.3.1 names exactly those: "Each combination of the RTP payload
/// transport format configuration parameters (octet-align, crc,
/// robust-sorting, interleaving, and channels) is unique in its bit-pattern
/// and not compatible with any other combination."
///
/// Everything else — `mode-set`, `mode-change-period`, `max-red` — constrains
/// which modes may be used or when they may change, not how a frame is laid
/// out, so it does not prevent relaying. A mode-set difference is a real
/// concern for a transcoding B2BUA but not for a transparent one: the frames
/// still parse, and each leg's own negotiation already bound its peer.
fn wire_formats_are_interchangeable(a: &str, b: &str) -> bool {
    /// The transport-format parameters, defaulting to RFC 4867's defaults.
    fn transport_shape(fmtp: &str) -> (bool, bool, bool, bool, u8) {
        let mut octet_align = false;
        let mut crc = false;
        let mut robust_sorting = false;
        let mut interleaving = false;
        let mut channels = 1u8;
        for part in fmtp.split(';') {
            let Some((name, value)) = part.trim().split_once('=') else {
                continue;
            };
            let value = value.trim().trim_matches('"');
            match name.trim().to_ascii_lowercase().as_str() {
                "octet-align" => octet_align = value == "1",
                "crc" => crc = value == "1",
                "robust-sorting" => robust_sorting = value == "1",
                "interleaving" => interleaving = true,
                "channels" => channels = value.parse().unwrap_or(1),
                _ => {}
            }
        }
        // crc, robust-sorting and interleaving each imply octet alignment, so
        // normalise before comparing — `crc=1` alone and `octet-align=1; crc=1`
        // describe the same wire format.
        (
            octet_align || crc || robust_sorting || interleaving,
            crc,
            robust_sorting,
            interleaving,
            channels,
        )
    }

    transport_shape(a) == transport_shape(b)
}

/// Forwarder task: subscribe to `src`'s RTP events and replay each inbound
/// packet's payload+timestamp+marker as an outbound packet on `dst`.
///
/// The destination RTP session assigns its own sequence number and SSRC —
/// we only carry the timestamp, payload bytes, and marker bit.
///
/// If a lock-free `RtpSendHandle` was successfully snapshot at bridge
/// setup, the per-packet send goes through it without locking
/// `dst_session`. Otherwise we fall back to per-packet
/// `dst_session.lock()`.
async fn forward_rtp(
    src: DialogId,
    dst: DialogId,
    mut events: broadcast::Receiver<RtpSessionEvent>,
    dst_session: Arc<Mutex<RtpSession>>,
    dst_send_handle: Option<rvoip_rtp_core::RtpSendHandle>,
    cancel: Arc<AtomicBool>,
) {
    debug!("🔗 bridge forwarder started: {} -> {}", src, dst);
    loop {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        match events.recv().await {
            Ok(RtpSessionEvent::PacketReceived(packet)) => {
                if cancel.load(Ordering::SeqCst) {
                    break;
                }
                let payload = packet.payload.clone();
                let ts = packet.header.timestamp;
                let marker = packet.header.marker;
                let send_result = if let Some(handle) = &dst_send_handle {
                    handle.send_packet(ts, payload, marker).await
                } else {
                    let dst_guard = dst_session.lock().await;
                    dst_guard.send_packet(ts, payload, marker).await
                };
                if let Err(e) = send_result {
                    warn!("bridge forward {}->{} send_packet failed: {}", src, dst, e);
                }
            }
            Ok(_) => {
                // Non-data events (BYE, NewStreamDetected, RTCP SR/RR, Error)
                // are ignored — each leg manages its own control plane.
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                warn!("bridge forwarder {}->{} lagged {} events", src, dst, n);
            }
            Err(broadcast::error::RecvError::Closed) => {
                debug!("🔗 bridge forwarder source closed: {}", src);
                break;
            }
        }
    }
    debug!("🔗 bridge forwarder exited: {} -> {}", src, dst);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::controller::{MediaConfig, MediaSessionController};
    use std::collections::HashMap;
    use std::net::SocketAddr;

    fn test_config(codec: &str) -> MediaConfig {
        MediaConfig {
            local_addr: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
            remote_addr: Some("127.0.0.1:40000".parse::<SocketAddr>().unwrap()),
            preferred_codec: Some(codec.to_string()),
            parameters: HashMap::new(),
        }
    }

    fn expect_err<T>(r: std::result::Result<T, BridgeError>) -> BridgeError {
        match r {
            Ok(_) => panic!("expected BridgeError, got Ok"),
            Err(e) => e,
        }
    }

    fn expect_ok<T>(r: std::result::Result<T, BridgeError>) -> T {
        match r {
            Ok(v) => v,
            Err(e) => panic!("expected Ok, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn bridge_same_session_errors() {
        let controller = MediaSessionController::new();
        let id = DialogId::new("same");
        controller
            .start_media(id.clone(), test_config("PCMU"))
            .await
            .unwrap();

        let err = expect_err(controller.bridge_sessions(id.clone(), id).await);
        assert!(matches!(err, BridgeError::SameSession(_)));
    }

    #[tokio::test]
    async fn bridge_missing_session_errors() {
        let controller = MediaSessionController::new();
        let a = DialogId::new("a");
        let b = DialogId::new("b");
        controller
            .start_media(a.clone(), test_config("PCMU"))
            .await
            .unwrap();

        let err = expect_err(controller.bridge_sessions(a, b).await);
        assert!(matches!(err, BridgeError::SessionNotFound(_)));
    }

    #[tokio::test]
    async fn bridge_codec_mismatch_errors() {
        let controller = MediaSessionController::new();
        let a = DialogId::new("a");
        let b = DialogId::new("b");
        controller
            .start_media(a.clone(), test_config("PCMU"))
            .await
            .unwrap();
        controller
            .start_media(b.clone(), test_config("PCMA"))
            .await
            .unwrap();

        let err = expect_err(controller.bridge_sessions(a, b).await);
        match err {
            BridgeError::CodecMismatch { a_pt, b_pt, .. } => {
                assert_eq!(a_pt, 0);
                assert_eq!(b_pt, 8);
            }
            other => panic!("expected CodecMismatch, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn bridge_success_and_drop_cleans_partnership() {
        let controller = MediaSessionController::new();
        let a = DialogId::new("a");
        let b = DialogId::new("b");
        controller
            .start_media(a.clone(), test_config("PCMU"))
            .await
            .unwrap();
        controller
            .start_media(b.clone(), test_config("PCMU"))
            .await
            .unwrap();

        let handle = expect_ok(controller.bridge_sessions(a.clone(), b.clone()).await);
        assert!(controller.is_bridged(&a));
        assert!(controller.is_bridged(&b));
        assert_eq!(controller.bridge_partner(&a).as_ref(), Some(&b));
        assert_eq!(controller.bridge_partner(&b).as_ref(), Some(&a));

        drop(handle);
        // Drop flips the gate synchronously; partner map is cleared
        // immediately, not on task completion.
        assert!(!controller.is_bridged(&a));
        assert!(!controller.is_bridged(&b));
    }

    #[test]
    fn identical_and_default_fmtp_are_interchangeable() {
        assert!(wire_formats_are_interchangeable("", ""));
        assert!(wire_formats_are_interchangeable(
            "octet-align=1",
            "octet-align=1"
        ));
        // Absent and explicitly-default mean the same wire format.
        assert!(wire_formats_are_interchangeable("", "octet-align=0"));
        assert!(wire_formats_are_interchangeable("mode-set=0,1", ""));
    }

    #[test]
    fn amr_framing_difference_is_not_interchangeable() {
        // The case this guard exists for. Both legs would carry the same
        // payload type, but bandwidth-efficient and octet-aligned are
        // different bit layouts, so relaying between them delivers payloads
        // the far end cannot parse.
        assert!(!wire_formats_are_interchangeable("", "octet-align=1"));
        assert!(!wire_formats_are_interchangeable(
            "octet-align=0; mode-set=0,1",
            "octet-align=1; mode-set=0,1"
        ));
    }

    #[test]
    fn transport_options_that_change_the_bytes_are_compared() {
        for (a, b) in [
            ("octet-align=1", "octet-align=1; crc=1"),
            ("octet-align=1", "octet-align=1; robust-sorting=1"),
            ("octet-align=1", "octet-align=1; interleaving=2"),
            ("octet-align=1", "octet-align=1; channels=2"),
        ] {
            assert!(
                !wire_formats_are_interchangeable(a, b),
                "{a:?} vs {b:?} must not be interchangeable"
            );
        }
    }

    #[test]
    fn options_implying_octet_align_normalise_before_comparison() {
        // RFC 4867: crc, robust-sorting and interleaving each imply
        // octet-aligned operation, so stating it explicitly changes nothing.
        assert!(wire_formats_are_interchangeable("crc=1", "octet-align=1; crc=1"));
        assert!(wire_formats_are_interchangeable(
            "robust-sorting=1",
            "octet-align=1; robust-sorting=1"
        ));
    }

    #[test]
    fn parameters_that_do_not_change_the_layout_still_bridge() {
        // mode-set and friends restrict which modes may be used, not how a
        // frame is laid out. The frames still parse, and each leg's own
        // negotiation already bound its peer, so a transparent relay can
        // forward across the difference.
        assert!(wire_formats_are_interchangeable(
            "octet-align=1; mode-set=0,1,2",
            "octet-align=1; mode-set=3,4"
        ));
        assert!(wire_formats_are_interchangeable(
            "octet-align=1; mode-change-period=2",
            "octet-align=1; max-red=220"
        ));
    }

    #[test]
    fn fmtp_comparison_tolerates_real_world_spacing_and_case() {
        assert!(wire_formats_are_interchangeable(
            "OCTET-ALIGN=1;Mode-Set=0,1",
            "  octet-align = 1 ; mode-set=0,1  "
        ));
        // Quoted values appear in the field too.
        assert!(wire_formats_are_interchangeable(
            "octet-align=\"1\"",
            "octet-align=1"
        ));
    }

    /// A config carrying an explicit negotiated fmtp string.
    ///
    /// Through the same builder the SIP layer uses, not a raw map insert: a
    /// helper that writes the key itself would keep passing if the builder
    /// stopped writing it, which is most of how this parameter came to be
    /// unwired in the first place.
    fn test_config_with_fmtp(codec: &str, fmtp: &str) -> MediaConfig {
        test_config(codec).with_negotiated_fmtp(Some(fmtp))
    }

    #[test]
    fn clearing_the_fmtp_removes_the_key_rather_than_emptying_it() {
        // Absent and empty are different states of the map, and the guard
        // reads the map. A re-negotiation that carries no fmtp arrives here.
        let carried = test_config_with_fmtp("PCMU", "octet-align=1");
        assert_eq!(
            carried.parameters.get(NEGOTIATED_FMTP_PARAMETER).cloned(),
            Some("octet-align=1".to_string())
        );
        let cleared = carried.with_negotiated_fmtp(None);
        assert!(!cleared.parameters.contains_key(NEGOTIATED_FMTP_PARAMETER));
    }

    #[tokio::test]
    async fn bridge_rejects_mismatched_amr_framing() {
        // End to end through bridge_sessions: same codec, same payload type,
        // different framing. Without the format check this bridges
        // "successfully" and then delivers unparseable audio — a silent
        // failure rather than an error.
        let controller = MediaSessionController::new();
        let a = DialogId::new("amr-a");
        let b = DialogId::new("amr-b");

        controller
            .start_media(a.clone(), test_config_with_fmtp("PCMU", ""))
            .await
            .unwrap();
        controller
            .start_media(b.clone(), test_config_with_fmtp("PCMU", "octet-align=1"))
            .await
            .unwrap();

        let err = expect_err(controller.bridge_sessions(a, b).await);
        assert!(
            matches!(err, BridgeError::FormatMismatch { .. }),
            "expected FormatMismatch, got {err:?}"
        );
    }

    #[tokio::test]
    async fn bridge_accepts_matching_amr_framing() {
        // The guard must not reject legs that genuinely can relay: same
        // framing, differing only in parameters that do not change the layout.
        let controller = MediaSessionController::new();
        let a = DialogId::new("amr-ok-a");
        let b = DialogId::new("amr-ok-b");

        controller
            .start_media(
                a.clone(),
                test_config_with_fmtp("PCMU", "octet-align=1; mode-set=0,1,2"),
            )
            .await
            .unwrap();
        controller
            .start_media(
                b.clone(),
                test_config_with_fmtp("PCMU", "octet-align=1; mode-set=3,4"),
            )
            .await
            .unwrap();

        let handle = expect_ok(controller.bridge_sessions(a.clone(), b.clone()).await);
        assert!(controller.is_bridged(&a));
        drop(handle);
    }

    #[tokio::test]
    async fn bridge_double_bridge_errors() {
        let controller = MediaSessionController::new();
        let a = DialogId::new("a");
        let b = DialogId::new("b");
        let c = DialogId::new("c");
        controller
            .start_media(a.clone(), test_config("PCMU"))
            .await
            .unwrap();
        controller
            .start_media(b.clone(), test_config("PCMU"))
            .await
            .unwrap();
        controller
            .start_media(c.clone(), test_config("PCMU"))
            .await
            .unwrap();

        let _first = expect_ok(controller.bridge_sessions(a.clone(), b.clone()).await);
        let err = expect_err(controller.bridge_sessions(a, c).await);
        assert!(matches!(err, BridgeError::AlreadyBridged(_)));
    }

    #[tokio::test]
    async fn stop_media_clears_bridge_partnership() {
        let controller = MediaSessionController::new();
        let a = DialogId::new("a");
        let b = DialogId::new("b");
        controller
            .start_media(a.clone(), test_config("PCMU"))
            .await
            .unwrap();
        controller
            .start_media(b.clone(), test_config("PCMU"))
            .await
            .unwrap();

        let _handle = expect_ok(controller.bridge_sessions(a.clone(), b.clone()).await);
        assert!(controller.is_bridged(&a));

        controller.stop_media(&a).await.unwrap();

        // stop_media clears both ends of the partnership so b isn't left
        // pointing at a dead session.
        assert!(!controller.is_bridged(&a));
        assert!(!controller.is_bridged(&b));
    }
}
