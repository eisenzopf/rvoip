//! Per-call control API shared by all peer surfaces.
//!
//! [`SessionHandle`] is returned by high-level peer APIs when a call is
//! created or accepted. It is a cheap, cloneable handle to the underlying
//! session and exposes the operations application developers usually need
//! once a call exists: hangup, hold/resume, DTMF, transfer, INFO/NOTIFY,
//! event subscription, state inspection, and audio frames.

use rvoip_media_core::types::AudioFrame;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::api::audio::{AudioReceiver, AudioSender, AudioStream};
use crate::api::events::Event;
use crate::api::unified::UnifiedCoordinator;
use crate::errors::{Result, SessionError};
use crate::state_table::types::SessionId;
use crate::types::{CallState, SessionInfo};

/// Type alias so callers can refer to a session by `CallId`.
pub type CallId = SessionId;

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
    pub async fn hold(&self) -> Result<()> {
        self.coordinator.hold(&self.call_id).await
    }

    /// Resume a held call with a target-refresh re-INVITE.
    ///
    /// On success, applications observe [`Event::CallResumed`].
    pub async fn resume(&self) -> Result<()> {
        self.coordinator.resume(&self.call_id).await
    }

    /// Mute local audio.
    ///
    /// This is a local media-state transition; it does not place the SIP dialog
    /// on hold. Use [`hold`](Self::hold) when the remote peer must be signalled.
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
    pub async fn transfer_blind(&self, target: &str) -> Result<()> {
        self.coordinator.send_refer(&self.call_id, target).await
    }

    /// Initiate a blind transfer and wait for a terminal transfer event.
    ///
    /// Returns `TransferCompleted` on success or `TransferFailed` on failure.
    /// Intermediate progress events are consumed while waiting, so create a
    /// separate event receiver if another task also needs to observe them.
    pub async fn transfer_blind_and_wait(
        &self,
        target: &str,
        timeout: Option<Duration>,
    ) -> Result<Event> {
        let mut events = self.events().await?;
        self.transfer_blind(target).await?;

        let fut = async {
            loop {
                match events.next().await {
                    Some(event @ Event::TransferCompleted { .. })
                    | Some(event @ Event::TransferFailed { .. }) => return Ok(event),
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
    pub async fn accept_refer(&self) -> Result<()> {
        self.coordinator.accept_refer(&self.call_id).await
    }

    /// Reject a pending inbound REFER on this call.
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
    pub async fn dialog_identity(&self) -> Result<Option<crate::api::types::DialogIdentity>> {
        self.coordinator.dialog_identity(&self.call_id).await
    }

    // ===== DTMF =====

    /// Send a single RFC 4733 DTMF digit over the active media session.
    pub async fn send_dtmf(&self, digit: char) -> Result<()> {
        self.coordinator.send_dtmf(&self.call_id, digit).await
    }

    /// Send a SIP INFO request (RFC 6086) with caller-chosen `Content-Type`.
    ///
    /// Typical uses: `application/dtmf-relay` for out-of-band DTMF when a
    /// carrier prefers SIP-INFO over RFC 2833, `application/sipfrag` for
    /// fax (T.38) flow control, or `application/media_control+xml` for
    /// video FIR/PLI requests. The body is sent verbatim.
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
    pub async fn state(&self) -> Result<CallState> {
        self.coordinator.get_state(&self.call_id).await
    }

    /// Get detailed session information from the session store.
    pub async fn info(&self) -> Result<SessionInfo> {
        self.coordinator.get_session_info(&self.call_id).await
    }

    // ===== State predicates =====

    /// Check whether the call is currently active (connected and not on hold).
    pub async fn is_active(&self) -> bool {
        matches!(self.state().await, Ok(CallState::Active))
    }

    /// Check whether the call is currently on hold.
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
    pub async fn events(&self) -> Result<crate::api::stream_peer::EventReceiver> {
        let rx = self.coordinator.subscribe_events().await?;
        Ok(crate::api::stream_peer::EventReceiver::filtered(
            rx,
            self.call_id.clone(),
        ))
    }

    /// Wait for this specific call to end, with optional timeout.
    ///
    /// Returns the reason the call ended, or a `Timeout` error if the deadline
    /// is reached first.
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
