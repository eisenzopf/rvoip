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

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use thiserror::Error;
use tokio::sync::{broadcast, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::error::Error;
use crate::processing::format::converter::{ConversionParams, FormatConverter};
use crate::types::{DialogId, SampleRate};
use rvoip_rtp_core::session::RtpSessionEvent;
use rvoip_rtp_core::RtpSendHandle;
use rvoip_rtp_core::RtpSession;

use super::codec_runtime::normalized_codec_name;
use super::types::{MediaConfig, TelephoneEventFormat, NEGOTIATED_FMTP_PARAMETER};
use super::MediaSessionController;

/// How a bridge moves payloads between its two legs.
///
/// [`BridgeMode::Relay`] forwards RTP payloads byte-for-byte (the historical
/// behaviour, and the only mode when both legs negotiated the same wire
/// format). [`BridgeMode::Transcode`] decodes each leg's payloads with that
/// leg's negotiated codec, converts the decoded PCM to the other leg's
/// negotiated rate/channel shape, and re-encodes with the other leg's codec —
/// so two legs that agreed different codecs with their peers can still talk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeMode {
    /// Forward RTP payloads unchanged (payload, timestamp and marker all
    /// preserved). Requires both legs to share a payload type and an
    /// interchangeable wire format.
    Relay,
    /// Decode → convert → re-encode per direction. Requires both codecs to be
    /// encodable/decodable by this build (feature-gated codecs like Opus need
    /// their feature enabled).
    Transcode,
}

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
    mode_map: Option<Arc<DashMap<DialogId, BridgeMode>>>,
    transcode_counters: Option<Arc<TranscodeCounters>>,
    cancel: Arc<AtomicBool>,
    tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl BridgeHandle {
    /// Return the two session IDs involved in this bridge.
    pub fn sessions(&self) -> (&DialogId, &DialogId) {
        (&self.session_a, &self.session_b)
    }

    /// The mode this bridge was created with.
    pub fn mode(&self) -> BridgeMode {
        self.mode_map
            .as_ref()
            .and_then(|modes| modes.get(&self.session_a).map(|m| *m.value()))
            .unwrap_or(BridgeMode::Relay)
    }

    /// What a [`BridgeMode::Transcode`] bridge did with non-audio packets so
    /// far (relayed or dropped telephone-event, dropped comfort noise and
    /// unknown payload types). `None` for a relay bridge, which forwards
    /// every packet unchanged.
    pub fn transcode_stats(&self) -> Option<TranscodeBridgeStats> {
        self.transcode_counters
            .as_ref()
            .map(|counters| counters.snapshot())
    }
}

impl Drop for BridgeHandle {
    fn drop(&mut self) {
        // Synchronously stop accepting new forwarded packets. Forwarder tasks
        // observe this gate on their next loop iteration or via task abort.
        self.cancel.store(true, Ordering::SeqCst);
        self.partner_map.remove(&self.session_a);
        self.partner_map.remove(&self.session_b);
        if let Some(modes) = &self.mode_map {
            modes.remove(&self.session_a);
            modes.remove(&self.session_b);
        }

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
        self.bridge_sessions_with_mode(a, b, BridgeMode::Relay)
            .await
    }

    /// Bridge two media sessions, choosing how payloads move between them.
    ///
    /// [`BridgeMode::Relay`] is the historical [`Self::bridge_sessions`]
    /// behaviour: payloads are forwarded byte-for-byte and both legs must have
    /// negotiated the same, interchangeable wire format.
    ///
    /// [`BridgeMode::Transcode`] relaxes the shared-codec requirement: each
    /// direction decodes with the *source* leg's negotiated codec, converts the
    /// decoded PCM to the *destination* leg's negotiated rate/channel shape,
    /// and re-encodes with the destination codec. Both codecs must be
    /// encodable/decodable in this build. RFC 4733 telephone-event is relayed
    /// to a leg that negotiated it ([`MediaConfig::with_telephone_event`])
    /// under that leg's payload type and clock, and dropped otherwise;
    /// comfort noise and other unnegotiated payload types are dropped. See
    /// [`BridgeHandle::transcode_stats`] for the counts.
    pub async fn bridge_sessions_with_mode(
        &self,
        a: DialogId,
        b: DialogId,
        mode: BridgeMode,
    ) -> std::result::Result<BridgeHandle, BridgeError> {
        if a == b {
            return Err(BridgeError::SameSession(a.to_string()));
        }

        // Preflight: both sessions exist and have remote addresses.
        let (a_session_arc, a_pt, a_fmtp, a_name) = self.read_bridge_preconditions(&a).await?;
        let (b_session_arc, b_pt, b_fmtp, b_name) = self.read_bridge_preconditions(&b).await?;

        if mode == BridgeMode::Relay {
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
            if !wire_formats_are_interchangeable(&a_name, &a_fmtp, &b_name, &b_fmtp) {
                return Err(BridgeError::FormatMismatch {
                    a: a.to_string(),
                    b: b.to_string(),
                    payload_type: a_pt,
                    a_fmtp,
                    b_fmtp,
                });
            }
        } else {
            // Transcode: both legs must have an encodable/decodable codec
            // runtime. Resolve both up front so an unsupported combination
            // fails before any partnership is registered (leaving the
            // previous state intact).
            let a_runtime = self
                .codec_runtimes
                .get(&a)
                .map(|entry| entry.value().clone())
                .ok_or_else(|| BridgeError::SessionNotFound(a.to_string()))?;
            let b_runtime = self
                .codec_runtimes
                .get(&b)
                .map(|entry| entry.value().clone())
                .ok_or_else(|| BridgeError::SessionNotFound(b.to_string()))?;
            // Both directions must be convertible: the destination sample
            // rate has to be inside the converter's supported set.
            for (from, to) in [
                (a_runtime.format.clock_rate, b_runtime.format.clock_rate),
                (b_runtime.format.clock_rate, a_runtime.format.clock_rate),
            ] {
                if SampleRate::from_hz(to).is_none() {
                    return Err(BridgeError::FormatMismatch {
                        a: a.to_string(),
                        b: b.to_string(),
                        payload_type: a_pt,
                        a_fmtp: format!("clock={from}"),
                        b_fmtp: format!("clock={to}"),
                    });
                }
            }
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
        if mode == BridgeMode::Transcode {
            self.bridge_modes.insert(a.clone(), mode);
            self.bridge_modes.insert(b.clone(), mode);
        }

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
        // Media executor v2: the forwarders consume `PacketReceived`, so both
        // legs leave the inline receive path before their next packet.
        self.request_inline_demotion(&a);
        self.request_inline_demotion(&b);

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

        let transcode_counters =
            (mode == BridgeMode::Transcode).then(|| Arc::new(TranscodeCounters::default()));
        let tasks = match mode {
            BridgeMode::Relay => {
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
                vec![task_ab, task_ba]
            }
            BridgeMode::Transcode => {
                let counters = transcode_counters
                    .clone()
                    .unwrap_or_else(|| Arc::new(TranscodeCounters::default()));
                let task_ab = tokio::spawn(forward_rtp_transcode(
                    a.clone(),
                    b.clone(),
                    a_subscriber,
                    b_send_handle,
                    b_session_arc.clone(),
                    Arc::clone(&self.codec_runtimes),
                    Arc::clone(&self.sessions),
                    Arc::clone(&counters),
                    cancel.clone(),
                ));
                let task_ba = tokio::spawn(forward_rtp_transcode(
                    b.clone(),
                    a.clone(),
                    b_subscriber,
                    a_send_handle,
                    a_session_arc.clone(),
                    Arc::clone(&self.codec_runtimes),
                    Arc::clone(&self.sessions),
                    counters,
                    cancel.clone(),
                ));
                vec![task_ab, task_ba]
            }
        };

        match mode {
            BridgeMode::Relay => {
                info!("🔗 Bridged RTP sessions: {} <-> {} (PT={})", a, b, a_pt);
            }
            BridgeMode::Transcode => {
                info!(
                    "🔗 Bridged RTP sessions with transcoding: {} ({}, PT={}) <-> {} ({}, PT={})",
                    a, a_name, a_pt, b, b_name, b_pt
                );
            }
        }

        Ok(BridgeHandle {
            session_a: a,
            session_b: b,
            partner_map: self.bridge_partners.clone(),
            mode_map: (mode == BridgeMode::Transcode).then(|| Arc::clone(&self.bridge_modes)),
            transcode_counters,
            cancel,
            tasks: Arc::new(Mutex::new(tasks)),
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
    ) -> std::result::Result<(Arc<Mutex<RtpSession>>, u8, String, String), BridgeError> {
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

        let (pt, fmtp, name) = self
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
                let name = config.preferred_codec.clone().unwrap_or_default();
                (pt, fmtp, name)
            })
            .ok_or_else(|| BridgeError::SessionNotFound(id.to_string()))?;

        Ok((session_arc, pt, fmtp, name))
    }
}

impl MediaSessionController {
    /// Re-check a bridged pair's wire formats against a *proposed* config.
    ///
    /// The bridge's framing guard runs once, when the bridge is created. A
    /// re-INVITE that flips `octet-align` on one leg of a live bridge would
    /// otherwise be accepted, and the bridge would go on relaying frames the
    /// far end cannot parse — the same silent-wrong-audio failure the guard
    /// exists to prevent, arriving through the one door it did not watch.
    ///
    /// Returns `Ok(())` when this dialog is not bridged, which is the common
    /// case and costs one `DashMap` lookup.
    ///
    /// # Errors
    ///
    /// When the proposed config's payload type or transport-format parameters
    /// would no longer match the partner's. The caller must reject the update
    /// rather than apply it: a bridge relaying the wrong framing is worse than
    /// a re-INVITE that fails, because it is not diagnosable from either end.
    pub(super) fn revalidate_bridge_format(
        &self,
        dialog_id: &DialogId,
        proposed: &MediaConfig,
    ) -> std::result::Result<(), BridgeError> {
        let Some(partner) = self
            .bridge_partners
            .get(dialog_id)
            .map(|e| e.value().clone())
        else {
            return Ok(());
        };

        // A transcode bridge re-encodes whatever each leg negotiates, so a
        // codec change on one leg is absorbed by the codec-runtime rebuild in
        // `update_media` instead of being refused. Only relay bridges need the
        // wire-format guards below.
        if self
            .bridge_modes
            .get(dialog_id)
            .is_some_and(|mode| *mode.value() == BridgeMode::Transcode)
        {
            return Ok(());
        }

        let shape = |config: &MediaConfig| {
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
            (pt, fmtp, config.preferred_codec.clone().unwrap_or_default())
        };

        let (proposed_pt, proposed_fmtp, proposed_name) = shape(proposed);
        let Some((partner_pt, partner_fmtp, partner_name)) = self
            .sessions
            .get(&partner)
            .map(|entry| shape(&entry.value().config))
        else {
            // The partner vanished between the lookup and here. Nothing to
            // relay to, so nothing to protect.
            return Ok(());
        };

        if proposed_pt != partner_pt {
            return Err(BridgeError::CodecMismatch {
                a: dialog_id.to_string(),
                b: partner.to_string(),
                a_pt: proposed_pt,
                b_pt: partner_pt,
            });
        }
        if !wire_formats_are_interchangeable(
            &proposed_name,
            &proposed_fmtp,
            &partner_name,
            &partner_fmtp,
        ) {
            return Err(BridgeError::FormatMismatch {
                a: dialog_id.to_string(),
                b: partner.to_string(),
                payload_type: proposed_pt,
                a_fmtp: proposed_fmtp,
                b_fmtp: partner_fmtp,
            });
        }
        Ok(())
    }
}

/// Whether two negotiated `a=fmtp` strings describe the same wire format.
///
/// Only parameters that change the bytes on the wire matter, and **which
/// parameters those are depends on the codec**. This keyed on parameter names
/// alone, which had two consequences: a peer-supplied nonstandard `channels=2`
/// on a PCMU or Opus leg refused a relay that works, and AMR's rules were
/// reimplemented here rather than taken from the codec that owns them.
///
/// # AMR
///
/// RFC 4867 §8.3.1: "Each combination of the RTP payload transport format
/// configuration parameters (octet-align, crc, robust-sorting, interleaving,
/// and channels) is unique in its bit-pattern and not compatible with any
/// other combination."
///
/// That rule lives in `codec-core`'s [`AmrFmtp::same_transport_as`], and this
/// calls it rather than restating it. A local restatement had already drifted:
/// `octet-align` was `value == "1"` where the parser accepts the flag values
/// RFC 4867 defines and rejects the rest, and `channels` fell back to 1 on a
/// malformed value where the parser refuses the session. Two implementations
/// of one rule is one more than can be kept correct.
///
/// An fmtp that will not parse is treated as *not* interchangeable with
/// anything, including an identical unparseable one. A bridge is not the place
/// to decide what a malformed parameter meant.
///
/// # G.729
///
/// `annexb` genuinely changes the wire — it decides whether SID frames may
/// appear at all — so it is compared. Absent means `yes`, per RFC 3555.
///
/// # Everything else
///
/// Interchangeable. Opus's `maxaveragebitrate`, `stereo`, `useinbandfec` and
/// the rest constrain what an encoder produces, not how a frame is laid out,
/// and a transparent relay forwards whatever arrives. `mode-set` is the same
/// story for AMR: it restricts which modes may be used, and each leg's own
/// negotiation already bound its peer.
fn wire_formats_are_interchangeable(a_codec: &str, a: &str, b_codec: &str, b: &str) -> bool {
    if !a_codec.eq_ignore_ascii_case(b_codec) {
        return false;
    }

    #[cfg(any(feature = "amr-nb", feature = "amr-wb"))]
    {
        use codec_core::codecs::amr::mode::AmrVariant;
        use codec_core::codecs::amr::sdp::AmrFmtp;

        let variant = if a_codec.eq_ignore_ascii_case("AMR-WB") {
            Some(AmrVariant::WideBand)
        } else if a_codec.eq_ignore_ascii_case("AMR") {
            Some(AmrVariant::NarrowBand)
        } else {
            None
        };
        if let Some(variant) = variant {
            let (Ok(parsed_a), Ok(parsed_b)) =
                (AmrFmtp::parse(variant, a), AmrFmtp::parse(variant, b))
            else {
                return false;
            };
            return parsed_a.same_transport_as(&parsed_b);
        }
    }

    if normalized_codec_name(a_codec).starts_with("G729") {
        return g729_annexb(a) == g729_annexb(b);
    }

    true
}

/// G.729's `annexb`, defaulting to enabled as RFC 3555 specifies.
fn g729_annexb(fmtp: &str) -> bool {
    for part in fmtp.split(';') {
        let Some((name, value)) = part.trim().split_once('=') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("annexb") {
            return !value.trim().trim_matches('"').eq_ignore_ascii_case("no");
        }
    }
    true
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

/// Per-direction state for the transcoding bridge forwarder.
struct TranscodeDirection {
    /// Converts decoded source PCM into the destination leg's negotiated
    /// rate/channel shape.
    converter: FormatConverter,
    /// Destination negotiated shape (the encoder uses the destination runtime
    /// fetched per packet, so a re-INVITE generation change is picked up).
    dst_clock_rate: u32,
    dst_channels: u8,
    /// Source negotiated shape.
    src_payload_type: u8,
    src_clock_rate: u32,
    src_channels: u8,
    /// Source RTP timestamp at the first packet; the destination timestamp is
    /// `src_ts0 + (src_ts - src_ts0) * dst_rate / src_rate`.
    src_ts0: Option<u32>,
}

impl TranscodeDirection {
    /// Map a source RTP timestamp into the destination audio clock.
    fn map_timestamp(&mut self, src_ts: u32) -> u32 {
        let ts0 = *self.src_ts0.get_or_insert(src_ts);
        scale_timestamp(ts0, src_ts, self.src_clock_rate, self.dst_clock_rate)
    }
}

/// `ts0 + (ts - ts0) * to_rate / from_rate`, with `ts - ts0` taken as a
/// signed 32-bit RTP timestamp distance so a wrap does not jump.
fn scale_timestamp(ts0: u32, ts: u32, from_rate: u32, to_rate: u32) -> u32 {
    if from_rate == to_rate || from_rate == 0 {
        return ts;
    }
    let delta = i64::from(ts.wrapping_sub(ts0) as i32) * i64::from(to_rate);
    ts0.wrapping_add((delta / i64::from(from_rate)) as u32)
}

/// Counters of a transcoding bridge's non-audio traffic, shared by both of
/// its directions.
#[derive(Debug, Default)]
struct TranscodeCounters {
    telephone_event_relayed: AtomicU64,
    telephone_event_dropped: AtomicU64,
    comfort_noise_dropped: AtomicU64,
    unknown_payload_dropped: AtomicU64,
}

/// What a transcoding bridge did with packets that are not the source leg's
/// negotiated audio payload, across both directions. See
/// [`BridgeHandle::transcode_stats`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct TranscodeBridgeStats {
    /// RFC 4733 telephone-event packets relayed to a leg that negotiated
    /// telephone-event, under that leg's payload type and clock.
    pub telephone_event_relayed: u64,
    /// Telephone-event packets dropped because the destination leg did not
    /// negotiate telephone-event.
    pub telephone_event_dropped: u64,
    /// Comfort-noise (RFC 3389, payload type 13) packets dropped; the
    /// transcoder does not carry silence descriptors across codecs.
    pub comfort_noise_dropped: u64,
    /// Packets with any other payload type the source leg did not negotiate
    /// as audio or telephone-event, dropped.
    pub unknown_payload_dropped: u64,
}

impl TranscodeCounters {
    fn snapshot(&self) -> TranscodeBridgeStats {
        TranscodeBridgeStats {
            telephone_event_relayed: self.telephone_event_relayed.load(Ordering::Relaxed),
            telephone_event_dropped: self.telephone_event_dropped.load(Ordering::Relaxed),
            comfort_noise_dropped: self.comfort_noise_dropped.load(Ordering::Relaxed),
            unknown_payload_dropped: self.unknown_payload_dropped.load(Ordering::Relaxed),
        }
    }
}

/// RFC 3389 comfort noise, static payload type 13.
const COMFORT_NOISE_PAYLOAD_TYPE: u8 = 13;

/// The payload type rtp-core decodes as RFC 4733 telephone-event and hands
/// to sessions as `RtpSessionEvent::DtmfReceived` rather than as a packet.
const RTP_CORE_TELEPHONE_EVENT_PAYLOAD_TYPE: u8 = 101;

/// The RFC 4733 default telephone-event clock, assumed for a source leg
/// whose configuration does not name its telephone-event format.
const DEFAULT_TELEPHONE_EVENT_CLOCK_RATE: u32 = 8_000;

/// One RFC 4733 §2.3 event block: event, `E` bit and volume, duration.
fn telephone_event_payload(
    event: u8,
    end_of_event: bool,
    volume: u8,
    duration: u16,
) -> bytes::Bytes {
    let flags = (u8::from(end_of_event) << 7) | (volume & 0x3f);
    let [high, low] = duration.to_be_bytes();
    bytes::Bytes::from(vec![event, flags, high, low])
}

/// The fate of one non-audio packet on a transcoding bridge.
#[derive(Debug, PartialEq, Eq)]
enum AuxiliaryPacket {
    /// Send this telephone-event packet to the destination as given.
    RelayTelephoneEvent {
        payload_type: u8,
        timestamp: u32,
        payload: bytes::Bytes,
    },
    /// Telephone-event the destination leg did not negotiate.
    DropTelephoneEvent,
    /// RFC 3389 comfort noise.
    DropComfortNoise,
    /// Any other payload type.
    DropUnknown,
}

/// Decide what a transcoding bridge does with a packet that is not the
/// source leg's audio payload type.
///
/// A source telephone-event packet (RFC 4733) is relayed only when the
/// destination leg negotiated telephone-event: its payload type becomes the
/// destination's, and its timestamp and every event block's duration field
/// are rescaled from the source's telephone-event clock to the destination's
/// around `ts0`, the anchor the audio timestamps share, so events stay on the
/// audio they overlay when each leg's event clock is its audio clock.
fn translate_auxiliary_packet(
    payload_type: u8,
    timestamp: u32,
    payload: &bytes::Bytes,
    ts0: u32,
    src_telephone_event: Option<TelephoneEventFormat>,
    dst_telephone_event: Option<TelephoneEventFormat>,
) -> AuxiliaryPacket {
    match src_telephone_event {
        Some(src) if src.payload_type == payload_type => {
            let Some(dst) = dst_telephone_event else {
                return AuxiliaryPacket::DropTelephoneEvent;
            };
            let payload = if src.clock_rate == dst.clock_rate || src.clock_rate == 0 {
                payload.clone()
            } else {
                // Each 4-byte event block carries a 16-bit duration in
                // timestamp units (RFC 4733 §2.3); rescale every block.
                let mut rescaled = payload.to_vec();
                for block in rescaled.chunks_exact_mut(4) {
                    let duration = u64::from(u16::from_be_bytes([block[2], block[3]]));
                    let scaled = (duration * u64::from(dst.clock_rate) / u64::from(src.clock_rate))
                        .min(u64::from(u16::MAX)) as u16;
                    block[2..4].copy_from_slice(&scaled.to_be_bytes());
                }
                bytes::Bytes::from(rescaled)
            };
            AuxiliaryPacket::RelayTelephoneEvent {
                payload_type: dst.payload_type,
                timestamp: scale_timestamp(ts0, timestamp, src.clock_rate, dst.clock_rate),
                payload,
            }
        }
        _ if payload_type == COMFORT_NOISE_PAYLOAD_TYPE => AuxiliaryPacket::DropComfortNoise,
        _ => AuxiliaryPacket::DropUnknown,
    }
}

/// Destination RTP timestamp and marker for each packet the destination
/// encoder produced from one source packet.
///
/// The encoder stamps its packets in the destination clock from the frame's
/// own timestamp, so each packet after the first starts `packet.timestamp -
/// first.timestamp` destination ticks after it: the duration of the packets
/// before it, including any interval G.729 Annex B suppressed. The source
/// marker goes on the first packet only; an encoder's own marker (a G.729
/// talkspurt start) is kept.
fn stamp_encoded_packets(
    dst_ts: u32,
    source_marker: bool,
    packets: &[super::codec_runtime::EncodedAudioPacket],
) -> Vec<(u32, bool)> {
    let Some(first) = packets.first() else {
        return Vec::new();
    };
    packets
        .iter()
        .enumerate()
        .map(|(index, packet)| {
            (
                dst_ts.wrapping_add(packet.timestamp.wrapping_sub(first.timestamp)),
                (index == 0 && source_marker) || packet.marker,
            )
        })
        .collect()
}

/// A leg's negotiated telephone-event format, read from its live session
/// configuration so a re-INVITE that changes it is followed.
fn leg_telephone_event(
    sessions: &DashMap<DialogId, super::types::MediaSessionInfo>,
    dialog: &DialogId,
) -> Option<TelephoneEventFormat> {
    sessions
        .get(dialog)
        .and_then(|entry| entry.value().config.telephone_event())
}

/// Send one packet on the destination leg, under `payload_type` when given
/// and the session's negotiated audio payload type otherwise.
async fn send_on_destination(
    dst_send_handle: &Option<RtpSendHandle>,
    dst_session: &Arc<Mutex<RtpSession>>,
    timestamp: u32,
    payload: bytes::Bytes,
    marker: bool,
    payload_type: Option<u8>,
) -> std::result::Result<(), String> {
    let result = match (dst_send_handle, payload_type) {
        (Some(handle), Some(pt)) => {
            handle
                .send_packet_with_pt(timestamp, payload, marker, pt)
                .await
        }
        (Some(handle), None) => handle.send_packet(timestamp, payload, marker).await,
        (None, Some(pt)) => {
            dst_session
                .lock()
                .await
                .send_packet_with_pt(timestamp, payload, marker, pt)
                .await
        }
        (None, None) => {
            dst_session
                .lock()
                .await
                .send_packet(timestamp, payload, marker)
                .await
        }
    };
    result.map_err(|error| error.to_string())
}

/// Transcoding variant of [`forward_rtp`]: decode with the *source* leg's
/// negotiated codec, convert the PCM to the *destination* leg's negotiated
/// rate/channel shape, re-encode with the destination codec, and send on the
/// destination session.
///
/// Packets that are not the source leg's audio payload type bypass the codec
/// path (see [`translate_auxiliary_packet`]): RFC 4733 telephone-event is
/// relayed under the destination leg's telephone-event payload type and
/// clock when that leg negotiated it ([`MediaConfig::with_telephone_event`])
/// and dropped otherwise; comfort noise and unknown payload types are
/// dropped. Every drop is counted in [`TranscodeBridgeStats`]. Telephone
/// events on payload type 101 arrive already decoded
/// (`RtpSessionEvent::DtmfReceived`) and are rebuilt into RFC 4733 packets.
///
/// The destination codec runtime is re-read from the controller whenever its
/// generation changes, so a re-INVITE that renegotiates either leg's codec is
/// absorbed without tearing the bridge down (`update_media` rebuilds the
/// runtime; this task only observes the new generation).
#[allow(clippy::too_many_arguments)]
async fn forward_rtp_transcode(
    src: DialogId,
    dst: DialogId,
    mut events: broadcast::Receiver<RtpSessionEvent>,
    dst_send_handle: Option<RtpSendHandle>,
    dst_session: Arc<Mutex<RtpSession>>,
    codec_runtimes: Arc<DashMap<DialogId, Arc<super::codec_runtime::DialogCodecRuntime>>>,
    sessions: Arc<DashMap<DialogId, super::types::MediaSessionInfo>>,
    counters: Arc<TranscodeCounters>,
    cancel: Arc<AtomicBool>,
) {
    debug!("🔗 transcode bridge forwarder started: {} -> {}", src, dst);

    // The first packet resolves both runtimes' formats; nothing can be sent
    // before that anyway because the bridge is only created after both legs
    // are active (remote address known).
    let mut direction: Option<TranscodeDirection> = None;
    // Timestamp of the last telephone event relayed, so the first packet of
    // each new event carries the RTP marker.
    let mut last_event_timestamp: Option<u32> = None;

    loop {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        match events.recv().await {
            Ok(RtpSessionEvent::PacketReceived(packet)) => {
                if cancel.load(Ordering::SeqCst) {
                    break;
                }

                // Resolve/refresh the destination runtime. A generation change
                // (re-INVITE) also resets the converter and timestamp anchor so
                // the new rate is applied from a clean state.
                let Some(dst_runtime) = codec_runtimes.get(&dst).map(|entry| entry.value().clone())
                else {
                    debug!("🔗 transcode forward {src}->{dst}: destination runtime gone");
                    break;
                };
                let Some(src_runtime) = codec_runtimes.get(&src).map(|entry| entry.value().clone())
                else {
                    debug!("🔗 transcode forward {src}->{dst}: source runtime gone");
                    break;
                };

                let shape = (
                    dst_runtime.format.clock_rate,
                    dst_runtime.format.channels,
                    src_runtime.format.payload_type,
                    src_runtime.format.clock_rate,
                    src_runtime.format.channels,
                );
                let rebuild = match &direction {
                    Some(existing) => {
                        (
                            existing.dst_clock_rate,
                            existing.dst_channels,
                            existing.src_payload_type,
                            existing.src_clock_rate,
                            existing.src_channels,
                        ) != shape
                    }
                    None => true,
                };
                if rebuild {
                    let (
                        dst_clock_rate,
                        dst_channels,
                        src_payload_type,
                        src_clock_rate,
                        src_channels,
                    ) = shape;
                    direction = Some(TranscodeDirection {
                        converter: FormatConverter::new(),
                        dst_clock_rate,
                        dst_channels,
                        src_payload_type,
                        src_clock_rate,
                        src_channels,
                        src_ts0: None,
                    });
                    debug!(
                        "🔗 transcode forward {src}->{dst}: {}Hz/{}ch (PT {}) -> {}Hz/{}ch (PT {})",
                        src_clock_rate,
                        src_channels,
                        src_payload_type,
                        dst_clock_rate,
                        dst_channels,
                        dst_runtime.format.payload_type
                    );
                }
                let Some(direction_state) = direction.as_mut() else {
                    continue;
                };

                let payload = packet.payload.clone();
                let ts = packet.header.timestamp;
                let marker = packet.header.marker;
                let packet_pt = packet.header.payload_type;

                if packet_pt != direction_state.src_payload_type {
                    let ts0 = *direction_state.src_ts0.get_or_insert(ts);
                    match translate_auxiliary_packet(
                        packet_pt,
                        ts,
                        &payload,
                        ts0,
                        leg_telephone_event(&sessions, &src),
                        leg_telephone_event(&sessions, &dst),
                    ) {
                        AuxiliaryPacket::RelayTelephoneEvent {
                            payload_type,
                            timestamp,
                            payload,
                        } => {
                            counters
                                .telephone_event_relayed
                                .fetch_add(1, Ordering::Relaxed);
                            if let Err(e) = send_on_destination(
                                &dst_send_handle,
                                &dst_session,
                                timestamp,
                                payload,
                                marker,
                                Some(payload_type),
                            )
                            .await
                            {
                                warn!(
                                    "transcode bridge {src}->{dst} telephone-event send failed: {e}"
                                );
                            }
                        }
                        AuxiliaryPacket::DropTelephoneEvent => {
                            counters
                                .telephone_event_dropped
                                .fetch_add(1, Ordering::Relaxed);
                            debug!(
                                "transcode bridge {src}->{dst}: destination has no telephone-event; dropping DTMF"
                            );
                        }
                        AuxiliaryPacket::DropComfortNoise => {
                            counters
                                .comfort_noise_dropped
                                .fetch_add(1, Ordering::Relaxed);
                        }
                        AuxiliaryPacket::DropUnknown => {
                            counters
                                .unknown_payload_dropped
                                .fetch_add(1, Ordering::Relaxed);
                            debug!(
                                "transcode bridge {src}->{dst}: dropping unnegotiated PT={packet_pt}"
                            );
                        }
                    }
                    continue;
                }

                // Decode with the source codec, convert to the destination
                // shape, and re-encode with the destination codec.
                let decoded = match src_runtime.decode_source(&payload, ts, false).await {
                    Ok(frame) => frame,
                    Err(error) => {
                        debug!("transcode bridge {src}->{dst} decode failed: {error}");
                        continue;
                    }
                };
                let Some(target_rate) = SampleRate::from_hz(direction_state.dst_clock_rate) else {
                    // Validated at bridge creation; unreachable in practice.
                    continue;
                };
                let converted = match direction_state.converter.convert_frame(
                    &decoded,
                    &ConversionParams::new(target_rate, direction_state.dst_channels),
                ) {
                    Ok(result) => result.frame,
                    Err(error) => {
                        debug!("transcode bridge {src}->{dst} convert failed: {error}");
                        continue;
                    }
                };
                let dst_ts = direction_state.map_timestamp(ts);
                let encoded = match dst_runtime.encode_packets(&converted).await {
                    Ok(packets) => packets,
                    Err(error) => {
                        debug!("transcode bridge {src}->{dst} encode failed: {error}");
                        continue;
                    }
                };
                // The bridge's mapping keeps the stream continuous across
                // source timestamp wraps; packets after the first advance by
                // the destination-clock duration before them.
                let stamps = stamp_encoded_packets(dst_ts, marker, &encoded);
                for (encoded_packet, (packet_ts, packet_marker)) in encoded.into_iter().zip(stamps)
                {
                    if let Err(e) = send_on_destination(
                        &dst_send_handle,
                        &dst_session,
                        packet_ts,
                        bytes::Bytes::from(encoded_packet.payload),
                        packet_marker,
                        None,
                    )
                    .await
                    {
                        warn!("transcode bridge {src}->{dst} send failed: {e}");
                    }
                }
            }
            // rtp-core turns RFC 4733 packets on payload type 101 into
            // decoded events (dropping repeated end-of-event frames) instead
            // of `PacketReceived`; rebuild each one for the destination.
            Ok(RtpSessionEvent::DtmfReceived {
                event,
                end_of_event,
                volume,
                duration,
                timestamp,
                ..
            }) => {
                if cancel.load(Ordering::SeqCst) {
                    break;
                }
                let ts0 = direction
                    .as_mut()
                    .map_or(timestamp, |state| *state.src_ts0.get_or_insert(timestamp));
                let src_format = leg_telephone_event(&sessions, &src)
                    .filter(|format| format.payload_type == RTP_CORE_TELEPHONE_EVENT_PAYLOAD_TYPE)
                    .unwrap_or(TelephoneEventFormat {
                        payload_type: RTP_CORE_TELEPHONE_EVENT_PAYLOAD_TYPE,
                        clock_rate: DEFAULT_TELEPHONE_EVENT_CLOCK_RATE,
                    });
                let payload = telephone_event_payload(event, end_of_event, volume, duration);
                match translate_auxiliary_packet(
                    src_format.payload_type,
                    timestamp,
                    &payload,
                    ts0,
                    Some(src_format),
                    leg_telephone_event(&sessions, &dst),
                ) {
                    AuxiliaryPacket::RelayTelephoneEvent {
                        payload_type,
                        timestamp: dst_timestamp,
                        payload,
                    } => {
                        counters
                            .telephone_event_relayed
                            .fetch_add(1, Ordering::Relaxed);
                        let marker = last_event_timestamp != Some(timestamp);
                        last_event_timestamp = Some(timestamp);
                        // Only the first end-of-event frame survives rtp-core's
                        // dedup; send it three times, as RFC 4733 senders do.
                        let copies = if end_of_event { 3 } else { 1 };
                        for copy in 0..copies {
                            if let Err(e) = send_on_destination(
                                &dst_send_handle,
                                &dst_session,
                                dst_timestamp,
                                payload.clone(),
                                marker && copy == 0,
                                Some(payload_type),
                            )
                            .await
                            {
                                warn!(
                                    "transcode bridge {src}->{dst} telephone-event send failed: {e}"
                                );
                            }
                        }
                    }
                    _ => {
                        counters
                            .telephone_event_dropped
                            .fetch_add(1, Ordering::Relaxed);
                        debug!(
                            "transcode bridge {src}->{dst}: destination has no telephone-event; dropping DTMF"
                        );
                    }
                }
            }
            Ok(_) => {}
            Err(broadcast::error::RecvError::Lagged(n)) => {
                warn!("transcode bridge {src}->{dst} lagged {n} events");
            }
            Err(broadcast::error::RecvError::Closed) => {
                debug!("🔗 transcode bridge source closed: {src}");
                break;
            }
        }
    }
    debug!("🔗 transcode bridge forwarder exited: {} -> {}", src, dst);
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

    /// A PCMU leg and a PCMA leg cannot relay (different payload types) but a
    /// transcode bridge moves audio between them: PCMU payloads decoded on the
    /// PCMU leg arrive as PCMA payloads (PT 8) on the PCMA leg, and vice versa.
    #[tokio::test]
    async fn transcode_bridge_moves_audio_between_different_codecs() {
        use bytes::Bytes;
        use rvoip_rtp_core::packet::rtp::RtpPacket;

        let controller = MediaSessionController::new();
        let a = DialogId::new("xcode-a");
        let b = DialogId::new("xcode-b");
        controller
            .start_media(a.clone(), test_config("PCMU"))
            .await
            .unwrap();
        controller
            .start_media(b.clone(), test_config("PCMA"))
            .await
            .unwrap();

        // Peers for both legs so the sessions have remote addresses and the
        // test can drive/observe real RTP.
        let peer_a = Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let peer_b = Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap());
        for (dialog, peer) in [(&a, &peer_a), (&b, &peer_b)] {
            let info = controller.get_session_info(dialog).await.unwrap();
            let mut config = info.config.clone();
            config.remote_addr = Some(peer.local_addr().unwrap());
            controller
                .update_media(dialog.clone(), config)
                .await
                .unwrap();
        }
        let a_port = controller
            .get_session_info(&a)
            .await
            .unwrap()
            .rtp_port
            .unwrap();
        let b_port = controller
            .get_session_info(&b)
            .await
            .unwrap()
            .rtp_port
            .unwrap();

        // Relay refuses the pair; transcode accepts it.
        let relay_error = expect_err(controller.bridge_sessions(a.clone(), b.clone()).await);
        assert!(
            matches!(relay_error, BridgeError::CodecMismatch { .. }),
            "expected CodecMismatch for relay, got {relay_error:?}"
        );
        let handle = expect_ok(
            controller
                .bridge_sessions_with_mode(a.clone(), b.clone(), BridgeMode::Transcode)
                .await,
        );
        assert_eq!(handle.mode(), BridgeMode::Transcode);

        // Drive A → B: PCMU (PT 0) in, PCMA (PT 8) out.
        let send_a = {
            let peer_a = Arc::clone(&peer_a);
            tokio::spawn(async move {
                for i in 0..40u16 {
                    let payload = vec![0xD5u8; 160]; // mid-range A-law-adjacent pattern
                    let packet = RtpPacket::new_with_payload(
                        0,
                        i,
                        u32::from(i) * 160,
                        0x1234_5678,
                        Bytes::from(payload),
                    );
                    let bytes = packet.serialize().expect("serialize PCMU packet");
                    peer_a
                        .send_to(&bytes, format!("127.0.0.1:{a_port}"))
                        .await
                        .expect("send PCMU toward leg A");
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
        };

        // Expect PCMA (PT 8) payloads on B's peer.
        let mut received_pt8 = 0usize;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline && received_pt8 < 5 {
            let mut buf = vec![0u8; 2048];
            let (len, _) = tokio::time::timeout(
                std::time::Duration::from_millis(500),
                peer_b.recv_from(&mut buf),
            )
            .await
            .expect("timed out waiting for transcoded RTP on leg B")
            .expect("recv_from leg B peer");
            let packet = RtpPacket::parse(&buf[..len]).expect("parse RTP from leg B");
            if packet.header.payload_type == 8 {
                received_pt8 += 1;
                assert_eq!(
                    packet.payload.len(),
                    160,
                    "transcoded PCMA payload must stay 20 ms at 8 kHz"
                );
            } else {
                panic!(
                    "unexpected payload type {} on leg B",
                    packet.header.payload_type
                );
            }
        }
        assert!(
            received_pt8 >= 5,
            "expected several transcoded PCMA packets, got {received_pt8}"
        );
        send_a.abort();

        // Drive B → A: PCMA (PT 8) in, PCMU (PT 0) out.
        let send_b = {
            let peer_b = Arc::clone(&peer_b);
            tokio::spawn(async move {
                for i in 0..40u16 {
                    let payload = vec![0xD5u8; 160];
                    let packet = RtpPacket::new_with_payload(
                        8,
                        i,
                        u32::from(i) * 160,
                        0x8765_4321,
                        Bytes::from(payload),
                    );
                    let bytes = packet.serialize().expect("serialize PCMA packet");
                    peer_b
                        .send_to(&bytes, format!("127.0.0.1:{b_port}"))
                        .await
                        .expect("send PCMA toward leg B");
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
        };

        let mut received_pt0 = 0usize;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline && received_pt0 < 5 {
            let mut buf = vec![0u8; 2048];
            let (len, _) = tokio::time::timeout(
                std::time::Duration::from_millis(500),
                peer_a.recv_from(&mut buf),
            )
            .await
            .expect("timed out waiting for transcoded RTP on leg A")
            .expect("recv_from leg A peer");
            let packet = RtpPacket::parse(&buf[..len]).expect("parse RTP from leg A");
            assert_eq!(
                packet.header.payload_type, 0,
                "unexpected payload type {} on leg A",
                packet.header.payload_type
            );
            received_pt0 += 1;
            assert_eq!(packet.payload.len(), 160);
        }
        assert!(
            received_pt0 >= 5,
            "expected several transcoded PCMU packets, got {received_pt0}"
        );
        send_b.abort();

        drop(handle);
        assert!(!controller.is_bridged(&a) && !controller.is_bridged(&b));
    }

    #[test]
    fn transcode_timestamps_scale_around_the_anchor_and_across_a_wrap() {
        assert_eq!(scale_timestamp(1_000, 1_160, 8_000, 16_000), 1_320);
        assert_eq!(scale_timestamp(1_000, 1_320, 16_000, 8_000), 1_160);
        assert_eq!(scale_timestamp(7, 1_234, 8_000, 8_000), 1_234);
        // 160 ticks past a wrap is still 160 ticks, doubled.
        let ts0 = u32::MAX - 79;
        assert_eq!(
            scale_timestamp(ts0, 80, 8_000, 16_000),
            ts0.wrapping_add(320)
        );
    }

    #[test]
    fn telephone_event_takes_the_destination_payload_type() {
        let payload = bytes::Bytes::from_static(&[5, 0x0a, 0x01, 0x40]);
        let src = TelephoneEventFormat {
            payload_type: 101,
            clock_rate: 8_000,
        };
        let dst = TelephoneEventFormat {
            payload_type: 96,
            clock_rate: 8_000,
        };
        assert_eq!(
            translate_auxiliary_packet(101, 4_000, &payload, 3_840, Some(src), Some(dst)),
            AuxiliaryPacket::RelayTelephoneEvent {
                payload_type: 96,
                timestamp: 4_000,
                payload: payload.clone(),
            }
        );
        // No telephone-event on the destination: dropped, not relayed raw.
        assert_eq!(
            translate_auxiliary_packet(101, 4_000, &payload, 3_840, Some(src), None),
            AuxiliaryPacket::DropTelephoneEvent
        );
    }

    #[test]
    fn telephone_event_timestamp_and_duration_follow_the_destination_clock() {
        // Two blocks (RFC 4733 redundancy): durations 320 and 160 at 8 kHz.
        let payload = bytes::Bytes::from_static(&[5, 0x0a, 0x01, 0x40, 4, 0x8a, 0x00, 0xa0]);
        let src = TelephoneEventFormat {
            payload_type: 101,
            clock_rate: 8_000,
        };
        let dst = TelephoneEventFormat {
            payload_type: 97,
            clock_rate: 16_000,
        };
        match translate_auxiliary_packet(101, 1_160, &payload, 1_000, Some(src), Some(dst)) {
            AuxiliaryPacket::RelayTelephoneEvent {
                payload_type,
                timestamp,
                payload,
            } => {
                assert_eq!(payload_type, 97);
                assert_eq!(timestamp, 1_320, "160 ticks at 8 kHz are 320 at 16 kHz");
                assert_eq!(
                    payload.as_ref(),
                    &[5, 0x0a, 0x02, 0x80, 4, 0x8a, 0x01, 0x40],
                    "event, E bit and volume kept; each duration doubled"
                );
            }
            other => panic!("expected a relay, got {other:?}"),
        }
    }

    #[test]
    fn comfort_noise_and_unknown_payloads_are_dropped() {
        let payload = bytes::Bytes::from_static(&[0x40]);
        let te = TelephoneEventFormat {
            payload_type: 101,
            clock_rate: 8_000,
        };
        assert_eq!(
            translate_auxiliary_packet(13, 1, &payload, 0, Some(te), Some(te)),
            AuxiliaryPacket::DropComfortNoise
        );
        assert_eq!(
            translate_auxiliary_packet(99, 1, &payload, 0, Some(te), Some(te)),
            AuxiliaryPacket::DropUnknown
        );
        // Telephone-event the source did not negotiate is unknown, too.
        assert_eq!(
            translate_auxiliary_packet(101, 1, &payload, 0, None, Some(te)),
            AuxiliaryPacket::DropUnknown
        );
    }

    #[test]
    fn multi_packet_output_advances_by_each_packets_destination_duration() {
        use super::super::codec_runtime::EncodedAudioPacket;
        let packet = |timestamp: u32, marker: bool| EncodedAudioPacket {
            payload: vec![0; 10],
            timestamp,
            marker,
        };
        // Three 10 ms G.729 packets at 8 kHz from one frame stamped 5_000,
        // the second interval suppressed (Annex B), the third a new
        // talkspurt the encoder marked.
        let encoded = [
            packet(5_000, false),
            packet(5_160, false),
            packet(5_240, true),
        ];
        assert_eq!(
            stamp_encoded_packets(90_000, true, &encoded),
            vec![(90_000, true), (90_160, false), (90_240, true)]
        );
        // The source marker is not repeated on later packets.
        assert_eq!(
            stamp_encoded_packets(u32::MAX - 79, false, &encoded[..2]),
            vec![(u32::MAX - 79, false), (80, false)]
        );
        assert!(stamp_encoded_packets(1, true, &[]).is_empty());
    }

    #[test]
    fn media_config_round_trips_the_telephone_event_format() {
        let format = TelephoneEventFormat {
            payload_type: 96,
            clock_rate: 16_000,
        };
        let config = test_config("PCMU").with_telephone_event(Some(format));
        assert_eq!(config.telephone_event(), Some(format));
        assert_eq!(config.with_telephone_event(None).telephone_event(), None);
    }

    /// End to end: PT 101 DTMF from a PCMU leg reaches a PCMA leg that
    /// negotiated telephone-event as PT 96, and is dropped and counted once
    /// that leg has no telephone-event.
    #[tokio::test]
    async fn transcode_bridge_relays_dtmf_under_the_destination_payload_type() {
        use bytes::Bytes;
        use rvoip_rtp_core::packet::rtp::RtpPacket;

        let controller = MediaSessionController::new();
        let a = DialogId::new("xcode-dtmf-a");
        let b = DialogId::new("xcode-dtmf-b");
        let te = |payload_type| {
            Some(TelephoneEventFormat {
                payload_type,
                clock_rate: 8_000,
            })
        };
        controller
            .start_media(a.clone(), test_config("PCMU").with_telephone_event(te(101)))
            .await
            .unwrap();
        controller
            .start_media(b.clone(), test_config("PCMA").with_telephone_event(te(96)))
            .await
            .unwrap();
        let peer_a = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let peer_b = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        for (dialog, peer) in [(&a, &peer_a), (&b, &peer_b)] {
            let info = controller.get_session_info(dialog).await.unwrap();
            let mut config = info.config.clone();
            config.remote_addr = Some(peer.local_addr().unwrap());
            controller
                .update_media(dialog.clone(), config)
                .await
                .unwrap();
        }
        let a_port = controller
            .get_session_info(&a)
            .await
            .unwrap()
            .rtp_port
            .unwrap();
        let handle = expect_ok(
            controller
                .bridge_sessions_with_mode(a.clone(), b.clone(), BridgeMode::Transcode)
                .await,
        );

        // One digit: a start frame and an end frame, at one timestamp.
        let send_digit = |timestamp: u32, sequence: u16| {
            let peer_a = &peer_a;
            async move {
                for (offset, flags) in [(0u16, 0x0au8), (1, 0x8a)] {
                    let packet = RtpPacket::new_with_payload(
                        101,
                        sequence + offset,
                        timestamp,
                        0x1234_5678,
                        Bytes::from(vec![5, flags, 0x01, 0x40]),
                    );
                    peer_a
                        .send_to(
                            &packet.serialize().expect("serialize DTMF"),
                            format!("127.0.0.1:{a_port}"),
                        )
                        .await
                        .expect("send DTMF toward leg A");
                }
            }
        };

        send_digit(8_000, 1).await;
        let mut received = Vec::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline && received.len() < 4 {
            let mut buf = vec![0u8; 2048];
            let Ok(Ok((len, _))) = tokio::time::timeout(
                std::time::Duration::from_millis(500),
                peer_b.recv_from(&mut buf),
            )
            .await
            else {
                continue;
            };
            let packet = RtpPacket::parse(&buf[..len]).expect("parse RTP from leg B");
            received.push((
                packet.header.payload_type,
                packet.header.marker,
                packet.payload.to_vec(),
            ));
        }
        // Start frame (marked) plus the end frame sent three times.
        assert_eq!(
            received,
            vec![
                (96, true, vec![5, 0x0a, 0x01, 0x40]),
                (96, false, vec![5, 0x8a, 0x01, 0x40]),
                (96, false, vec![5, 0x8a, 0x01, 0x40]),
                (96, false, vec![5, 0x8a, 0x01, 0x40]),
            ]
        );
        assert_eq!(handle.transcode_stats().unwrap().telephone_event_relayed, 2);

        // Leg B renegotiates without telephone-event: DTMF is dropped.
        let config = controller
            .get_session_config(&b)
            .unwrap()
            .with_telephone_event(None);
        controller.update_media(b.clone(), config).await.unwrap();
        send_digit(16_000, 10).await;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline
            && handle.transcode_stats().unwrap().telephone_event_dropped < 2
        {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let stats = handle.transcode_stats().unwrap();
        assert_eq!(stats.telephone_event_dropped, 2);
        assert_eq!(stats.telephone_event_relayed, 2);
        let mut buf = vec![0u8; 2048];
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(300),
                peer_b.recv_from(&mut buf)
            )
            .await
            .is_err(),
            "no DTMF may reach a leg without telephone-event"
        );
        drop(handle);
    }

    #[tokio::test]
    async fn relay_bridge_reports_no_transcode_stats() {
        let controller = MediaSessionController::new();
        let a = DialogId::new("relay-stats-a");
        let b = DialogId::new("relay-stats-b");
        controller
            .start_media(a.clone(), test_config("PCMU"))
            .await
            .unwrap();
        controller
            .start_media(b.clone(), test_config("PCMU"))
            .await
            .unwrap();
        let handle = expect_ok(controller.bridge_sessions(a.clone(), b.clone()).await);
        assert_eq!(handle.mode(), BridgeMode::Relay);
        assert_eq!(handle.transcode_stats(), None);
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

    /// A re-INVITE that flips the framing on one leg of a live bridge is
    /// refused, and the previous configuration survives.
    ///
    /// The guard ran once, at bridge setup. `update_media` touches the codec
    /// runtimes, the RTP sessions and the published configs but never looked
    /// at `bridge_partners` — so this door was open even while the front one
    /// was watched. Both halves matter: that the update fails, and that the
    /// session it failed on is unchanged rather than half-applied.
    #[tokio::test]
    #[cfg(feature = "amr-nb")]
    async fn a_reinvite_may_not_break_a_live_bridges_framing() {
        let controller = MediaSessionController::new();
        let a = DialogId::new("reinvite-a");
        let b = DialogId::new("reinvite-b");

        controller
            .start_media(a.clone(), amr_config_with_fmtp("octet-align=1"))
            .await
            .unwrap();
        controller
            .start_media(b.clone(), amr_config_with_fmtp("octet-align=1"))
            .await
            .unwrap();
        let _handle = expect_ok(controller.bridge_sessions(a.clone(), b.clone()).await);

        // The same framing is still fine -- otherwise this test would pass
        // for a hook that refused every re-INVITE.
        controller
            .update_media(a.clone(), amr_config_with_fmtp("octet-align=1"))
            .await
            .expect("an unchanged framing must still be accepted");

        let err = controller
            .update_media(a.clone(), amr_config_with_fmtp("octet-align=0"))
            .await
            .expect_err("flipping the framing under a live bridge must be refused");
        assert!(err.to_string().contains("format mismatch"), "{err}");

        // And the leg still carries what it had.
        let carried = controller
            .sessions
            .get(&a)
            .map(|entry| {
                entry
                    .value()
                    .config
                    .parameters
                    .get(NEGOTIATED_FMTP_PARAMETER)
                    .cloned()
            })
            .expect("the session survived");
        assert_eq!(carried.as_deref(), Some("octet-align=1"));
    }

    /// An unbridged session is not subject to the partner check.
    #[tokio::test]
    #[cfg(feature = "amr-nb")]
    async fn an_unbridged_session_may_change_its_framing_freely() {
        let controller = MediaSessionController::new();
        let solo = DialogId::new("solo");
        controller
            .start_media(solo.clone(), amr_config_with_fmtp("octet-align=1"))
            .await
            .unwrap();
        controller
            .update_media(solo.clone(), amr_config_with_fmtp("octet-align=0"))
            .await
            .expect("nothing is relaying, so nothing to protect");
    }

    /// AMR's rule, delegated to the parser that owns it.
    ///
    /// These were written against a local reimplementation of RFC 4867 §8.3.1
    /// and now run against `codec-core`'s. The expectations are unchanged,
    /// which is the point: the delegation must not alter the answer, only
    /// remove the second copy of the rule.
    #[test]
    #[cfg(feature = "amr-nb")]
    fn identical_and_default_amr_fmtp_are_interchangeable() {
        let same = |a, b| wire_formats_are_interchangeable("AMR", a, "AMR", b);
        assert!(same("", ""));
        assert!(same("octet-align=1", "octet-align=1"));
        // Absent and explicitly-default mean the same wire format.
        assert!(same("", "octet-align=0"));
        assert!(same("mode-set=0,1", ""));
    }

    #[test]
    #[cfg(feature = "amr-nb")]
    fn amr_framing_difference_is_not_interchangeable() {
        // The case this guard exists for. Both legs would carry the same
        // payload type, but bandwidth-efficient and octet-aligned are
        // different bit layouts, so relaying between them delivers payloads
        // the far end cannot parse.
        let same = |a, b| wire_formats_are_interchangeable("AMR", a, "AMR", b);
        assert!(!same("", "octet-align=1"));
        assert!(!same(
            "octet-align=0; mode-set=0,1",
            "octet-align=1; mode-set=0,1"
        ));
    }

    #[test]
    #[cfg(feature = "amr-nb")]
    fn amr_transport_options_that_change_the_bytes_are_compared() {
        for (a, b) in [
            ("octet-align=1", "octet-align=1; crc=1"),
            ("octet-align=1", "octet-align=1; robust-sorting=1"),
            ("octet-align=1", "octet-align=1; interleaving=2"),
        ] {
            assert!(
                !wire_formats_are_interchangeable("AMR", a, "AMR", b),
                "{a:?} vs {b:?} must not be interchangeable"
            );
        }
    }

    /// An fmtp that will not parse is interchangeable with nothing.
    ///
    /// Including an identical unparseable one: a bridge is not the place to
    /// decide what a malformed parameter meant. The local parser used to fall
    /// back to defaults here — `channels=banana` became `channels=1` and the
    /// relay went ahead.
    #[test]
    #[cfg(feature = "amr-nb")]
    fn an_unparseable_amr_fmtp_bridges_with_nothing() {
        for bad in ["channels=2", "channels=banana", "octet-align=maybe"] {
            assert!(
                !wire_formats_are_interchangeable("AMR", bad, "AMR", bad),
                "{bad:?} must not bridge, even with itself"
            );
            assert!(!wire_formats_are_interchangeable(
                "AMR",
                bad,
                "AMR",
                "octet-align=1"
            ));
        }
    }

    #[test]
    #[cfg(feature = "amr-nb")]
    fn amr_options_implying_octet_align_normalise_before_comparison() {
        // RFC 4867: crc, robust-sorting and interleaving each imply
        // octet-aligned operation, so stating it explicitly changes nothing.
        let same = |a, b| wire_formats_are_interchangeable("AMR", a, "AMR", b);
        assert!(same("crc=1", "octet-align=1; crc=1"));
        assert!(same("robust-sorting=1", "octet-align=1; robust-sorting=1"));
    }

    /// The two variants are different codecs and do not bridge to each other.
    #[test]
    #[cfg(all(feature = "amr-nb", feature = "amr-wb"))]
    fn the_two_amr_variants_do_not_bridge_to_each_other() {
        assert!(!wire_formats_are_interchangeable(
            "AMR",
            "octet-align=1",
            "AMR-WB",
            "octet-align=1"
        ));
    }

    /// A parameter that means something to AMR means nothing to PCMU.
    ///
    /// The guard used to key on parameter *names* regardless of codec, so a
    /// peer that put a nonstandard `channels=2` on a PCMU or Opus leg had its
    /// relay refused — a working configuration rejected over a field the codec
    /// does not have.
    #[test]
    fn non_amr_codecs_are_not_judged_by_amr_parameters() {
        for codec in ["PCMU", "PCMA", "opus"] {
            assert!(
                wire_formats_are_interchangeable(codec, "channels=2", codec, ""),
                "{codec} should not be judged by AMR's channels parameter"
            );
            assert!(wire_formats_are_interchangeable(
                codec,
                "octet-align=1",
                codec,
                "octet-align=0"
            ));
            assert!(wire_formats_are_interchangeable(
                codec,
                "maxaveragebitrate=24000",
                codec,
                "stereo=1"
            ));
        }
    }

    /// Different codecs never bridge, whatever their parameters say.
    #[test]
    fn different_codecs_do_not_bridge() {
        assert!(!wire_formats_are_interchangeable("PCMU", "", "PCMA", ""));
        assert!(!wire_formats_are_interchangeable("opus", "", "PCMU", ""));
        // Case is not a difference.
        assert!(wire_formats_are_interchangeable("PCMU", "", "pcmu", ""));
    }

    /// G.729's `annexb` changes the wire, so it is compared.
    ///
    /// It decides whether SID frames may appear at all, which is exactly the
    /// class of difference this guard exists for — and it was not among the
    /// five parameter names the old implementation matched.
    #[test]
    fn g729_annexb_is_compared() {
        assert!(!wire_formats_are_interchangeable(
            "G729",
            "annexb=no",
            "G729",
            ""
        ));
        assert!(!wire_formats_are_interchangeable(
            "G729",
            "annexb=no",
            "G729",
            "annexb=yes"
        ));
        // Absent means yes, per RFC 3555.
        assert!(wire_formats_are_interchangeable(
            "G729",
            "annexb=yes",
            "G729",
            ""
        ));
        assert!(wire_formats_are_interchangeable("G729", "", "G729", ""));
    }

    /// Real-world spacing, case and quoting, which the delegated parser owns.
    #[test]
    #[cfg(feature = "amr-nb")]
    fn amr_fmtp_comparison_tolerates_real_world_spacing_and_case() {
        let same = |a, b| wire_formats_are_interchangeable("AMR", a, "AMR", b);
        assert!(same(
            "OCTET-ALIGN=1;Mode-Set=0,1",
            "  octet-align = 1 ; mode-set=0,1  "
        ));
        // Quoted values appear in the field too.
        assert!(same("octet-align=\"1\"", "octet-align=1"));
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

    /// A genuinely-AMR session config, with the dynamic payload type and
    /// clock rate a real negotiation would have supplied.
    ///
    /// These tests used to build PCMU configs carrying AMR fmtp, because
    /// `start_media` refused an AMR one — `resolve_codec` had no AMR arm. Now
    /// that it does, the guard is exercised on the codec it is for, which
    /// matters since the guard is codec-keyed: an AMR parameter on a PCMU leg
    /// is correctly ignored, so the old configs would pass whatever the guard
    /// did.
    #[cfg(feature = "amr-nb")]
    fn amr_config_with_fmtp(fmtp: &str) -> MediaConfig {
        let mut config = test_config("AMR").with_negotiated_fmtp(Some(fmtp));
        config.parameters.insert(
            super::super::types::RTP_PAYLOAD_TYPE_PARAMETER.to_string(),
            "96".to_string(),
        );
        config.parameters.insert(
            super::super::types::RTP_CLOCK_RATE_PARAMETER.to_string(),
            "8000".to_string(),
        );
        config
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
    #[cfg(feature = "amr-nb")]
    async fn bridge_rejects_mismatched_amr_framing() {
        // End to end through bridge_sessions: same codec, same payload type,
        // different framing. Without the format check this bridges
        // "successfully" and then delivers unparseable audio — a silent
        // failure rather than an error.
        let controller = MediaSessionController::new();
        let a = DialogId::new("amr-a");
        let b = DialogId::new("amr-b");

        controller
            .start_media(a.clone(), amr_config_with_fmtp(""))
            .await
            .unwrap();
        controller
            .start_media(b.clone(), amr_config_with_fmtp("octet-align=1"))
            .await
            .unwrap();

        let err = expect_err(controller.bridge_sessions(a, b).await);
        assert!(
            matches!(err, BridgeError::FormatMismatch { .. }),
            "expected FormatMismatch, got {err:?}"
        );
    }

    #[tokio::test]
    #[cfg(feature = "amr-nb")]
    async fn bridge_accepts_matching_amr_framing() {
        // The guard must not reject legs that genuinely can relay: same
        // framing, differing only in parameters that do not change the layout.
        let controller = MediaSessionController::new();
        let a = DialogId::new("amr-ok-a");
        let b = DialogId::new("amr-ok-b");

        controller
            .start_media(
                a.clone(),
                amr_config_with_fmtp("octet-align=1; mode-set=0,1,2"),
            )
            .await
            .unwrap();
        controller
            .start_media(
                b.clone(),
                amr_config_with_fmtp("octet-align=1; mode-set=3,4"),
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
