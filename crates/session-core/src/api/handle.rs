//! Per-call control API shared by all peer surfaces.
//!
//! [`SessionHandle`] is returned by high-level peer APIs when a call is
//! created or accepted. It is a cheap, cloneable handle to the underlying
//! session and exposes the operations application developers usually need
//! once a call exists: hangup, hold/resume, DTMF, transfer, INFO/NOTIFY,
//! event subscription, state inspection, and audio frames.

#![deny(missing_docs)]

use rvoip_media_core::types::AudioFrame;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::api::audio::{AudioReceiver, AudioSender, AudioStream};
use crate::api::events::{Event, MediaSecurityState};
use crate::api::unified::UnifiedCoordinator;
use crate::errors::{Result, SessionError};
use crate::state_table::types::SessionId;
use crate::types::{CallState, SessionInfo};

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
}

/// Handle for controlling an active SIP call session.
///
/// Returned by [`StreamPeer::call`](crate::api::stream_peer::StreamPeer::call),
/// [`IncomingCall::accept`](crate::api::incoming::IncomingCall::accept), and
/// similar methods.
///
/// `SessionHandle` is cheap to clone — all clones control the same underlying session.
/// It is `Send + Sync` and safe to share across tasks.
///
/// Most methods are thin call-control operations; the event stream is available
/// through [`events`](Self::events) when the caller needs to observe the result
/// of asynchronous SIP behavior such as remote hangup, REFER progress, DTMF,
/// or hold/resume completion. Deterministic helpers such as
/// [`hangup_and_wait`](Self::hangup_and_wait) and
/// [`transfer_blind_and_wait`](Self::transfer_blind_and_wait) subscribe before
/// sending the command so tests and servers can wait for terminal events.
///
/// # Example
///
/// ```rust,no_run
/// # async fn example(handle: rvoip_session_core::SessionHandle) -> anyhow::Result<()> {
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

    // ===== Call control =====

    /// Hang up the call.
    ///
    /// Fire-and-forget: schedules BYE/CANCEL and returns immediately.
    /// Subscribe to events or use [`hangup_and_wait`](Self::hangup_and_wait)
    /// when the caller needs to observe `CallEnded`, `CallFailed`, or
    /// `CallCancelled`.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_session_core::SessionHandle) -> rvoip_session_core::Result<()> {
    /// call.hangup().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn hangup(&self) -> Result<()> {
        let coordinator = self.coordinator.clone();
        let call_id = self.call_id.clone();
        tokio::spawn(async move {
            if let Err(e) = coordinator.hangup(&call_id).await {
                if e.is_session_gone() {
                    tracing::trace!(
                        "[SessionHandle] session {} already cleaned up before background hangup ran",
                        call_id
                    );
                } else {
                    tracing::warn!(
                        "[SessionHandle] background hangup failed for {}: {}",
                        call_id,
                        e
                    );
                }
            }
        });
        Ok(())
    }

    /// Hang up the call and wait for the terminal event.
    ///
    /// Unlike [`hangup`](Self::hangup), this subscribes to the call's event
    /// stream before sending BYE/CANCEL and returns only after
    /// `CallEnded`, `CallFailed`, or `CallCancelled` is observed.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_session_core::SessionHandle) -> rvoip_session_core::Result<()> {
    /// let reason = call
    ///     .hangup_and_wait(Some(std::time::Duration::from_secs(3)))
    ///     .await?;
    /// println!("call ended: {reason}");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn hangup_and_wait(&self, timeout: Option<Duration>) -> Result<String> {
        let mut events = self.events().await?;
        self.coordinator.hangup(&self.call_id).await?;

        let fut = async {
            loop {
                match events.next().await {
                    Some(Event::CallEnded { reason, .. }) => return Ok(reason),
                    Some(Event::CallFailed { reason, .. }) => return Ok(reason),
                    Some(Event::CallCancelled { .. }) => return Ok("Cancelled".to_string()),
                    Some(_) => {}
                    None => {
                        return Err(SessionError::Other(
                            "Event channel closed while waiting for hangup".to_string(),
                        ))
                    }
                }
            }
        };

        match timeout {
            Some(duration) => tokio::time::timeout(duration, fut)
                .await
                .map_err(|_| SessionError::Timeout("hangup_and_wait timed out".to_string()))?,
            None => fut.await,
        }
    }

    /// Put the call on hold with a target-refresh re-INVITE.
    ///
    /// On success, applications observe [`Event::CallOnHold`] through the
    /// peer/coordinator event stream.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_session_core::SessionHandle) -> rvoip_session_core::Result<()> {
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
    /// # async fn example(call: rvoip_session_core::SessionHandle) -> rvoip_session_core::Result<()> {
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
    /// # async fn example(call: rvoip_session_core::SessionHandle) -> rvoip_session_core::Result<()> {
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
    /// # async fn example(call: rvoip_session_core::SessionHandle) -> rvoip_session_core::Result<()> {
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
    /// [`transfer_blind_and_wait`](Self::transfer_blind_and_wait) when the
    /// caller needs to wait for terminal transfer success/failure.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_session_core::SessionHandle) -> rvoip_session_core::Result<()> {
    /// call.transfer_blind("sip:charlie@example.com").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn transfer_blind(&self, target: &str) -> Result<()> {
        self.coordinator.send_refer(&self.call_id, target).await
    }

    /// Initiate a blind transfer and wait for a terminal REFER NOTIFY.
    ///
    /// Returns `TransferCompleted` on a final 2xx sipfrag or `TransferFailed`
    /// on a final failure sipfrag. `TransferCompleted` means the REFER
    /// subscription reached a terminal success state; it does not prove the
    /// transfer target answered. Use
    /// [`transfer_blind_and_wait_for`](Self::transfer_blind_and_wait_for)
    /// with [`TransferWaitMode::TargetAnswered`] when target evidence matters.
    /// Intermediate progress events are consumed while waiting, so create a
    /// separate event receiver if another task also needs to observe them.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_session_core::SessionHandle) -> rvoip_session_core::Result<()> {
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
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use rvoip_session_core::TransferWaitMode;
    /// # async fn example(call: rvoip_session_core::SessionHandle) -> rvoip_session_core::Result<()> {
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
                    Some(event @ Event::TransferProgress { status_code, .. })
                        if (180..=199).contains(&status_code) =>
                    {
                        target_progress_seen = true;
                        if mode == TransferWaitMode::TargetRinging {
                            return Ok(event);
                        }
                    }
                    Some(event @ Event::TransferCompleted { .. }) => match mode {
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
                    },
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

    /// Accept a pending inbound REFER on this call.
    ///
    /// Use this from `StreamPeer` or direct coordinator event handling after an
    /// [`Event::ReferReceived`] event. `CallbackPeer` usually drives this from
    /// [`CallHandler::on_transfer_request`](crate::api::callback_peer::CallHandler::on_transfer_request).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_session_core::SessionHandle) -> rvoip_session_core::Result<()> {
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
    /// # async fn example(call: rvoip_session_core::SessionHandle) -> rvoip_session_core::Result<()> {
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
    /// session-core only exposes the wire-level primitive. Linking an
    /// original call to its consultation call, waiting on REFER NOTIFY
    /// progress, and tearing down the consultation after success are all
    /// orchestration concerns for a higher layer (application code or a
    /// dedicated multi-session coordinator).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(original: rvoip_session_core::SessionHandle, consultation: rvoip_session_core::SessionHandle) -> rvoip_session_core::Result<()> {
    /// if let Some(identity) = consultation.dialog_identity().await? {
    ///     if let Some(replaces) = identity.to_replaces_value() {
    ///         original.transfer_attended("sip:charlie@example.com", &replaces).await?;
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn transfer_attended(&self, target: &str, replaces: &str) -> Result<()> {
        self.coordinator
            .send_refer_with_replaces(&self.call_id, target, replaces)
            .await
    }

    /// SIP-level dialog identity for this session: `Call-ID`, local tag,
    /// remote tag. Returns `None` if the dialog isn't yet established or
    /// has already been cleaned up.
    ///
    /// Intended for orchestrators building a `Replaces` header for
    /// attended transfer — see [`transfer_attended`](Self::transfer_attended).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_session_core::SessionHandle) -> rvoip_session_core::Result<()> {
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
    /// # async fn example(call: rvoip_session_core::SessionHandle) -> rvoip_session_core::Result<()> {
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
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_session_core::SessionHandle) -> rvoip_session_core::Result<()> {
    /// call.send_info(
    ///     "application/dtmf-relay",
    ///     b"Signal=1\r\nDuration=100\r\n",
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_info(&self, content_type: &str, body: &[u8]) -> Result<()> {
        self.coordinator
            .send_info(&self.call_id, content_type, body)
            .await
    }

    /// Send a SIP NOTIFY request (RFC 6665) on this session's dialog.
    ///
    /// `event_package` populates the required `Event:` header (e.g. `dialog`,
    /// `message-summary`, `presence`, `refer`). `subscription_state` is the
    /// raw `Subscription-State:` header value (`"active;expires=3600"`,
    /// `"terminated;reason=noresource"`, …). The body is sent verbatim with
    /// dialog-core choosing the Content-Type (`message/sipfrag` for the
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
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_session_core::SessionHandle) -> rvoip_session_core::Result<()> {
    /// call.send_notify(
    ///     "message-summary",
    ///     Some("Messages-Waiting: no\r\n".to_string()),
    ///     Some("active;expires=3600".to_string()),
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_notify(
        &self,
        event_package: &str,
        body: Option<String>,
        subscription_state: Option<String>,
    ) -> Result<()> {
        self.coordinator
            .send_notify(&self.call_id, event_package, body, subscription_state)
            .await
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
    /// # async fn example(call: rvoip_session_core::SessionHandle) -> rvoip_session_core::Result<()> {
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
        let (recv_tx, recv_rx) = mpsc::channel::<AudioFrame>(512);
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
        let (send_tx, mut send_rx) = mpsc::channel::<AudioFrame>(512);
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
    /// # async fn example(call: rvoip_session_core::SessionHandle) -> rvoip_session_core::Result<()> {
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
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_session_core::SessionHandle) -> rvoip_session_core::Result<()> {
    /// let info = call.info().await?;
    /// println!("{} -> {}", info.from, info.to);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn info(&self) -> Result<SessionInfo> {
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
    /// # async fn example(call: rvoip_session_core::SessionHandle) -> rvoip_session_core::Result<()> {
    /// if let Some(security) = call.media_security().await? {
    ///     println!("SRTP suite: {:?}", security.suite);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn media_security(&self) -> Result<Option<MediaSecurityState>> {
        let session = self
            .coordinator
            .helpers
            .state_machine
            .store
            .get_session(&self.call_id)
            .await
            .map_err(|e| SessionError::SessionNotFound(e.to_string()))?;
        Ok(session.media_security)
    }

    // ===== State predicates =====

    /// Check whether the call is currently active (connected and not on hold).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_session_core::SessionHandle) {
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
    /// # async fn example(call: rvoip_session_core::SessionHandle) {
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
    /// # async fn example(call: rvoip_session_core::SessionHandle) -> rvoip_session_core::Result<()> {
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
    /// therefore does not masquerade as progress.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_session_core::SessionHandle) -> rvoip_session_core::Result<()> {
    /// let progress = call
    ///     .wait_for_progress(
    ///         |event| matches!(event, rvoip_session_core::Event::CallProgress { status_code: 180 | 183, .. }),
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
        let mut rx = self.events().await?;
        let fut = async {
            loop {
                match rx.next().await {
                    Some(event @ Event::CallProgress { .. }) if predicate(&event) => {
                        return Ok(event)
                    }
                    Some(Event::CallAnswered { .. }) => {
                        return Err(SessionError::Other(
                            "call answered before matching provisional progress".to_string(),
                        ))
                    }
                    Some(Event::CallFailed { reason, .. }) => {
                        return Err(SessionError::Other(reason))
                    }
                    Some(Event::CallCancelled { .. }) => {
                        return Err(SessionError::Other(
                            "call cancelled before matching provisional progress".to_string(),
                        ))
                    }
                    Some(Event::CallEnded { reason, .. }) => {
                        return Err(SessionError::Other(reason))
                    }
                    Some(_) => {}
                    None => return Err(SessionError::Other("Event channel closed".to_string())),
                }
            }
        };
        match timeout {
            Some(d) => tokio::time::timeout(d, fut)
                .await
                .map_err(|_| SessionError::Timeout("wait_for_progress timed out".to_string()))?,
            None => fut.await,
        }
    }

    /// Wait for this specific call to end, with optional timeout.
    ///
    /// Returns the reason the call ended, or a `Timeout` error if the deadline
    /// is reached first.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_session_core::SessionHandle) -> rvoip_session_core::Result<()> {
    /// let reason = call.wait_for_end(Some(std::time::Duration::from_secs(30))).await?;
    /// println!("call ended: {reason}");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn wait_for_end(&self, timeout: Option<Duration>) -> Result<String> {
        let mut rx = self.events().await?;
        let fut = async {
            loop {
                match rx.next().await {
                    Some(Event::CallEnded { reason, .. }) => return Ok(reason),
                    Some(Event::CallFailed { reason, .. }) => return Ok(reason),
                    None => return Err(SessionError::Other("Event channel closed".to_string())),
                    _ => {}
                }
            }
        };
        match timeout {
            Some(d) => tokio::time::timeout(d, fut)
                .await
                .map_err(|_| SessionError::Timeout("wait_for_end timed out".to_string()))?,
            None => fut.await,
        }
    }
}
