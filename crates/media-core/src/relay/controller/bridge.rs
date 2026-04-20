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

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use dashmap::DashMap;
use thiserror::Error;
use tokio::sync::{Mutex, broadcast};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::error::Error;
use crate::types::DialogId;
use rvoip_rtp_core::RtpSession;
use rvoip_rtp_core::session::RtpSessionEvent;

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

    /// Negotiated payload types differ. Transparent relay can't re-encode.
    #[error(
        "codec payload-type mismatch: session {a} uses PT={a_pt}, session {b} uses PT={b_pt}"
    )]
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
        let (a_session_arc, a_pt) = self.read_bridge_preconditions(&a).await?;
        let (b_session_arc, b_pt) = self.read_bridge_preconditions(&b).await?;

        if a_pt != b_pt {
            return Err(BridgeError::CodecMismatch {
                a: a.to_string(),
                b: b.to_string(),
                a_pt,
                b_pt,
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

        let task_ab = tokio::spawn(forward_rtp(
            a.clone(),
            b.clone(),
            a_subscriber,
            b_session_arc.clone(),
            cancel.clone(),
        ));
        let task_ba = tokio::spawn(forward_rtp(
            b.clone(),
            a.clone(),
            b_subscriber,
            a_session_arc.clone(),
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
            debug!("🔗 Cleared bridge partnership for stopped session: {}", dialog);
        }
    }

    /// Read the RTP session Arc and negotiated payload type for `id`.
    /// Returns [`BridgeError::SessionNotFound`] if the session is missing
    /// and [`BridgeError::SessionNotActive`] if it has no remote address.
    async fn read_bridge_preconditions(
        &self,
        id: &DialogId,
    ) -> std::result::Result<(Arc<Mutex<RtpSession>>, u8), BridgeError> {
        let rtp_sessions = self.rtp_sessions.read().await;
        let wrapper = rtp_sessions
            .get(id)
            .ok_or_else(|| BridgeError::SessionNotFound(id.to_string()))?;
        if wrapper.remote_addr.is_none() {
            return Err(BridgeError::SessionNotActive(id.to_string()));
        }
        let session_arc = wrapper.session.clone();
        drop(rtp_sessions);

        let sessions = self.sessions.read().await;
        let session_info = sessions
            .get(id)
            .ok_or_else(|| BridgeError::SessionNotFound(id.to_string()))?;
        let pt = session_info
            .config
            .preferred_codec
            .as_ref()
            .and_then(|codec| self.codec_mapper.codec_to_payload(codec))
            .unwrap_or(0);
        Ok((session_arc, pt))
    }
}

/// Forwarder task: subscribe to `src`'s RTP events and replay each inbound
/// packet's payload+timestamp+marker as an outbound packet on `dst`.
///
/// The destination RTP session assigns its own sequence number and SSRC —
/// we only carry the timestamp, payload bytes, and marker bit.
async fn forward_rtp(
    src: DialogId,
    dst: DialogId,
    mut events: broadcast::Receiver<RtpSessionEvent>,
    dst_session: Arc<Mutex<RtpSession>>,
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
                let mut dst_guard = dst_session.lock().await;
                if let Err(e) = dst_guard.send_packet(ts, payload, marker).await {
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
        controller.start_media(id.clone(), test_config("PCMU")).await.unwrap();

        let err = expect_err(controller.bridge_sessions(id.clone(), id).await);
        assert!(matches!(err, BridgeError::SameSession(_)));
    }

    #[tokio::test]
    async fn bridge_missing_session_errors() {
        let controller = MediaSessionController::new();
        let a = DialogId::new("a");
        let b = DialogId::new("b");
        controller.start_media(a.clone(), test_config("PCMU")).await.unwrap();

        let err = expect_err(controller.bridge_sessions(a, b).await);
        assert!(matches!(err, BridgeError::SessionNotFound(_)));
    }

    #[tokio::test]
    async fn bridge_codec_mismatch_errors() {
        let controller = MediaSessionController::new();
        let a = DialogId::new("a");
        let b = DialogId::new("b");
        controller.start_media(a.clone(), test_config("PCMU")).await.unwrap();
        controller.start_media(b.clone(), test_config("PCMA")).await.unwrap();

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
        controller.start_media(a.clone(), test_config("PCMU")).await.unwrap();
        controller.start_media(b.clone(), test_config("PCMU")).await.unwrap();

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

    #[tokio::test]
    async fn bridge_double_bridge_errors() {
        let controller = MediaSessionController::new();
        let a = DialogId::new("a");
        let b = DialogId::new("b");
        let c = DialogId::new("c");
        controller.start_media(a.clone(), test_config("PCMU")).await.unwrap();
        controller.start_media(b.clone(), test_config("PCMU")).await.unwrap();
        controller.start_media(c.clone(), test_config("PCMU")).await.unwrap();

        let _first = expect_ok(controller.bridge_sessions(a.clone(), b.clone()).await);
        let err = expect_err(controller.bridge_sessions(a, c).await);
        assert!(matches!(err, BridgeError::AlreadyBridged(_)));
    }

    #[tokio::test]
    async fn stop_media_clears_bridge_partnership() {
        let controller = MediaSessionController::new();
        let a = DialogId::new("a");
        let b = DialogId::new("b");
        controller.start_media(a.clone(), test_config("PCMU")).await.unwrap();
        controller.start_media(b.clone(), test_config("PCMU")).await.unwrap();

        let _handle = expect_ok(controller.bridge_sessions(a.clone(), b.clone()).await);
        assert!(controller.is_bridged(&a));

        controller.stop_media(&a).await.unwrap();

        // stop_media clears both ends of the partnership so b isn't left
        // pointing at a dead session.
        assert!(!controller.is_bridged(&a));
        assert!(!controller.is_bridged(&b));
    }
}
