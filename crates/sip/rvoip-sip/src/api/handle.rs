//! Per-call control API shared by all peer surfaces.
//!
//! [`SessionHandle`] is returned by high-level peer APIs when a call is
//! created or accepted. It is a cheap, cloneable handle to the underlying
//! session and exposes the operations application developers usually need
//! once a call exists: hangup, hold/resume, DTMF, transfer, INFO/NOTIFY,
//! event subscription, state inspection, and audio frames.

#![deny(missing_docs)]

use rvoip_media_core::types::AudioFrame;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::api::audio::{AudioReceiver, AudioSender, AudioStream};
use crate::api::dialog_package::{DialogInfo, DialogPackageState};
use crate::api::events::{Event, MediaSecurityState, TransferTargetEvidence};
use crate::api::lifecycle::{CallLifecycleSnapshot, CallTerminalInfo};
use crate::api::unified::UnifiedCoordinator;
use crate::errors::{Result, SessionError};
use crate::state_table::types::SessionId;
use crate::types::{CallState, SessionInfo};

const AUDIO_STREAM_CHANNEL_FRAMES: usize = 128;

/// Type alias so callers can refer to a session by `CallId`.
pub type CallId = SessionId;

/// Evidence level required by blind-transfer wait helpers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferWaitMode {
    /// Return when a terminal REFER NOTIFY is received, regardless of whether
    /// target-leg progress was observed first.
    NotifyFinal,
    /// Return only after the REFER subscription reports a provisional target
    /// status such as `180 Ringing` or `183 Session Progress`.
    TargetRinging,
    /// Return only after a terminal successful REFER NOTIFY that was preceded
    /// by target progress evidence.
    TargetAnswered,
    /// Return only after a replacement dialog is observed terminated via a
    /// local target leg or RFC 4235 dialog-package evidence.
    ReplacementTerminated,
}

/// Typed result returned by high-level blind-transfer wait helpers.
///
/// This is the ergonomic application-facing view of REFER lifecycle events.
/// `ReferCompleted` means the REFER subscription reported a final successful
/// referenced request; it does not by itself prove replacement-call lifecycle.
#[derive(Clone, PartialEq, Eq)]
pub enum TransferOutcome {
    /// A final successful REFER NOTIFY was received.
    ReferCompleted {
        /// Transfer call/session id.
        call_id: CallId,
        /// Transfer target URI reported by rvoip-sip.
        target: String,
        /// Final sipfrag status code.
        status_code: u16,
        /// Final sipfrag reason phrase.
        reason: String,
    },
    /// The transfer target produced provisional ringing or early-media evidence.
    TargetRinging {
        /// Transfer call/session id.
        call_id: CallId,
        /// Provisional sipfrag status code.
        status_code: u16,
        /// Provisional sipfrag reason phrase.
        reason: String,
    },
    /// The transfer target answered with trustworthy evidence.
    TargetAnswered {
        /// Transfer call/session id.
        call_id: CallId,
        /// Transfer target URI.
        target_uri: String,
        /// Evidence used to classify the target as answered.
        evidence: TransferTargetEvidence,
    },
    /// A related replacement dialog was observed terminated.
    ReplacementTerminated {
        /// Transfer call/session id.
        call_id: CallId,
        /// Dialog-package entry for the replacement dialog.
        dialog: DialogInfo,
        /// Optional teardown reason.
        reason: Option<String>,
    },
    /// REFER or transfer processing failed.
    Failed {
        /// Transfer call/session id.
        call_id: CallId,
        /// SIP status code reported by REFER/NOTIFY handling.
        status_code: u16,
        /// Human-readable failure reason.
        reason: String,
    },
}

impl fmt::Debug for TransferOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReferCompleted {
                status_code,
                reason,
                target,
                ..
            } => formatter
                .debug_struct("ReferCompleted")
                .field("target_bytes", &target.len())
                .field("status_code", status_code)
                .field("reason_bytes", &reason.len())
                .finish(),
            Self::TargetRinging {
                status_code,
                reason,
                ..
            } => formatter
                .debug_struct("TargetRinging")
                .field("status_code", status_code)
                .field("reason_bytes", &reason.len())
                .finish(),
            Self::TargetAnswered {
                target_uri,
                evidence,
                ..
            } => formatter
                .debug_struct("TargetAnswered")
                .field("target_uri_bytes", &target_uri.len())
                .field("evidence", evidence)
                .finish(),
            Self::ReplacementTerminated { dialog, reason, .. } => formatter
                .debug_struct("ReplacementTerminated")
                .field("dialog", dialog)
                .field("reason_present", &reason.is_some())
                .field("reason_bytes", &reason.as_ref().map_or(0, String::len))
                .finish(),
            Self::Failed {
                status_code,
                reason,
                ..
            } => formatter
                .debug_struct("Failed")
                .field("status_code", status_code)
                .field("reason_bytes", &reason.len())
                .finish(),
        }
    }
}

impl TryFrom<Event> for TransferOutcome {
    type Error = SessionError;

    fn try_from(event: Event) -> std::result::Result<Self, Self::Error> {
        match event {
            Event::ReferCompleted {
                call_id,
                target,
                status_code,
                reason,
            } => Ok(Self::ReferCompleted {
                call_id,
                target,
                status_code,
                reason,
            }),
            Event::ReferProgress {
                call_id,
                status_code,
                reason,
            } => Ok(Self::TargetRinging {
                call_id,
                status_code,
                reason,
            }),
            Event::TransferTargetAnswered {
                transfer_call_id,
                target_uri,
                evidence,
            } => Ok(Self::TargetAnswered {
                call_id: transfer_call_id,
                target_uri,
                evidence,
            }),
            Event::TransferReplacementDialogTerminated {
                transfer_call_id,
                dialog,
                reason,
            } => Ok(Self::ReplacementTerminated {
                call_id: transfer_call_id,
                dialog,
                reason,
            }),
            Event::TransferFailed {
                call_id,
                status_code,
                reason,
            } => Ok(Self::Failed {
                call_id,
                status_code,
                reason,
            }),
            other => Err(SessionError::Other(format!(
                "event is not a transfer outcome: {:?}",
                other
            ))),
        }
    }
}

/// RFC 3326 Reason header value for explicit teardown causes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SipReason {
    /// Reason protocol, usually `SIP` or `Q.850`.
    pub protocol: String,
    /// Numeric cause code.
    pub cause: u16,
    /// Optional human-readable cause text.
    pub text: Option<String>,
}

impl SipReason {
    /// Build a SIP-protocol Reason value.
    pub fn sip(cause: u16, text: impl Into<String>) -> Self {
        Self {
            protocol: "SIP".to_string(),
            cause,
            text: Some(text.into()),
        }
    }
}

/// How to correlate RFC 4235 dialog entries with a transfer target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferDialogMatcher {
    /// Match any observed dialog in the supplied subscription.
    Any,
    /// Match when the target URI appears in the dialog's local or remote URI.
    TargetUri,
}

/// Optional evidence sources for transfer lifecycle waits.
#[derive(Clone)]
pub struct TransferLifecycleOptions {
    /// Optional RFC 4235 dialog subscription to observe third-party replacement legs.
    pub dialog_subscription: Option<crate::api::dialog_subscription::DialogSubscriptionHandle>,
    /// Dialog matching policy for RFC 4235 events.
    pub target_match: TransferDialogMatcher,
    /// Whether the wait requires replacement-dialog termination.
    pub wait_for_replacement_termination: bool,
}

impl Default for TransferLifecycleOptions {
    fn default() -> Self {
        Self {
            dialog_subscription: None,
            target_match: TransferDialogMatcher::TargetUri,
            wait_for_replacement_termination: false,
        }
    }
}

/// Handle for controlling an active SIP call session.
///
/// Returned by `peer.invite(...).send()`, then resolved via
/// [`coordinator().session(call_id)`](crate::api::unified::UnifiedCoordinator::session),
/// and by [`IncomingCall::accept`](crate::api::incoming::IncomingCall::accept)
/// and similar methods.
///
/// `SessionHandle` is cheap to clone — all clones control the same underlying session.
/// It is `Send + Sync` and safe to share across tasks.
///
/// Most methods are thin call-control operations; the event stream is available
/// through [`events`](Self::events) when the caller needs to observe the result
/// of asynchronous SIP behavior such as remote hangup, REFER progress, DTMF,
/// or hold/resume completion. Deterministic helpers such as
/// [`hangup_and_wait`](Self::hangup_and_wait) and
/// [`transfer_blind_and_wait_for_outcome`](Self::transfer_blind_and_wait_for_outcome)
/// subscribe before sending the command so tests and servers can wait for typed
/// terminal evidence.
/// Wait helpers observe typed events and lifecycle state; their optional
/// timeout only bounds the wait. Timeouts never send SIP messages, change call
/// state, or suppress later events. Use command methods such as
/// [`hangup`](Self::hangup), [`hangup_and_wait`](Self::hangup_and_wait), and
/// [`transfer_blind`](Self::transfer_blind) when the application chooses to
/// mutate call lifecycle.
///
/// # Example
///
/// ```rust,no_run
/// # async fn example(handle: rvoip_sip::SessionHandle) -> anyhow::Result<()> {
/// // Put call on hold
/// handle.hold().await?;
/// tokio::time::sleep(std::time::Duration::from_secs(5)).await;
/// handle.resume().await?;
///
/// // Get audio stream
/// let audio = handle.audio().await?;
///
/// // Hang up and wait for the terminal call event.
/// handle.hangup_and_wait(None).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct SessionHandle {
    pub(crate) call_id: CallId,
    pub(crate) coordinator: Arc<UnifiedCoordinator>,
}

impl SessionHandle {
    pub(crate) fn new(call_id: CallId, coordinator: Arc<UnifiedCoordinator>) -> Self {
        Self {
            call_id,
            coordinator,
        }
    }

    /// The unique identifier for this call session.
    pub fn id(&self) -> &CallId {
        &self.call_id
    }

    // ===== In-dialog request builders =====
    //
    // Canonical entry points for in-dialog SIP requests on this session.
    // Each method returns the builder for the corresponding method,
    // pre-bound to this session's `CallId`, so callers can stage extra
    // headers via the `SipRequestOptions` trait and dispatch with
    // `.send().await`. See `SIP_API_DESIGN_2.md` §3.3.

    /// Begin building an outbound BYE for this session.
    ///
    /// Lower-level than [`hangup`](Self::hangup): returns a builder so
    /// the caller can stage `Reason`, custom `X-*` headers, etc. before
    /// sending. Prefer `hangup()` for fire-and-forget teardown.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// use rvoip_sip::SipReason;
    /// call.bye()
    ///     .with_sip_reason(SipReason::sip(200, "Normal call clearing"))
    ///     .send()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn bye(&self) -> crate::api::send::ByeBuilder {
        self.coordinator.bye(&self.call_id)
    }

    /// Begin building an outbound CANCEL for this session.
    ///
    /// CANCEL applies to an early INVITE that has received a provisional
    /// response. RFC 3261 §9 carries `Call-ID`, `From`, `To` (without
    /// tag), `CSeq`, and `Route` from the INVITE automatically — the
    /// builder lets you attach `Reason` or custom headers in addition.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// call.cancel().send().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn cancel(&self) -> crate::api::send::CancelBuilder {
        self.coordinator.cancel(&self.call_id)
    }

    /// Begin building an outbound REFER for this session.
    ///
    /// `refer_to` is the target URI (Refer-To header value). Use
    /// `with_replaces` for attended transfer or `with_referred_by` for
    /// RFC 3892 attribution.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// call.refer("sip:bob@example.com").send().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn refer(&self, refer_to: impl Into<String>) -> crate::api::send::ReferBuilder {
        self.coordinator.refer(&self.call_id, refer_to)
    }

    /// Begin building an outbound NOTIFY for this session.
    ///
    /// `event_package` populates the required RFC 6665 `Event:` header
    /// (e.g. `"dialog"`, `"message-summary"`, `"refer"`). Attach the
    /// `Subscription-State`, body, and any other headers via the builder.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// call.notify("message-summary")
    ///     .with_subscription_state("active;expires=3600")
    ///     .send()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn notify(&self, event_package: impl Into<String>) -> crate::api::send::NotifyBuilder {
        self.coordinator.notify(&self.call_id, event_package)
    }

    /// Begin building an outbound INFO for this session.
    ///
    /// `content_type` becomes the `Content-Type` header value (e.g.
    /// `"application/dtmf-relay"`). Attach the body via
    /// [`InfoBuilder::with_body`](crate::api::send::InfoBuilder::with_body).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// call.info("application/dtmf-relay")
    ///     .with_body(bytes::Bytes::from_static(b"Signal=1\r\nDuration=100\r\n"))
    ///     .send()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn info(&self, content_type: impl Into<String>) -> crate::api::send::InfoBuilder {
        self.coordinator.info(&self.call_id, content_type)
    }

    /// Begin building an outbound UPDATE (RFC 3311) for this session.
    ///
    /// UPDATE renegotiates session parameters without a full re-INVITE
    /// dance. Common uses: session-timer refresh (via
    /// `as_session_timer_refresh`) and early-dialog SDP updates.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// call.update().as_session_timer_refresh().send().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn update(&self) -> crate::api::send::UpdateBuilder {
        self.coordinator.update(&self.call_id)
    }

    /// Begin building an outbound re-INVITE for this session.
    ///
    /// re-INVITE is the established-dialog renegotiation path (hold/
    /// resume, SDP swap, session-timer refresh). For hold/resume use
    /// the higher-level [`hold`](Self::hold)/[`resume`](Self::resume)
    /// helpers instead; reach for `reinvite()` when finer control over
    /// the SDP or extra headers is needed.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle, new_sdp: String) -> rvoip_sip::Result<()> {
    /// call.reinvite().with_sdp(new_sdp).send().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn reinvite(&self) -> crate::api::send::ReInviteBuilder {
        self.coordinator.reinvite(&self.call_id)
    }

    // ===== Call control =====

    /// Hang up the call.
    ///
    /// Fire-and-forget: hands the teardown request to rvoip-sip and returns
    /// after the command is accepted. Established calls send BYE. Ringing or
    /// early-media calls send CANCEL and wait internally for the final INVITE
    /// outcome. If the outbound INVITE has not received a provisional response
    /// yet, rvoip-sip records cancel intent and sends CANCEL only if/when
    /// RFC 3261 makes it legal; a fast 200 OK on that path is ACKed and then
    /// immediately BYE-cleaned.
    ///
    /// Subscribe to events or use [`hangup_and_wait`](Self::hangup_and_wait)
    /// when the caller needs to observe `CallEnded`, `CallFailed`, or
    /// `CallCancelled`. `CallCancelled` means SIP cancellation teardown is
    /// terminal, not merely that the user requested cancel.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// call.hangup().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn hangup(&self) -> Result<()> {
        self.coordinator.hangup(&self.call_id).await
    }

    /// Hang up the call and wait for the terminal event.
    ///
    /// Unlike [`hangup`](Self::hangup), this subscribes to the call's event
    /// stream before sending BYE/CANCEL and returns only after `CallEnded`,
    /// `CallFailed`, or `CallCancelled` is observed. For outbound pre-answer
    /// calls, this follows SIP timing: no CANCEL before provisional response,
    /// CANCEL waits for the final INVITE outcome, and a late 200 OK after
    /// cancel is ACKed then BYE-cleaned before `CallCancelled` resolves.
    ///
    /// The timeout bounds observation of the terminal event; it does not roll
    /// back or otherwise alter the SIP teardown already requested.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// let reason = call
    ///     .hangup_and_wait(Some(std::time::Duration::from_secs(3)))
    ///     .await?;
    /// println!("call ended: {reason}");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn hangup_and_wait(&self, timeout: Option<Duration>) -> Result<String> {
        let rx = self.coordinator.lifecycle_watcher(&self.call_id);
        if let Some(reason) = terminal_reason(&self.lifecycle().await?) {
            return Ok(reason);
        }
        match self.coordinator.hangup(&self.call_id).await {
            Ok(()) => {}
            Err(e) if e.is_session_gone() => {
                if let Some(reason) = terminal_reason(&self.lifecycle().await?) {
                    return Ok(reason);
                }
                return Err(e);
            }
            Err(e) => return Err(e),
        }

        let result =
            wait_for_lifecycle(self, rx, timeout, "hangup_and_wait timed out", |snapshot| {
                Ok(terminal_reason(snapshot))
            })
            .await;
        if matches!(result, Err(SessionError::Timeout(_))) {
            self.spawn_late_answer_teardown_observer();
        }
        result
    }

    /// Hang up the call with an RFC 3326 `Reason` header and wait for the
    /// terminal event.
    ///
    /// Internally `call.bye().with_sip_reason(reason).send()` plus event
    /// observation — when you don't need the wait, reach for
    /// [`bye`](Self::bye) directly:
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// use rvoip_sip::SipReason;
    /// call.bye()
    ///     .with_sip_reason(SipReason::sip(200, "Normal call clearing"))
    ///     .send().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn hangup_with_reason(
        &self,
        reason: SipReason,
        timeout: Option<Duration>,
    ) -> Result<String> {
        let rx = self.coordinator.lifecycle_watcher(&self.call_id);
        if let Some(cached) = terminal_reason(&self.lifecycle().await?) {
            return Ok(cached);
        }
        self.bye().with_sip_reason(reason).send().await?;

        wait_for_lifecycle(
            self,
            rx,
            timeout,
            "hangup_with_reason timed out",
            |snapshot| Ok(terminal_reason(snapshot)),
        )
        .await
    }

    /// Put the call on hold with a target-refresh re-INVITE.
    ///
    /// On success, applications observe [`Event::CallOnHold`] through the
    /// peer/coordinator event stream.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// call.hold().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn hold(&self) -> Result<()> {
        self.coordinator.hold(&self.call_id).await
    }

    /// Resume a held call with a target-refresh re-INVITE.
    ///
    /// On success, applications observe [`Event::CallResumed`].
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// call.resume().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn resume(&self) -> Result<()> {
        self.coordinator.resume(&self.call_id).await
    }

    /// Mute local audio.
    ///
    /// This is a local media-state transition; it does not place the SIP dialog
    /// on hold. Use [`hold`](Self::hold) when the remote peer must be signalled.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// call.mute().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn mute(&self) -> Result<()> {
        use crate::state_table::types::EventType;
        self.coordinator
            .helpers
            .state_machine
            .process_event(&self.call_id, EventType::MuteCall)
            .await?;
        Ok(())
    }

    /// Unmute local audio.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// call.unmute().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn unmute(&self) -> Result<()> {
        use crate::state_table::types::EventType;
        self.coordinator
            .helpers
            .state_machine
            .process_event(&self.call_id, EventType::UnmuteCall)
            .await?;
        Ok(())
    }

    // ===== Transfer =====

    /// Initiate a blind transfer by sending REFER to the remote party.
    ///
    /// The remote party is expected to call the `target` URI and then send
    /// REFER progress NOTIFYs. Use
    /// [`transfer_blind_and_wait_for_outcome`](Self::transfer_blind_and_wait_for_outcome)
    /// when the caller needs to wait for typed transfer success/failure.
    ///
    /// Equivalent to `call.refer(target).send().await` — reach for that
    /// shape directly via [`refer`](Self::refer) when you need to stage
    /// `Referred-By`, `Replaces`, or custom headers on the REFER.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// call.transfer_blind("sip:charlie@example.com").await?;
    /// // Equivalent — and gives access to with_referred_by / extras:
    /// // call.refer("sip:charlie@example.com").send().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn transfer_blind(&self, target: &str) -> Result<()> {
        self.refer(target).send().await
    }

    /// Initiate a blind transfer and wait for a raw terminal transfer event.
    ///
    /// Prefer
    /// [`transfer_blind_and_wait_for_outcome`](Self::transfer_blind_and_wait_for_outcome)
    /// for application code. This lower-level helper returns the raw
    /// [`Event`] for callers that need debugging detail or custom event
    /// handling.
    ///
    /// Returns `Event::ReferCompleted` on a final 2xx sipfrag or
    /// `Event::TransferFailed` on a final failure sipfrag. `ReferCompleted`
    /// means the REFER
    /// subscription reached a terminal success state; it does not prove the
    /// transfer target answered. Use
    /// [`transfer_blind_and_wait_for_outcome`](Self::transfer_blind_and_wait_for_outcome)
    /// with [`TransferWaitMode::TargetAnswered`] when target evidence matters.
    /// Intermediate progress events are consumed while waiting, so create a
    /// separate event receiver if another task also needs to observe them.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// let terminal = call
    ///     .transfer_blind_and_wait("sip:charlie@example.com", Some(std::time::Duration::from_secs(10)))
    ///     .await?;
    /// # let _ = terminal;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn transfer_blind_and_wait(
        &self,
        target: &str,
        timeout: Option<Duration>,
    ) -> Result<Event> {
        self.transfer_blind_and_wait_for(target, TransferWaitMode::NotifyFinal, timeout)
            .await
    }

    /// Initiate a blind transfer and wait for a specific transfer evidence mode.
    ///
    /// `NotifyFinal` matches terminal REFER NOTIFY semantics. `TargetRinging`
    /// requires a provisional target sipfrag. `TargetAnswered` requires a
    /// terminal successful sipfrag that was preceded by target progress
    /// evidence, preventing an immediate PBX `200 OK` NOTIFY from being
    /// interpreted as a proven target answer.
    ///
    /// The timeout only cancels this wait; it does not undo or terminate the
    /// REFER transaction.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use rvoip_sip::TransferWaitMode;
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// let event = call
    ///     .transfer_blind_and_wait_for(
    ///         "sip:charlie@example.com",
    ///         TransferWaitMode::TargetAnswered,
    ///         Some(std::time::Duration::from_secs(15)),
    ///     )
    ///     .await?;
    /// # let _ = event;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn transfer_blind_and_wait_for(
        &self,
        target: &str,
        mode: TransferWaitMode,
        timeout: Option<Duration>,
    ) -> Result<Event> {
        let mut events = self.events().await?;
        self.transfer_blind(target).await?;

        let fut = async {
            let mut target_progress_seen = false;
            loop {
                match events.next().await {
                    Some(event @ Event::ReferProgress { status_code, .. })
                        if (180..=199).contains(&status_code) =>
                    {
                        target_progress_seen = true;
                        if mode == TransferWaitMode::TargetRinging {
                            return Ok(event);
                        }
                    }
                    Some(event @ Event::ReferCompleted { .. }) => match mode {
                        TransferWaitMode::NotifyFinal => return Ok(event),
                        TransferWaitMode::TargetRinging => {
                            return Err(SessionError::Other(
                                "terminal REFER NOTIFY arrived before target ringing evidence"
                                    .to_string(),
                            ))
                        }
                        TransferWaitMode::TargetAnswered if target_progress_seen => {
                            return Ok(event)
                        }
                        TransferWaitMode::TargetAnswered => {
                            return Err(SessionError::Other(
                                "terminal REFER NOTIFY arrived before target-answer evidence"
                                    .to_string(),
                            ))
                        }
                        TransferWaitMode::ReplacementTerminated => {
                            return Err(SessionError::Other(
                                "ReplacementTerminated requires transfer lifecycle options"
                                    .to_string(),
                            ))
                        }
                    },
                    Some(event @ Event::TransferTargetAnswered { .. })
                        if mode == TransferWaitMode::TargetAnswered =>
                    {
                        return Ok(event)
                    }
                    Some(event @ Event::TransferReplacementDialogTerminated { .. })
                        if mode == TransferWaitMode::ReplacementTerminated =>
                    {
                        return Ok(event)
                    }
                    Some(event @ Event::TransferFailed { .. }) => return Ok(event),
                    Some(_) => {}
                    None => {
                        return Err(SessionError::Other(
                            "Event channel closed while waiting for transfer".to_string(),
                        ))
                    }
                }
            }
        };

        match timeout {
            Some(duration) => tokio::time::timeout(duration, fut).await.map_err(|_| {
                SessionError::Timeout("transfer_blind_and_wait timed out".to_string())
            })?,
            None => fut.await,
        }
    }

    /// Initiate a blind transfer and wait for a typed transfer outcome.
    ///
    /// This is the preferred high-level API for application code. It wraps
    /// [`transfer_blind_and_wait_for`](Self::transfer_blind_and_wait_for) but
    /// returns [`TransferOutcome`] so callers do not need to pattern-match raw
    /// session events.
    ///
    /// `TransferWaitMode::NotifyFinal` returns
    /// [`TransferOutcome::ReferCompleted`] for a final successful REFER NOTIFY;
    /// that means RFC 3515 referenced-request completion, not guaranteed
    /// replacement-call lifecycle. The timeout only cancels this wait.
    pub async fn transfer_blind_and_wait_for_outcome(
        &self,
        target: &str,
        mode: TransferWaitMode,
        timeout: Option<Duration>,
    ) -> Result<TransferOutcome> {
        let event = self
            .transfer_blind_and_wait_for(target, mode, timeout)
            .await?;
        TransferOutcome::try_from(event)
    }

    /// Initiate a blind transfer and wait using optional lifecycle evidence.
    pub async fn transfer_blind_and_wait_for_with_options(
        &self,
        target: &str,
        mode: TransferWaitMode,
        options: TransferLifecycleOptions,
        timeout: Option<Duration>,
    ) -> Result<Event> {
        if mode != TransferWaitMode::ReplacementTerminated && options.dialog_subscription.is_none()
        {
            return self
                .transfer_blind_and_wait_for(target, mode, timeout)
                .await;
        }

        let mut transfer_events = self.events().await?;
        let mut dialog_events = if let Some(subscription) = options.dialog_subscription.as_ref() {
            Some(subscription.events().await?)
        } else {
            None
        };
        self.transfer_blind(target).await?;

        let target = target.to_string();
        let fut = async {
            let mut target_progress_seen = false;
            loop {
                if let Some(dialog_events) = dialog_events.as_mut() {
                    tokio::select! {
                        event = transfer_events.next() => {
                            if let Some(result) = handle_transfer_wait_event(event, mode, &mut target_progress_seen)? {
                                return Ok(result);
                            }
                        }
                        event = dialog_events.next() => {
                            if let Some(result) = handle_dialog_wait_event(
                                event,
                                &self.call_id,
                                &target,
                                mode,
                                &options,
                            )? {
                                return Ok(result);
                            }
                        }
                    }
                } else if let Some(result) = handle_transfer_wait_event(
                    transfer_events.next().await,
                    mode,
                    &mut target_progress_seen,
                )? {
                    return Ok(result);
                }
            }
        };

        match timeout {
            Some(duration) => tokio::time::timeout(duration, fut).await.map_err(|_| {
                SessionError::Timeout(
                    "transfer_blind_and_wait_for_with_options timed out".to_string(),
                )
            })?,
            None => fut.await,
        }
    }

    /// Accept a pending inbound REFER on this call.
    ///
    /// Use this from `StreamPeer` or direct coordinator event handling after an
    /// [`Event::ReferReceived`] event. `CallbackPeer` usually drives this from
    /// its `on_refer_received` builder hook.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// // Call after receiving Event::ReferReceived for this session.
    /// call.accept_refer().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn accept_refer(&self) -> Result<()> {
        self.coordinator.accept_refer(&self.call_id).await
    }

    /// Reject a pending inbound REFER on this call.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// call.reject_refer(603, "Decline").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn reject_refer(&self, status_code: u16, reason: &str) -> Result<()> {
        self.coordinator
            .reject_refer(&self.call_id, status_code, reason)
            .await
    }

    /// Attended-transfer primitive: send REFER with a pre-built `Replaces`
    /// header value (RFC 3891). `replaces` is the raw header value
    /// (`call-id;to-tag=<remote>;from-tag=<local>`) — use
    /// [`crate::api::types::DialogIdentity::to_replaces_value`] on the
    /// *consultation* session's identity to produce it. The adapter
    /// URI-escapes the value when embedding it in the Refer-To target.
    ///
    /// rvoip-sip only exposes the wire-level primitive. Linking an
    /// original call to its consultation call, waiting on REFER NOTIFY
    /// progress, and tearing down the consultation after success are all
    /// orchestration concerns for a higher layer (application code or a
    /// dedicated multi-session coordinator).
    ///
    /// Equivalent to
    /// `call.refer(target).with_replaces(replaces).send().await` — reach
    /// for [`refer`](Self::refer) directly when you need to stage
    /// `Referred-By` or other headers alongside the `Replaces`.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(original: rvoip_sip::SessionHandle, consultation: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// if let Some(identity) = consultation.dialog_identity().await? {
    ///     if let Some(replaces) = identity.to_replaces_value() {
    ///         original.transfer_attended("sip:charlie@example.com", &replaces).await?;
    ///         // Equivalent — and exposes the rest of ReferBuilder's setters:
    ///         // original.refer("sip:charlie@example.com")
    ///         //     .with_replaces(&replaces)
    ///         //     .with_referred_by("sip:alice@example.com")
    ///         //     .send().await?;
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn transfer_attended(&self, target: &str, replaces: &str) -> Result<()> {
        self.refer(target).with_replaces(replaces).send().await
    }

    /// SIP-level dialog identity for this session: `Call-ID`, local tag,
    /// remote tag. Returns `None` if the dialog isn't yet established or
    /// has already been cleaned up.
    ///
    /// The identity corresponds to the underlying
    /// [`rvoip_sip_dialog::Dialog`] tracked by [`rvoip_sip_dialog`] — the
    /// returned tuple is what RFC 3891 §3 requires when constructing a
    /// `Replaces` header. Intended for orchestrators building that header
    /// for attended transfer — see
    /// [`transfer_attended`](Self::transfer_attended).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// if let Some(identity) = call.dialog_identity().await? {
    ///     println!("dialog call-id: {}", identity.call_id);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn dialog_identity(&self) -> Result<Option<crate::api::types::DialogIdentity>> {
        self.coordinator.dialog_identity(&self.call_id).await
    }

    // ===== DTMF =====

    /// Send a single RFC 4733 DTMF digit over the active media session.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// call.send_dtmf('1').await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_dtmf(&self, digit: char) -> Result<()> {
        self.coordinator.send_dtmf(&self.call_id, digit).await
    }

    /// Send a SIP INFO request (RFC 6086) with caller-chosen `Content-Type`.
    ///
    /// Typical uses: `application/dtmf-relay` for out-of-band DTMF when a
    /// carrier prefers SIP-INFO over RFC 2833, `application/sipfrag` for
    /// fax (T.38) flow control, or `application/media_control+xml` for
    /// video FIR/PLI requests. The body is sent verbatim.
    ///
    /// Equivalent to
    /// `call.info(content_type).with_body(body).send().await` — reach
    /// for [`info`](Self::info) directly when you need to stage extra
    /// headers (`X-*`, custom routing hints) alongside the INFO.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// call.send_info(
    ///     "application/dtmf-relay",
    ///     b"Signal=1\r\nDuration=100\r\n",
    /// ).await?;
    /// // Equivalent — and exposes the rest of InfoBuilder's setters:
    /// // call.info("application/dtmf-relay")
    /// //     .with_body(bytes::Bytes::from_static(b"Signal=1\r\nDuration=100\r\n"))
    /// //     .send().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_info(&self, content_type: &str, body: &[u8]) -> Result<()> {
        self.info(content_type)
            .with_body(bytes::Bytes::copy_from_slice(body))
            .send()
            .await
    }

    /// Send a SIP NOTIFY request (RFC 6665) on this session's dialog.
    ///
    /// `event_package` populates the required `Event:` header (e.g. `dialog`,
    /// `message-summary`, `presence`, `refer`). `subscription_state` is the
    /// raw `Subscription-State:` header value (`"active;expires=3600"`,
    /// `"terminated;reason=noresource"`, …). The body is sent verbatim with
    /// rvoip-sip-dialog choosing the Content-Type (`message/sipfrag` for the
    /// `refer` package, caller-supplied otherwise — see
    /// `DialogAdapter::send_notify`).
    ///
    /// This helper targets general-purpose NOTIFY emission (custom event
    /// packages, presence, ad-hoc telemetry). RFC 3515 §2.4.5 REFER
    /// progress NOTIFYs are driven automatically by the state machine
    /// when a session is linked as a transfer leg via
    /// [`UnifiedCoordinator::make_transfer_leg`], so apps do not need to
    /// call this helper for transfer progress.
    ///
    /// Equivalent to building via [`notify`](Self::notify): reach for
    /// that shape directly when you need to stage additional headers
    /// (`Retry-After`, custom `X-*`) or target a specific multiplexed
    /// subscription via `for_subscription`.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// call.send_notify(
    ///     "message-summary",
    ///     Some("Messages-Waiting: no\r\n".to_string()),
    ///     Some("active;expires=3600".to_string()),
    /// ).await?;
    /// // Equivalent — and exposes the rest of NotifyBuilder's setters:
    /// // call.notify("message-summary")
    /// //     .with_subscription_state("active;expires=3600")
    /// //     .with_body(bytes::Bytes::from_static(b"Messages-Waiting: no\r\n"))
    /// //     .send().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_notify(
        &self,
        event_package: &str,
        body: Option<String>,
        subscription_state: Option<String>,
    ) -> Result<()> {
        let mut b = self.notify(event_package);
        if let Some(text) = body {
            b = b.with_body(bytes::Bytes::from(text.into_bytes()));
        }
        if let Some(state) = subscription_state {
            b = b.with_subscription_state(state);
        }
        b.send().await
    }

    // ===== Audio =====

    /// Get a duplex audio stream for this session.
    ///
    /// Calling this multiple times creates independent send channels that all
    /// feed the same media session, but only one `AudioReceiver` is valid at a time
    /// (each call creates a new subscription that consumes from that point forward).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// let audio = call.audio().await?;
    /// let (sender, mut receiver) = audio.split();
    /// # let _ = sender;
    /// # let _ = receiver.try_recv();
    /// # Ok(())
    /// # }
    /// ```
    pub async fn audio(&self) -> Result<AudioStream> {
        // Subscribe to receive audio frames from media layer
        let mut subscriber = self.coordinator.subscribe_to_audio(&self.call_id).await?;

        // Create a channel for receiving frames: drain the subscriber into an mpsc channel
        let (recv_tx, recv_rx) = mpsc::channel::<AudioFrame>(AUDIO_STREAM_CHANNEL_FRAMES);
        tokio::spawn(async move {
            while let Some(frame) = subscriber.receiver.recv().await {
                if recv_tx.send(frame).await.is_err() {
                    break; // AudioReceiver dropped
                }
            }
        });

        // Create a channel for sending frames to the media layer
        let coordinator = self.coordinator.clone();
        let call_id = self.call_id.clone();
        let (send_tx, mut send_rx) = mpsc::channel::<AudioFrame>(AUDIO_STREAM_CHANNEL_FRAMES);
        tokio::spawn(async move {
            while let Some(frame) = send_rx.recv().await {
                if let Err(e) = coordinator.send_audio(&call_id, frame).await {
                    tracing::debug!("[SessionHandle] audio send error for {}: {}", call_id, e);
                    break;
                }
            }
        });

        Ok(AudioStream::new(
            AudioSender::new(send_tx),
            AudioReceiver::new(recv_rx),
        ))
    }

    // ===== State / info =====

    /// Get the current call state from the session store.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// let state = call.state().await?;
    /// println!("state: {state:?}");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn state(&self) -> Result<CallState> {
        self.coordinator.get_state(&self.call_id).await
    }

    /// Get detailed session information from the session store.
    ///
    /// Renamed from `info()` in the 2026-05-20 SessionHandle ergonomic
    /// pass to free the `info` slot for the SIP INFO request builder
    /// ([`info`](Self::info)). The returned [`SessionInfo`] still
    /// carries the same per-call inspection data.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// let info = call.session_info().await?;
    /// println!("{} -> {}", info.from, info.to);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn session_info(&self) -> Result<SessionInfo> {
        self.coordinator.get_session_info(&self.call_id).await
    }

    /// Get the current negotiated media-security state for this call.
    ///
    /// Returns `Ok(None)` when media is plaintext or SRTP has not been
    /// negotiated/installed yet.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// if let Some(security) = call.media_security().await? {
    ///     println!("SRTP suite: {:?}", security.suite);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn media_security(&self) -> Result<Option<MediaSecurityState>> {
        Ok(self.lifecycle().await?.media_security)
    }

    /// Get the current event-bus-backed lifecycle snapshot for this call.
    ///
    /// This is a typed inspection view used by the wait helpers. It combines
    /// current session-store state with lifecycle evidence captured from
    /// app-level events before they are published on the global event bus.
    pub async fn lifecycle(&self) -> Result<CallLifecycleSnapshot> {
        Ok(self.coordinator.lifecycle_snapshot(&self.call_id).await)
    }

    /// Wait for typed SRTP media-security negotiation on this call.
    ///
    /// Returns immediately if the current session state already has negotiated
    /// media security. Otherwise subscribes to this call's event stream and
    /// waits for [`Event::MediaSecurityNegotiated`]. The returned state omits
    /// SRTP key material. The timeout only cancels this wait.
    pub async fn wait_for_media_security(
        &self,
        timeout: Option<Duration>,
    ) -> Result<MediaSecurityState> {
        let rx = self.coordinator.lifecycle_watcher(&self.call_id);
        wait_for_lifecycle(
            self,
            rx,
            timeout,
            "wait_for_media_security timed out",
            |snapshot| {
                if let Some(security) = snapshot.media_security.clone() {
                    return Ok(Some(security));
                }
                if let Some(err) = terminal_error(snapshot, "media security was negotiated") {
                    return Err(err);
                }
                Ok(None)
            },
        )
        .await
    }

    // ===== State predicates =====

    /// Check whether the call is currently active (connected and not on hold).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) {
    /// if call.is_active().await {
    ///     println!("call is active");
    /// }
    /// # }
    /// ```
    pub async fn is_active(&self) -> bool {
        matches!(self.state().await, Ok(CallState::Active))
    }

    /// Check whether the call is currently on hold.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) {
    /// if call.is_on_hold().await {
    ///     println!("call is on hold");
    /// }
    /// # }
    /// ```
    pub async fn is_on_hold(&self) -> bool {
        matches!(self.state().await, Ok(CallState::OnHold))
    }

    // ===== Events =====

    /// Subscribe to events for this specific session.
    ///
    /// Returns an [`EventReceiver`](crate::api::stream_peer::EventReceiver)
    /// pre-filtered to this session's [`CallId`].
    /// Each call returns an independent receiver — all subscribers receive the
    /// same events (broadcast semantics via the global event bus).
    ///
    /// Open the receiver before sending a command if the first resulting event
    /// matters. The bus does not replay old events.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// let mut events = call.events().await?;
    /// call.hold().await?;
    /// # let _ = events.next().await;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn events(&self) -> Result<crate::api::stream_peer::EventReceiver> {
        let rx = self.coordinator.subscribe_events().await?;
        Ok(crate::api::stream_peer::EventReceiver::filtered(
            rx,
            self.call_id.clone(),
        ))
    }

    /// Wait for matching provisional progress on this call.
    ///
    /// The predicate is evaluated only for [`Event::CallProgress`] events.
    /// Terminal call events fail the wait immediately; a fast `200 OK`
    /// therefore does not masquerade as progress. The timeout only cancels
    /// this wait.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// let progress = call
    ///     .wait_for_progress(
    ///         |event| matches!(event, rvoip_sip::Event::CallProgress { status_code: 180 | 183, .. }),
    ///         Some(std::time::Duration::from_secs(5)),
    ///     )
    ///     .await?;
    /// # let _ = progress;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn wait_for_progress<F>(
        &self,
        predicate: F,
        timeout: Option<Duration>,
    ) -> Result<Event>
    where
        F: Fn(&Event) -> bool,
    {
        let rx = self.coordinator.lifecycle_watcher(&self.call_id);
        wait_for_lifecycle(
            self,
            rx,
            timeout,
            "wait_for_progress timed out",
            |snapshot| {
                for progress in &snapshot.progress {
                    let event = progress.to_event();
                    if predicate(&event) {
                        return Ok(Some(event));
                    }
                }
                if snapshot.answered.is_some() || snapshot.state.is_some_and(is_answered_state) {
                    return Err(SessionError::Other(
                        "call answered before matching provisional progress".to_string(),
                    ));
                }
                if let Some(err) = terminal_error(snapshot, "matching provisional progress") {
                    return Err(err);
                }
                Ok(None)
            },
        )
        .await
    }

    /// Wait for this call to be answered and return a handle to the same call.
    ///
    /// This is the handle-first equivalent of
    /// [`StreamPeer::wait_for_answered`](crate::StreamPeer::wait_for_answered).
    /// It returns immediately when the current call state is already
    /// established, otherwise it waits for [`Event::CallAnswered`]. The
    /// timeout only cancels this wait.
    pub async fn wait_for_answered(&self, timeout: Option<Duration>) -> Result<SessionHandle> {
        let rx = self.coordinator.lifecycle_watcher(&self.call_id);
        wait_for_lifecycle(
            self,
            rx,
            timeout,
            "wait_for_answered timed out",
            |snapshot| {
                if snapshot.answered.is_some() || snapshot.state.is_some_and(is_answered_state) {
                    return Ok(Some(self.clone()));
                }
                if let Some(err) = terminal_error(snapshot, "answer") {
                    return Err(err);
                }
                if snapshot.state.is_some_and(|state| state.is_final()) {
                    return Err(SessionError::Other(
                        "call reached terminal state before answer".to_string(),
                    ));
                }
                Ok(None)
            },
        )
        .await
    }

    /// Wait for this specific call to end, with optional timeout.
    ///
    /// Returns the reason the call ended, or a `Timeout` error if the deadline
    /// is reached first. The timeout only cancels this wait.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// let reason = call.wait_for_end(Some(std::time::Duration::from_secs(30))).await?;
    /// println!("call ended: {reason}");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn wait_for_end(&self, timeout: Option<Duration>) -> Result<String> {
        let rx = self.coordinator.lifecycle_watcher(&self.call_id);
        wait_for_lifecycle(self, rx, timeout, "wait_for_end timed out", |snapshot| {
            Ok(terminal_reason(snapshot))
        })
        .await
    }

    fn spawn_late_answer_teardown_observer(&self) {
        let timeout = self.coordinator.setup_teardown_timeout_duration();
        if timeout.is_zero() {
            return;
        }

        let handle = self.clone();
        tokio::spawn(async move {
            let mut rx = handle.coordinator.lifecycle_watcher(&handle.call_id);
            let watch = async {
                loop {
                    let snapshot = match handle.lifecycle().await {
                        Ok(snapshot) => snapshot,
                        Err(err) => {
                            tracing::debug!(
                                "late answer teardown observer failed to read lifecycle for {}: {}",
                                handle.call_id,
                                err
                            );
                            return;
                        }
                    };

                    if snapshot.terminal.is_some()
                        || snapshot.state.is_some_and(|state| state.is_final())
                    {
                        return;
                    }

                    if snapshot.answered.is_some() || snapshot.state.is_some_and(is_answered_state)
                    {
                        if let Err(err) = handle.coordinator.hangup(&handle.call_id).await {
                            tracing::debug!(
                                "late answer teardown observer failed to hang up {}: {}",
                                handle.call_id,
                                err
                            );
                        }
                        return;
                    }

                    if rx.changed().await.is_err() {
                        return;
                    }
                }
            };

            let _ = tokio::time::timeout(timeout, watch).await;
        });
    }
}

fn handle_transfer_wait_event(
    event: Option<Event>,
    mode: TransferWaitMode,
    target_progress_seen: &mut bool,
) -> Result<Option<Event>> {
    match event {
        Some(event @ Event::ReferProgress { status_code, .. })
            if (180..=199).contains(&status_code) =>
        {
            *target_progress_seen = true;
            if mode == TransferWaitMode::TargetRinging {
                Ok(Some(event))
            } else {
                Ok(None)
            }
        }
        Some(event @ Event::ReferCompleted { .. }) => match mode {
            TransferWaitMode::NotifyFinal => Ok(Some(event)),
            TransferWaitMode::TargetRinging => Err(SessionError::Other(
                "terminal REFER NOTIFY arrived before target ringing evidence".to_string(),
            )),
            TransferWaitMode::TargetAnswered if *target_progress_seen => Ok(Some(event)),
            TransferWaitMode::TargetAnswered => Err(SessionError::Other(
                "terminal REFER NOTIFY arrived before target-answer evidence".to_string(),
            )),
            TransferWaitMode::ReplacementTerminated => Ok(None),
        },
        Some(event @ Event::TransferTargetAnswered { .. })
            if mode == TransferWaitMode::TargetAnswered =>
        {
            Ok(Some(event))
        }
        Some(event @ Event::TransferReplacementDialogTerminated { .. })
            if mode == TransferWaitMode::ReplacementTerminated =>
        {
            Ok(Some(event))
        }
        Some(event @ Event::TransferFailed { .. }) => Ok(Some(event)),
        Some(_) => Ok(None),
        None => Err(SessionError::Other(
            "Event channel closed while waiting for transfer".to_string(),
        )),
    }
}

fn handle_dialog_wait_event(
    event: Option<Event>,
    transfer_call_id: &CallId,
    target: &str,
    mode: TransferWaitMode,
    options: &TransferLifecycleOptions,
) -> Result<Option<Event>> {
    match event {
        Some(Event::DialogPackageNotify { dialogs, .. }) => {
            for dialog in dialogs {
                if let Some(event) =
                    dialog_lifecycle_event(dialog, transfer_call_id, target, mode, options)
                {
                    return Ok(Some(event));
                }
            }
            Ok(None)
        }
        Some(Event::DialogStateChanged { dialog, .. }) => Ok(dialog_lifecycle_event(
            dialog,
            transfer_call_id,
            target,
            mode,
            options,
        )),
        Some(_) => Ok(None),
        None => Err(SessionError::Other(
            "Dialog event channel closed while waiting for transfer lifecycle".to_string(),
        )),
    }
}

fn dialog_lifecycle_event(
    dialog: DialogInfo,
    transfer_call_id: &CallId,
    target: &str,
    mode: TransferWaitMode,
    options: &TransferLifecycleOptions,
) -> Option<Event> {
    if !dialog_matches_target(&dialog, target, &options.target_match) {
        return None;
    }

    if dialog.is_terminated() {
        return Some(Event::TransferReplacementDialogTerminated {
            transfer_call_id: transfer_call_id.clone(),
            reason: dialog.raw_event.clone(),
            dialog,
        });
    }

    match mode {
        TransferWaitMode::TargetRinging
            if matches!(
                dialog.state,
                DialogPackageState::Trying
                    | DialogPackageState::Proceeding
                    | DialogPackageState::Early
                    | DialogPackageState::Confirmed
            ) =>
        {
            Some(Event::TransferReplacementDialogObserved {
                transfer_call_id: transfer_call_id.clone(),
                dialog,
            })
        }
        TransferWaitMode::TargetAnswered if dialog.state == DialogPackageState::Confirmed => {
            Some(Event::TransferTargetAnswered {
                transfer_call_id: transfer_call_id.clone(),
                target_uri: target.to_string(),
                evidence: TransferTargetEvidence::DialogPackage { dialog },
            })
        }
        _ => None,
    }
}

fn dialog_matches_target(
    dialog: &DialogInfo,
    target: &str,
    matcher: &TransferDialogMatcher,
) -> bool {
    match matcher {
        TransferDialogMatcher::Any => true,
        TransferDialogMatcher::TargetUri => {
            let target = normalize_uri_for_match(target);
            dialog
                .local_uri
                .iter()
                .chain(dialog.remote_uri.iter())
                .map(|uri| normalize_uri_for_match(uri))
                .any(|uri| uri == target || uri.contains(&target) || target.contains(&uri))
        }
    }
}

fn is_answered_state(state: CallState) -> bool {
    matches!(
        state,
        CallState::Active
            | CallState::HoldPending
            | CallState::OnHold
            | CallState::Resuming
            | CallState::Muted
            | CallState::Bridged
            | CallState::Transferring
            | CallState::TransferringCall
            | CallState::ConsultationCall
    )
}

fn terminal_reason(snapshot: &CallLifecycleSnapshot) -> Option<String> {
    snapshot.terminal.as_ref().map(|terminal| terminal.reason())
}

fn terminal_error(snapshot: &CallLifecycleSnapshot, context: &str) -> Option<SessionError> {
    match snapshot.terminal.as_ref()? {
        CallTerminalInfo::Ended { reason } => Some(SessionError::Other(format!(
            "call ended before {context}: {reason}"
        ))),
        CallTerminalInfo::Failed {
            status_code,
            reason,
        } => Some(SessionError::Other(format!(
            "call failed before {context}: {status_code} {reason}"
        ))),
        CallTerminalInfo::Cancelled => Some(SessionError::Other(format!(
            "call cancelled before {context}"
        ))),
    }
}

async fn wait_for_lifecycle<T, F>(
    handle: &SessionHandle,
    mut rx: tokio::sync::watch::Receiver<u64>,
    timeout: Option<Duration>,
    timeout_message: &'static str,
    mut evaluate: F,
) -> Result<T>
where
    F: FnMut(&CallLifecycleSnapshot) -> Result<Option<T>>,
{
    let fut = async {
        loop {
            let snapshot = handle.lifecycle().await?;
            if let Some(value) = evaluate(&snapshot)? {
                return Ok(value);
            }

            if rx.changed().await.is_err() {
                let snapshot = handle.lifecycle().await?;
                if let Some(value) = evaluate(&snapshot)? {
                    return Ok(value);
                }
                return Err(SessionError::Other(
                    "Lifecycle waiter closed before matching event".to_string(),
                ));
            }
        }
    };

    match timeout {
        Some(duration) => tokio::time::timeout(duration, fut)
            .await
            .map_err(|_| SessionError::Timeout(timeout_message.to_string()))?,
        None => fut.await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::events::{MediaSecurityKeying, MediaSecurityProfile};
    use crate::api::unified::Config;
    use rvoip_sip_core::types::sdp::CryptoSuite;

    fn test_config(port: u16) -> Config {
        let mut config = Config::local("handle-api-test", port);
        config.media_port_start = port + 1000;
        config.media_port_end = port + 1100;
        config
    }

    async fn publish_synthetic(coordinator: &UnifiedCoordinator, event: Event) {
        coordinator
            .publish_app_event_for_test(event)
            .await
            .expect("publish synthetic event");
    }

    #[tokio::test]
    async fn session_handle_wait_for_answered_observes_typed_event() {
        let coordinator = UnifiedCoordinator::new(test_config(35680)).await.unwrap();
        let call_id = SessionId::new();
        let handle = SessionHandle::new(call_id.clone(), coordinator.clone());

        let waiter = tokio::spawn({
            let handle = handle.clone();
            async move { handle.wait_for_answered(Some(Duration::from_secs(2))).await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;

        publish_synthetic(
            &coordinator,
            Event::CallAnswered {
                call_id: call_id.clone(),
                sdp: None,
            },
        )
        .await;

        let answered = waiter.await.unwrap().unwrap();
        assert_eq!(answered.id(), &call_id);
        coordinator.shutdown();
    }

    #[tokio::test]
    async fn session_handle_wait_for_media_security_observes_typed_event() {
        let coordinator = UnifiedCoordinator::new(test_config(35690)).await.unwrap();
        let call_id = SessionId::new();
        let handle = SessionHandle::new(call_id.clone(), coordinator.clone());

        let waiter = tokio::spawn({
            let handle = handle.clone();
            async move {
                handle
                    .wait_for_media_security(Some(Duration::from_secs(2)))
                    .await
            }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;

        publish_synthetic(
            &coordinator,
            Event::MediaSecurityNegotiated {
                call_id,
                keying: MediaSecurityKeying::Sdes,
                suite: CryptoSuite::AesCm128HmacSha1_80,
                profile: MediaSecurityProfile::RtpSavp,
                contexts_installed: true,
            },
        )
        .await;

        let security = waiter.await.unwrap().unwrap();
        assert_eq!(security.keying, MediaSecurityKeying::Sdes);
        assert_eq!(security.suite, CryptoSuite::AesCm128HmacSha1_80);
        assert_eq!(security.profile, MediaSecurityProfile::RtpSavp);
        assert!(security.contexts_installed);
        coordinator.shutdown();
    }

    #[test]
    fn transfer_outcome_maps_raw_transfer_events() {
        let call_id = SessionId::new();

        let outcome = TransferOutcome::try_from(Event::ReferCompleted {
            call_id: call_id.clone(),
            target: "sip:1003@example.com".into(),
            status_code: 200,
            reason: "OK".into(),
        })
        .unwrap();
        assert!(matches!(
            outcome,
            TransferOutcome::ReferCompleted {
                status_code: 200,
                ..
            }
        ));

        let outcome = TransferOutcome::try_from(Event::ReferProgress {
            call_id: call_id.clone(),
            status_code: 180,
            reason: "Ringing".into(),
        })
        .unwrap();
        assert!(matches!(
            outcome,
            TransferOutcome::TargetRinging {
                status_code: 180,
                ..
            }
        ));

        let outcome = TransferOutcome::try_from(Event::TransferTargetAnswered {
            transfer_call_id: call_id.clone(),
            target_uri: "sip:1003@example.com".into(),
            evidence: TransferTargetEvidence::ReferProgressThenFinal {
                progress_status_code: 180,
                progress_reason: "Ringing".into(),
                final_status_code: 200,
                final_reason: "OK".into(),
            },
        })
        .unwrap();
        assert!(matches!(outcome, TransferOutcome::TargetAnswered { .. }));

        let outcome = TransferOutcome::try_from(Event::TransferFailed {
            call_id,
            status_code: 603,
            reason: "Decline".into(),
        })
        .unwrap();
        assert!(matches!(
            outcome,
            TransferOutcome::Failed {
                status_code: 603,
                ..
            }
        ));
    }
}

fn normalize_uri_for_match(uri: &str) -> String {
    uri.trim()
        .trim_matches('<')
        .trim_matches('>')
        .split(';')
        .next()
        .unwrap_or(uri)
        .to_ascii_lowercase()
}
