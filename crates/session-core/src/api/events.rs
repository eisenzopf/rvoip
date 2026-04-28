//! Typed application events emitted by `session-core`.
//!
//! [`Event`] is the common event contract used by [`StreamPeer`], per-call
//! [`SessionHandle`](crate::SessionHandle) receivers, and direct
//! [`UnifiedCoordinator`](crate::UnifiedCoordinator) subscribers. Events are
//! translated from lower-level dialog/media notifications into
//! application-facing call, registration, transfer, NOTIFY, and media events.
//! Helper methods provide typed views over compatibility fields such as REFER
//! transfer kind and NOTIFY subscription state.
//!
//! [`StreamPeer`]: crate::StreamPeer

use crate::errors::Result;
use crate::state_table::types::SessionId;
use tokio::sync::mpsc;

/// Type alias for call ID (same as SessionId)
pub type CallId = SessionId;

/// Typed classification for REFER transfer requests.
///
/// The wire-facing `Event::ReferReceived::transfer_type` field remains a
/// string for compatibility. Use [`Event::transfer_kind`] when application
/// code wants a typed view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferKind {
    /// Standard blind transfer REFER.
    Blind,
    /// REFER carrying attended-transfer context such as `Replaces`.
    Attended,
    /// Unrecognized or vendor-specific transfer flavor.
    Unknown,
}

impl TransferKind {
    /// Convert the raw transfer type field into a typed classification.
    pub fn from_header_value(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "blind" => Self::Blind,
            "attended" => Self::Attended,
            _ => Self::Unknown,
        }
    }

    /// Stable lowercase label for logs and UI display.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blind => "blind",
            Self::Attended => "attended",
            Self::Unknown => "unknown",
        }
    }
}

/// Parsed view of a `Subscription-State` header.
///
/// This intentionally preserves the raw header value while extracting the
/// common `state`, `expires`, and `reason` parameters. Use
/// [`Event::subscription_state`] to parse a NOTIFY event on demand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionState {
    /// Primary state token, such as `active`, `pending`, or `terminated`.
    pub state: String,
    /// Parsed `expires` parameter, if present and numeric.
    pub expires: Option<u32>,
    /// Parsed `reason` parameter, if present.
    pub reason: Option<String>,
    /// Original header value.
    pub raw: String,
}

impl SubscriptionState {
    /// Parse a raw `Subscription-State` header value.
    pub fn parse(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        let mut parts = raw.split(';').map(str::trim);
        let state = parts.next().unwrap_or_default().to_string();
        let mut expires = None;
        let mut reason = None;

        for part in parts {
            if let Some(value) = part.strip_prefix("expires=") {
                expires = value.parse::<u32>().ok();
            } else if let Some(value) = part.strip_prefix("reason=") {
                reason = Some(value.to_string());
            }
        }

        Self {
            state,
            expires,
            reason,
            raw,
        }
    }
}

/// Handle for managing a specific call
///
/// Provides audio channels and call identification for a specific call session.
/// Each call gets its own handle with dedicated audio send/receive channels.
#[derive(Debug)]
pub struct CallHandle {
    /// The call ID for this handle
    call_id: CallId,
    /// Channel for sending audio to this call
    audio_tx: mpsc::Sender<Vec<i16>>,
    /// Channel for receiving audio from this call
    audio_rx: mpsc::Receiver<Vec<i16>>,
}

impl CallHandle {
    /// Create a new call handle
    pub fn new(call_id: CallId) -> (Self, mpsc::Receiver<Vec<i16>>, mpsc::Sender<Vec<i16>>) {
        let (audio_tx, audio_rx_for_handle) = mpsc::channel(100);
        let (audio_tx_for_coordinator, audio_rx) = mpsc::channel(100);

        let handle = Self {
            call_id,
            audio_tx,
            audio_rx,
        };

        (handle, audio_rx_for_handle, audio_tx_for_coordinator)
    }

    /// Get the call ID for this handle
    pub fn call_id(&self) -> &CallId {
        &self.call_id
    }

    /// Send audio samples to this call
    ///
    /// # Arguments
    /// * `samples` - PCM audio samples (16-bit, mono, 8kHz)
    ///
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::events::CallHandle;
    /// # async fn example(mut call_handle: CallHandle) -> rvoip_session_core::Result<()> {
    /// let samples = vec![100, 200, 300]; // Simple audio data
    /// call_handle.send_audio(samples).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_audio(&mut self, samples: Vec<i16>) -> Result<()> {
        self.audio_tx
            .send(samples)
            .await
            .map_err(|_| crate::errors::SessionError::Other("Audio channel closed".to_string()))?;
        Ok(())
    }

    /// Receive audio samples from this call (non-blocking)
    ///
    /// # Returns
    /// * `Some(samples)` - Audio data received from remote party
    /// * `None` - No audio available right now
    ///
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::events::CallHandle;
    /// # async fn example(mut call_handle: CallHandle) {
    /// if let Some(samples) = call_handle.recv_audio().await {
    ///     // Play or process the received audio
    ///     println!("Received {} audio samples", samples.len());
    /// }
    /// # }
    /// ```
    pub async fn recv_audio(&mut self) -> Option<Vec<i16>> {
        self.audio_rx.recv().await
    }

    /// Try to receive audio samples (non-blocking)
    ///
    /// # Returns
    /// * `Ok(samples)` - Audio data received
    /// * `Err(TryRecvError::Empty)` - No audio available
    /// * `Err(TryRecvError::Disconnected)` - Call ended
    pub fn try_recv_audio(&mut self) -> std::result::Result<Vec<i16>, mpsc::error::TryRecvError> {
        self.audio_rx.try_recv()
    }

    /// Check if the call handle is still connected
    pub fn is_connected(&self) -> bool {
        !self.audio_tx.is_closed() && !self.audio_rx.is_closed()
    }
}

/// Typed session events delivered to applications.
///
/// These events are published by the state machine and adapters when SIP,
/// media, registration, or transfer activity occurs. Use
/// [`Event::call_id`] to route per-call events, or one of the `is_*`
/// helpers to classify events in generic event loops.
#[derive(Debug, Clone)]
pub enum Event {
    // ===== Call Lifecycle Events =====
    /// Incoming call received
    ///
    /// The state machine has already sent 180 Ringing. Developer must
    /// call `accept()` or `reject()` to complete the call handling.
    IncomingCall {
        /// Session identifier assigned to this incoming INVITE.
        call_id: CallId,
        /// Caller URI from the SIP `From` header.
        from: String,
        /// Called URI from the SIP `To` or request URI context.
        to: String,
        /// Remote SDP offer, if the INVITE contained one.
        sdp: Option<String>,
    },

    /// Call was answered (200 OK received for outgoing call)
    CallAnswered {
        /// Session identifier for the answered call.
        call_id: CallId,
        /// SDP answer received from the remote peer, if present.
        sdp: Option<String>,
    },

    /// Call ended (BYE sent/received)
    CallEnded {
        /// Session identifier for the ended call.
        call_id: CallId,
        /// Human-readable teardown reason.
        reason: String,
    },

    /// Call failed (4xx/5xx response or timeout)
    CallFailed {
        /// Session identifier for the failed call.
        call_id: CallId,
        /// SIP status code or synthesized failure code.
        status_code: u16,
        /// Human-readable failure reason.
        reason: String,
    },

    /// Caller cancelled before the call was answered (RFC 3261 §15.1.2 —
    /// 487 Request Terminated following CANCEL). Distinct from `CallFailed`
    /// so UIs can render "missed call" rather than "call rejected".
    CallCancelled {
        /// Session identifier for the cancelled incoming call.
        call_id: CallId,
    },

    /// RFC 4028 session timer refresh succeeded (UPDATE or re-INVITE
    /// round-tripped). Emitted once per successful refresh — applications
    /// can use this to reset connection-health dashboards or log activity.
    SessionRefreshed {
        /// Session identifier for the refreshed dialog.
        call_id: CallId,
        /// Negotiated session expiration interval in seconds.
        expires_secs: u32,
    },

    /// RFC 4028 session-timer refresh failed; the dialog has been torn
    /// down with BYE (§10). Follow-up `CallEnded` will still fire.
    SessionRefreshFailed {
        /// Session identifier for the dialog whose refresh failed.
        call_id: CallId,
        /// Human-readable refresh failure reason.
        reason: String,
    },

    /// RFC 3261 §22.2 — server challenged our INVITE with 401/407 and we're
    /// about to retry with a digest authorization header. Informational; no
    /// action required from the app. If the retry fails (wrong credentials
    /// or retry cap exceeded), `CallFailed` follows.
    CallAuthRetrying {
        /// Session identifier for the challenged outgoing call.
        call_id: CallId,
        /// 401 or 407.
        status_code: u16,
        /// Digest realm the server asked us to authenticate against.
        realm: String,
    },

    // ===== Transfer Events =====
    /// REFER request received
    ///
    /// Callback handlers may accept or reject the REFER through their return
    /// value. Stream/unified users can call `accept_refer` or `reject_refer`;
    /// if they do nothing, session-core preserves the legacy behavior and
    /// accepts the REFER after a short grace period.
    ReferReceived {
        /// Session identifier for the dialog that received REFER.
        call_id: CallId,
        /// Raw `Refer-To` target URI.
        refer_to: String,
        /// Optional `Referred-By` header value.
        referred_by: Option<String>,
        /// Optional `Replaces` parameter/header value for attended transfer.
        replaces: Option<String>,
        /// Dialog-core transaction ID used to correlate REFER response/NOTIFY.
        transaction_id: String, // For NOTIFY correlation
        /// Raw transfer flavor. Prefer [`Event::transfer_kind`] for typed
        /// classification.
        transfer_type: String, // "blind" or "attended"
    },

    /// Transfer accepted by recipient
    TransferAccepted {
        /// Session identifier for the call whose REFER was accepted.
        call_id: CallId,
        /// Target URI from the accepted REFER.
        refer_to: String,
    },

    /// Transfer completed successfully
    TransferCompleted {
        /// Original session being transferred.
        old_call_id: CallId,
        /// New session established as part of transfer handling, when known.
        new_call_id: CallId,
        /// Transfer target URI.
        target: String,
    },

    /// Transfer failed
    TransferFailed {
        /// Session identifier for the failed transfer.
        call_id: CallId,
        /// Human-readable failure reason.
        reason: String,
        /// SIP status code reported by REFER/NOTIFY handling.
        status_code: u16,
    },

    /// Transfer progress update (for transferor monitoring)
    TransferProgress {
        /// Session identifier for the REFER subscription/dialog.
        call_id: CallId,
        /// SIP status code from the progress NOTIFY sipfrag.
        status_code: u16,
        /// Reason phrase from the progress NOTIFY sipfrag.
        reason: String,
    },

    // ===== Subscription / NOTIFY =====
    /// Inbound NOTIFY surfaced to the application (RFC 6665).
    ///
    /// Fires for every NOTIFY received on any event package — REFER
    /// progress, dialog, presence, message-summary, etc. The session
    /// layer does not interpret the body; if `event_package == "refer"`
    /// and `content_type` is `message/sipfrag`, `TransferProgress` /
    /// `TransferCompleted` / `TransferFailed` are also emitted with the
    /// parsed status line.
    NotifyReceived {
        /// Session identifier for the dialog that received NOTIFY.
        call_id: CallId,
        /// SIP `Event` package name.
        event_package: String,
        /// Raw `Subscription-State:` header value (unparsed).
        subscription_state: Option<String>,
        /// Raw `Content-Type:` header value.
        content_type: Option<String>,
        /// NOTIFY body, if any.
        body: Option<String>,
    },

    // ===== Call State Events =====
    /// Local hold was accepted by the remote peer.
    ///
    /// Emitted after the hold re-INVITE/answer exchange succeeds.
    CallOnHold {
        /// Session identifier for the held call.
        call_id: CallId,
    },

    /// Local resume was accepted by the remote peer.
    ///
    /// Emitted after the resume re-INVITE/answer exchange succeeds.
    CallResumed {
        /// Session identifier for the resumed call.
        call_id: CallId,
    },

    /// The remote peer placed this call on hold with a mid-call offer.
    RemoteCallOnHold {
        /// Session identifier for the remotely held call.
        call_id: CallId,
    },

    /// The remote peer resumed this call with a mid-call offer.
    RemoteCallResumed {
        /// Session identifier for the remotely resumed call.
        call_id: CallId,
    },

    /// Call was muted locally
    CallMuted {
        /// Session identifier for the muted call.
        call_id: CallId,
    },

    /// Call was unmuted locally
    CallUnmuted {
        /// Session identifier for the unmuted call.
        call_id: CallId,
    },

    // ===== Media Events =====
    /// DTMF digit received
    DtmfReceived {
        /// Session identifier for the call that received DTMF.
        call_id: CallId,
        /// Received digit.
        digit: char,
    },

    /// Media quality changed
    MediaQualityChanged {
        /// Session identifier for the media stream.
        call_id: CallId,
        /// Packet loss percentage, rounded to an integer.
        packet_loss_percent: u32,
        /// Jitter in milliseconds, rounded to an integer.
        jitter_ms: u32,
    },

    // ===== Registration Events =====
    /// Registration successful.
    ///
    /// `expires` is the registrar-accepted expiry, not necessarily the value
    /// requested by the application. Use
    /// [`UnifiedCoordinator::registration_info`](crate::UnifiedCoordinator::registration_info)
    /// for refresh timing, Service-Route, GRUU, and failure metadata.
    RegistrationSuccess {
        /// Registrar URI used for the REGISTER.
        registrar: String,
        /// Expiration interval accepted by the registrar.
        expires: u32,
        /// Contact URI that was registered.
        contact: String,
    },

    /// Registration failed.
    ///
    /// Final failure after any supported retry path, such as digest auth retry
    /// or 423 Interval Too Brief retry.
    RegistrationFailed {
        /// Registrar URI used for the failed REGISTER.
        registrar: String,
        /// SIP status code returned by the registrar.
        status_code: u16,
        /// Human-readable failure reason.
        reason: String,
    },

    /// Unregistration successful.
    ///
    /// Automatic refresh for the registration has been aborted.
    UnregistrationSuccess {
        /// Registrar URI used for the unregistration.
        registrar: String,
    },

    /// Unregistration failed.
    UnregistrationFailed {
        /// Registrar URI used for the failed unregistration.
        registrar: String,
        /// Human-readable failure reason.
        reason: String,
    },

    // ===== Error Events =====
    /// Network error occurred
    NetworkError {
        /// Session identifier, if the transport error can be tied to one call.
        call_id: Option<CallId>,
        /// Human-readable error text.
        error: String,
    },

    /// Authentication required (401/407 response)
    AuthenticationRequired {
        /// Session identifier for the challenged request.
        call_id: CallId,
        /// Digest-auth realm from the challenge.
        realm: String,
    },
}

impl Event {
    /// Get the call ID associated with this event (if any)
    pub fn call_id(&self) -> Option<&CallId> {
        match self {
            Event::IncomingCall { call_id, .. }
            | Event::CallAnswered { call_id, .. }
            | Event::CallEnded { call_id, .. }
            | Event::CallFailed { call_id, .. }
            | Event::CallCancelled { call_id, .. }
            | Event::SessionRefreshed { call_id, .. }
            | Event::SessionRefreshFailed { call_id, .. }
            | Event::CallAuthRetrying { call_id, .. }
            | Event::ReferReceived { call_id, .. }
            | Event::TransferAccepted { call_id, .. }
            | Event::TransferFailed { call_id, .. }
            | Event::TransferProgress { call_id, .. }
            | Event::CallOnHold { call_id, .. }
            | Event::CallResumed { call_id, .. }
            | Event::RemoteCallOnHold { call_id, .. }
            | Event::RemoteCallResumed { call_id, .. }
            | Event::CallMuted { call_id, .. }
            | Event::CallUnmuted { call_id, .. }
            | Event::DtmfReceived { call_id, .. }
            | Event::MediaQualityChanged { call_id, .. }
            | Event::NotifyReceived { call_id, .. }
            | Event::AuthenticationRequired { call_id, .. } => Some(call_id),
            Event::TransferCompleted { old_call_id, .. } => Some(old_call_id),
            Event::NetworkError { call_id, .. } => call_id.as_ref(),
            // Registration events don't have call_id
            Event::RegistrationSuccess { .. }
            | Event::RegistrationFailed { .. }
            | Event::UnregistrationSuccess { .. }
            | Event::UnregistrationFailed { .. } => None,
        }
    }

    /// Check if this is a call lifecycle event
    pub fn is_call_event(&self) -> bool {
        matches!(
            self,
            Event::IncomingCall { .. }
                | Event::CallAnswered { .. }
                | Event::CallEnded { .. }
                | Event::CallFailed { .. }
                | Event::CallCancelled { .. }
        )
    }

    /// Check if this is a call state/control event
    pub fn is_call_state_event(&self) -> bool {
        matches!(
            self,
            Event::CallOnHold { .. }
                | Event::CallResumed { .. }
                | Event::RemoteCallOnHold { .. }
                | Event::RemoteCallResumed { .. }
                | Event::CallMuted { .. }
                | Event::CallUnmuted { .. }
        )
    }

    /// Check if this is a transfer-related event
    pub fn is_transfer_event(&self) -> bool {
        matches!(
            self,
            Event::ReferReceived { .. }
                | Event::TransferAccepted { .. }
                | Event::TransferCompleted { .. }
                | Event::TransferFailed { .. }
                | Event::TransferProgress { .. }
        )
    }

    /// Check if this is a media-related event
    pub fn is_media_event(&self) -> bool {
        matches!(
            self,
            Event::DtmfReceived { .. } | Event::MediaQualityChanged { .. }
        )
    }

    /// Typed transfer kind for `ReferReceived`.
    ///
    /// Returns `None` for non-REFER events.
    pub fn transfer_kind(&self) -> Option<TransferKind> {
        match self {
            Event::ReferReceived { transfer_type, .. } => {
                Some(TransferKind::from_header_value(transfer_type))
            }
            _ => None,
        }
    }

    /// Parsed `Subscription-State` for `NotifyReceived`.
    ///
    /// Returns `None` when the event is not NOTIFY or the header was absent.
    pub fn subscription_state(&self) -> Option<SubscriptionState> {
        match self {
            Event::NotifyReceived {
                subscription_state: Some(raw),
                ..
            } => Some(SubscriptionState::parse(raw.clone())),
            _ => None,
        }
    }
}
