//! Simplified endpoint API for softphones, PBX accounts, demos, and IVR legs.
//!
//! [`Endpoint`] is the easiest rvoip-sip surface to start with. It wraps
//! [`StreamPeer`], keeps the existing [`SessionHandle`] and [`IncomingCall`]
//! types, and adds only the account/profile conveniences that SIP applications
//! usually need first.
//!
//! For PBX or SBC integrations that require non-standard or vendor INVITE
//! headers, call `endpoint.invite(to).with_extra_headers(...).send()` to
//! attach a caller-supplied `Vec<TypedHeader>` to the first INVITE.

#![deny(missing_docs)]

use std::fmt;
use std::fs;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use tokio::sync::Mutex;

use rvoip_sip_core::types::uri::{Scheme, Uri};

use crate::api::audio::{AudioReceiver, AudioSender, AudioStream};
use crate::api::events::Event;
use crate::api::handle::{CallId, SessionHandle};
use crate::api::incoming::{IncomingCall, IncomingCallGuard};
use crate::api::stream_peer::{EventReceiver, PeerControl, StreamPeer};
use crate::api::unified::{
    Config, Registration, RegistrationHandle, RegistrationInfo, RegistrationStatus, SipTlsMode,
};
use crate::errors::{Result, SessionError};
use crate::types::Credentials;

/// A simplified SIP endpoint built on top of [`StreamPeer`].
///
/// Use `Endpoint` when an application wants a compact softphone/PBX-account
/// style API without losing access to the underlying stream/control objects.
/// Advanced applications can call [`control`](Self::control) or
/// [`into_stream_peer`](Self::into_stream_peer) and continue with the lower
/// level APIs.
pub struct Endpoint {
    peer: StreamPeer,
    registration: Option<Registration>,
    registration_handle: SharedRegistrationHandle,
    registrar: Option<String>,
    transport: EndpointTransport,
}

impl Endpoint {
    /// Start a new [`EndpointBuilder`].
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> rvoip_sip::Result<()> {
    /// let endpoint = rvoip_sip::Endpoint::builder()
    ///     .name("alice")
    ///     .build()
    ///     .await?;
    /// endpoint.shutdown().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn builder() -> EndpointBuilder {
        EndpointBuilder::new()
    }

    /// Build and start an endpoint from a serde-friendly configuration object.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(config: rvoip_sip::EndpointConfig) -> rvoip_sip::Result<()> {
    /// let endpoint = rvoip_sip::Endpoint::from_config(config).await?;
    /// endpoint.shutdown().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn from_config(config: EndpointConfig) -> Result<Self> {
        EndpointBuilder::from_config(config)?.build().await
    }

    /// Load endpoint configuration from a JSON file and start the endpoint.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> rvoip_sip::Result<()> {
    /// let endpoint = rvoip_sip::Endpoint::from_json_file("alice.json").await?;
    /// endpoint.shutdown().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn from_json_file(path: impl AsRef<Path>) -> Result<Self> {
        let text = fs::read_to_string(path.as_ref()).map_err(|err| {
            SessionError::ConfigError(format!(
                "failed to read endpoint JSON config '{}': {err}",
                path.as_ref().display()
            ))
        })?;
        let config = serde_json::from_str::<EndpointConfig>(&text).map_err(|err| {
            SessionError::ConfigError(format!(
                "failed to parse endpoint JSON config '{}': {err}",
                path.as_ref().display()
            ))
        })?;
        Self::from_config(config).await
    }

    /// Register the configured account with its registrar.
    ///
    /// Repeated calls return the existing registration handle. Build the
    /// endpoint with [`EndpointBuilder::account`],
    /// [`EndpointBuilder::password`], and [`EndpointBuilder::registrar`] or
    /// with [`EndpointBuilder::endpoint_account`] before calling this method.
    pub async fn register(&mut self) -> Result<RegistrationHandle> {
        let mut stored = self.registration_handle.lock().await;
        if let Some(handle) = stored.as_ref() {
            return Ok(handle.clone());
        }

        let registration = self.registration.clone().ok_or_else(|| {
            SessionError::ConfigError(
                "Endpoint has no complete registration account; set account, password, and registrar"
                    .to_string(),
            )
        })?;
        let mut b = self
            .peer
            .register(
                registration.registrar.clone(),
                registration.username.clone(),
                registration.password.clone(),
            )
            .with_expires(registration.expires);
        if let Some(from) = registration.from_uri.clone() {
            b = b.with_from_uri(from);
        }
        if let Some(contact) = registration.contact_uri.clone() {
            b = b.with_contact_uri(contact);
        }
        let handle = b.send().await?;
        *stored = Some(handle.clone());
        Ok(handle)
    }

    /// Register the configured account and wait for registrar confirmation.
    pub async fn register_and_wait(
        &mut self,
        timeout: Option<Duration>,
    ) -> Result<EndpointRegistrationInfo> {
        let mut events = self.events().await?;
        let handle = self.register().await?;
        wait_for_registration_result(&mut events, &handle, timeout).await
    }

    /// Unregister the current account if it has been registered.
    ///
    /// Calling this on an endpoint that has not registered is a no-op.
    pub async fn unregister(&mut self) -> Result<()> {
        let mut stored = self.registration_handle.lock().await;
        if let Some(handle) = stored.take() {
            self.peer.unregister(&handle).await?;
        }
        Ok(())
    }

    /// Initiate an outgoing call and wait for it to answer.
    pub async fn call_and_wait(
        &self,
        target: &str,
        timeout: Option<Duration>,
    ) -> Result<EndpointCall> {
        let call_id = self.invite(target)?.send().await?;
        let call = self.wrap_call(call_id);
        call.wait_for_answered(timeout).await
    }

    /// Wait for the next incoming call.
    pub async fn wait_for_incoming(&mut self) -> Result<EndpointIncomingCall> {
        let incoming = self.peer.wait_for_incoming().await?;
        Ok(EndpointIncomingCall::new(
            incoming,
            self.registrar.clone(),
            self.transport,
        ))
    }

    /// Subscribe to endpoint-level events without consuming the endpoint.
    pub async fn events(&self) -> Result<EndpointEvents> {
        let events = self.peer.control().subscribe_events().await?;
        Ok(EndpointEvents::new(
            events,
            self.peer.control().clone(),
            self.registrar.clone(),
            self.transport,
        ))
    }

    /// Split the endpoint into cloneable controls and an endpoint event stream.
    pub fn split(self) -> (EndpointControl, EndpointEvents) {
        let registration = self.registration;
        let registration_handle = self.registration_handle;
        let registrar = self.registrar;
        let transport = self.transport;
        let (control, events) = self.peer.split();
        let endpoint_control = EndpointControl::new(
            control.clone(),
            registration,
            registration_handle,
            registrar.clone(),
            transport,
        );
        let endpoint_events = EndpointEvents::new(events, control, registrar, transport);
        (endpoint_control, endpoint_events)
    }

    /// Access the command half of the wrapped [`StreamPeer`].
    pub fn control(&self) -> &PeerControl {
        self.peer.control()
    }

    /// Resolve a dial target the same way [`invite`](Self::invite) does.
    ///
    /// This is useful for logging or for handing the resolved URI to a lower
    /// level API.
    pub fn resolve_target(&self, target: &str) -> Result<String> {
        normalize_target(self.registrar.as_deref(), target, self.transport)
    }

    /// Begin building an outbound INVITE from this endpoint's
    /// registered AOR (or `local_uri`). Resolves bare extensions
    /// through the configured registrar. Returns an
    /// [`OutboundCallBuilder`](crate::api::send::OutboundCallBuilder).
    ///
    /// Returns `Err` only if the target can't be normalized into a SIP
    /// URI (e.g. a bare extension without a configured registrar).
    pub fn invite(&self, target: &str) -> Result<crate::api::send::OutboundCallBuilder> {
        let resolved = self.resolve_target(target)?;
        Ok(self.peer.control().invite(resolved))
    }

    /// Materialize an [`EndpointCall`] for a `CallId` returned by
    /// [`invite(...).send()`](Self::invite). Pairs with `invite()` the
    /// same way the unified coordinator's `session(...)` pairs with its
    /// bare builder — gives back the rich call wrapper around the raw
    /// [`SessionHandle`].
    pub fn wrap_call(&self, call_id: crate::api::handle::CallId) -> EndpointCall {
        let coord = self.peer.control().coordinator().clone();
        EndpointCall::new(
            crate::api::handle::SessionHandle::new(call_id, coord),
            self.registrar.clone(),
            self.transport,
        )
    }

    /// Consume this endpoint and return the wrapped [`StreamPeer`].
    pub fn into_stream_peer(self) -> StreamPeer {
        self.peer
    }

    /// Gracefully unregister and shut down the endpoint.
    pub async fn shutdown(self) -> Result<()> {
        self.peer.shutdown().await
    }
}

type SharedRegistrationHandle = Arc<Mutex<Option<RegistrationHandle>>>;

/// Cloneable command half returned by [`Endpoint::split`].
#[derive(Clone)]
pub struct EndpointControl {
    control: PeerControl,
    registration: Option<Registration>,
    registration_handle: SharedRegistrationHandle,
    registrar: Option<String>,
    transport: EndpointTransport,
}

impl EndpointControl {
    fn new(
        control: PeerControl,
        registration: Option<Registration>,
        registration_handle: SharedRegistrationHandle,
        registrar: Option<String>,
        transport: EndpointTransport,
    ) -> Self {
        Self {
            control,
            registration,
            registration_handle,
            registrar,
            transport,
        }
    }

    /// Register the configured account.
    pub async fn register(&self) -> Result<()> {
        let mut stored = self.registration_handle.lock().await;
        if stored.is_some() {
            return Ok(());
        }
        let registration = self.registration.clone().ok_or_else(|| {
            SessionError::ConfigError(
                "Endpoint has no complete registration account; set account, password, and registrar"
                    .to_string(),
            )
        })?;
        let mut b = self
            .control
            .coordinator()
            .register(
                registration.registrar,
                registration.username,
                registration.password,
            )
            .with_expires(registration.expires);
        if let Some(from) = registration.from_uri {
            b = b.with_from_uri(from);
        }
        if let Some(contact) = registration.contact_uri {
            b = b.with_contact_uri(contact);
        }
        let handle = b.send().await?;
        *stored = Some(handle);
        Ok(())
    }

    /// Register and wait for a registrar success or failure event.
    pub async fn register_and_wait(
        &self,
        timeout: Option<Duration>,
    ) -> Result<EndpointRegistrationInfo> {
        let mut events = self.events().await?;
        self.register().await?;
        let handle = self
            .registration_handle
            .lock()
            .await
            .clone()
            .ok_or_else(|| SessionError::Other("registration handle missing".to_string()))?;
        wait_for_registration_result(&mut events, &handle, timeout).await
    }

    /// Return the current registration information, if this endpoint registered.
    pub async fn registration_info(&self) -> Result<Option<EndpointRegistrationInfo>> {
        let handle = self.registration_handle.lock().await.clone();
        match handle {
            Some(handle) => self
                .control
                .coordinator()
                .registration_info(&handle)
                .await
                .map(EndpointRegistrationInfo::from)
                .map(Some),
            None => Ok(None),
        }
    }

    /// Unregister the current account, if registered.
    pub async fn unregister(&self) -> Result<()> {
        if let Some(handle) = self.registration_handle.lock().await.take() {
            self.control.coordinator().unregister(&handle).await?;
        }
        Ok(())
    }

    /// Unregister and wait for registrar confirmation.
    pub async fn unregister_and_wait(&self, timeout: Option<Duration>) -> Result<()> {
        if let Some(handle) = self.registration_handle.lock().await.take() {
            self.control
                .coordinator()
                .unregister_and_wait(&handle, timeout)
                .await?;
        }
        Ok(())
    }

    /// Subscribe to Endpoint-level events.
    pub async fn events(&self) -> Result<EndpointEvents> {
        let events = self.control.subscribe_events().await?;
        Ok(EndpointEvents::new(
            events,
            self.control.clone(),
            self.registrar.clone(),
            self.transport,
        ))
    }

    /// Resolve a dial target using this endpoint's account context.
    pub fn resolve_target(&self, target: &str) -> Result<String> {
        normalize_target(self.registrar.as_deref(), target, self.transport)
    }

    /// Begin building an outbound INVITE from this endpoint's
    /// account context. Resolves bare extensions through the configured
    /// registrar.
    pub fn invite(&self, target: &str) -> Result<crate::api::send::OutboundCallBuilder> {
        let resolved = self.resolve_target(target)?;
        Ok(self.control.invite(resolved))
    }

    /// Materialize an [`EndpointCall`] for a `CallId` returned by
    /// [`invite(...).send()`](Self::invite).
    pub fn wrap_call(&self, call_id: crate::api::handle::CallId) -> EndpointCall {
        let coord = self.control.coordinator().clone();
        EndpointCall::new(
            crate::api::handle::SessionHandle::new(call_id, coord),
            self.registrar.clone(),
            self.transport,
        )
    }

    /// Gracefully shut down the endpoint runtime.
    pub async fn shutdown(&self) -> Result<()> {
        self.control.coordinator().shutdown_gracefully(None).await
    }
}

/// Endpoint-level event stream returned by [`Endpoint::split`] and [`Endpoint::events`].
pub struct EndpointEvents {
    events: EventReceiver,
    control: PeerControl,
    registrar: Option<String>,
    transport: EndpointTransport,
}

impl EndpointEvents {
    fn new(
        events: EventReceiver,
        control: PeerControl,
        registrar: Option<String>,
        transport: EndpointTransport,
    ) -> Self {
        Self {
            events,
            control,
            registrar,
            transport,
        }
    }

    /// Wait for the next endpoint event.
    pub async fn next(&mut self) -> Result<Option<EndpointEvent>> {
        Ok(self.events.next().await.map(|event| self.map_event(event)))
    }

    /// Return the next endpoint event if one is ready immediately.
    pub fn try_next(&mut self) -> Option<EndpointEvent> {
        self.events.try_next().map(|event| self.map_event(event))
    }

    fn map_event(&self, event: Event) -> EndpointEvent {
        match event {
            Event::IncomingCall {
                call_id,
                from,
                to,
                sdp,
            } => {
                let incoming =
                    IncomingCall::new(call_id, from, to, sdp, self.control.coordinator().clone());
                EndpointEvent::IncomingCall(EndpointIncomingCall::new(
                    incoming,
                    self.registrar.clone(),
                    self.transport,
                ))
            }
            Event::CallProgress {
                call_id,
                status_code,
                reason,
                sdp,
            } => EndpointEvent::CallProgress {
                call_id: EndpointCallId(call_id),
                status_code,
                reason,
                has_sdp: sdp.is_some(),
            },
            Event::CallAnswered { call_id, sdp } => EndpointEvent::CallAnswered {
                call: EndpointCall::new(
                    SessionHandle::new(call_id, self.control.coordinator().clone()),
                    self.registrar.clone(),
                    self.transport,
                ),
                has_sdp: sdp.is_some(),
            },
            Event::CallEnded { call_id, reason } => EndpointEvent::CallEnded {
                call_id: EndpointCallId(call_id),
                reason,
            },
            Event::CallFailed {
                call_id,
                status_code,
                reason,
            } => EndpointEvent::CallFailed {
                call_id: EndpointCallId(call_id),
                status_code,
                reason,
            },
            Event::CallCancelled { call_id } => EndpointEvent::CallCancelled {
                call_id: EndpointCallId(call_id),
            },
            Event::CallOnHold { call_id } => EndpointEvent::LocalHold {
                call_id: EndpointCallId(call_id),
            },
            Event::CallResumed { call_id } => EndpointEvent::LocalResume {
                call_id: EndpointCallId(call_id),
            },
            Event::RemoteCallOnHold { call_id } => EndpointEvent::RemoteHold {
                call_id: EndpointCallId(call_id),
            },
            Event::RemoteCallResumed { call_id } => EndpointEvent::RemoteResume {
                call_id: EndpointCallId(call_id),
            },
            Event::DtmfReceived { call_id, digit } => EndpointEvent::DtmfReceived {
                call_id: EndpointCallId(call_id),
                digit,
            },
            Event::RegistrationSuccess {
                registrar,
                expires,
                contact,
            } => EndpointEvent::RegistrationChanged(EndpointRegistrationInfo {
                status: EndpointRegistrationStatus::Registered,
                registrar: Some(registrar),
                contact: Some(contact),
                expires_secs: Some(expires),
                accepted_expires_secs: Some(expires),
                next_refresh_in: None,
                retry_count: 0,
                last_failure: None,
            }),
            Event::RegistrationFailed {
                registrar,
                status_code,
                reason,
            } => EndpointEvent::RegistrationChanged(EndpointRegistrationInfo {
                status: EndpointRegistrationStatus::Failed,
                registrar: Some(registrar),
                contact: None,
                expires_secs: None,
                accepted_expires_secs: None,
                next_refresh_in: None,
                retry_count: 0,
                last_failure: Some(format!("{status_code} {reason}")),
            }),
            Event::UnregistrationSuccess { registrar } => {
                EndpointEvent::RegistrationChanged(EndpointRegistrationInfo {
                    status: EndpointRegistrationStatus::Unregistered,
                    registrar: Some(registrar),
                    contact: None,
                    expires_secs: None,
                    accepted_expires_secs: None,
                    next_refresh_in: None,
                    retry_count: 0,
                    last_failure: None,
                })
            }
            Event::UnregistrationFailed { registrar, reason } => {
                EndpointEvent::RegistrationChanged(EndpointRegistrationInfo {
                    status: EndpointRegistrationStatus::Failed,
                    registrar: Some(registrar),
                    contact: None,
                    expires_secs: None,
                    accepted_expires_secs: None,
                    next_refresh_in: None,
                    retry_count: 0,
                    last_failure: Some(reason),
                })
            }
            Event::NetworkError { call_id, error } => EndpointEvent::NetworkError {
                call_id: call_id.map(EndpointCallId),
                error,
            },
            Event::SipTrace(trace) => EndpointEvent::SipTrace(EndpointSipTrace {
                direction: trace.direction,
                transport: trace.transport,
                local_addr: trace.local_addr,
                remote_addr: trace.remote_addr,
                timestamp_unix_millis: trace.timestamp_unix_millis,
                start_line: trace.start_line,
                sip_call_id: trace.sip_call_id,
                session_id: trace.session_id.map(EndpointCallId),
                raw_message: trace.raw_message,
                original_len: trace.original_len,
                truncated: trace.truncated,
                redacted: trace.redacted,
            }),
            other => EndpointEvent::Info {
                call_id: other.call_id().cloned().map(EndpointCallId),
                message: format!("{other:?}"),
            },
        }
    }
}

/// Endpoint-level event type for softphone applications.
pub enum EndpointEvent {
    /// A new inbound call is ringing.
    IncomingCall(EndpointIncomingCall),
    /// An outgoing call received provisional progress.
    CallProgress {
        /// Call identifier.
        call_id: EndpointCallId,
        /// SIP status code.
        status_code: u16,
        /// SIP reason phrase.
        reason: String,
        /// Whether the event included SDP.
        has_sdp: bool,
    },
    /// A call was answered and is now controllable.
    CallAnswered {
        /// Active call handle.
        call: EndpointCall,
        /// Whether the event included SDP.
        has_sdp: bool,
    },
    /// A call ended.
    CallEnded {
        /// Call identifier.
        call_id: EndpointCallId,
        /// End reason.
        reason: String,
    },
    /// A call failed.
    CallFailed {
        /// Call identifier.
        call_id: EndpointCallId,
        /// SIP status code.
        status_code: u16,
        /// Failure reason.
        reason: String,
    },
    /// A ringing incoming call was cancelled by the caller.
    CallCancelled {
        /// Call identifier.
        call_id: EndpointCallId,
    },
    /// Local hold completed.
    LocalHold {
        /// Call identifier.
        call_id: EndpointCallId,
    },
    /// Local resume completed.
    LocalResume {
        /// Call identifier.
        call_id: EndpointCallId,
    },
    /// Remote hold was observed.
    RemoteHold {
        /// Call identifier.
        call_id: EndpointCallId,
    },
    /// Remote resume was observed.
    RemoteResume {
        /// Call identifier.
        call_id: EndpointCallId,
    },
    /// DTMF was received.
    DtmfReceived {
        /// Call identifier.
        call_id: EndpointCallId,
        /// Received digit.
        digit: char,
    },
    /// Registration state changed.
    RegistrationChanged(EndpointRegistrationInfo),
    /// SIP message observed at the transport boundary.
    SipTrace(EndpointSipTrace),
    /// A network error occurred.
    NetworkError {
        /// Call identifier, when known.
        call_id: Option<EndpointCallId>,
        /// Error text.
        error: String,
    },
    /// Informational event not otherwise modeled by the endpoint facade.
    Info {
        /// Call identifier, when known.
        call_id: Option<EndpointCallId>,
        /// Human-readable event summary.
        message: String,
    },
}

/// Endpoint-level SIP trace event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointSipTrace {
    /// Inbound or outbound at the local transport boundary.
    pub direction: crate::api::events::SipTraceDirection,
    /// Transport flavour, for example `UDP`, `TCP`, or `TLS`.
    pub transport: String,
    /// Local socket address.
    pub local_addr: String,
    /// Remote socket address.
    pub remote_addr: String,
    /// Milliseconds since Unix epoch when the trace event was created.
    pub timestamp_unix_millis: u64,
    /// SIP start line.
    pub start_line: String,
    /// Wire-level SIP `Call-ID` header value when present.
    pub sip_call_id: Option<String>,
    /// Endpoint call/session id after mapping, when known.
    pub session_id: Option<EndpointCallId>,
    /// Redacted, optionally body-stripped SIP message text.
    pub raw_message: String,
    /// Original rendered message byte length before redaction/body stripping/truncation.
    pub original_len: usize,
    /// Whether `raw_message` was truncated for bounded diagnostics.
    pub truncated: bool,
    /// Whether sensitive headers were redacted.
    pub redacted: bool,
}

/// Opaque call identifier for Endpoint applications.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EndpointCallId(CallId);

impl fmt::Display for EndpointCallId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Active call handle returned by Endpoint APIs.
#[derive(Clone)]
pub struct EndpointCall {
    handle: SessionHandle,
    registrar: Option<String>,
    transport: EndpointTransport,
}

impl EndpointCall {
    fn new(handle: SessionHandle, registrar: Option<String>, transport: EndpointTransport) -> Self {
        Self {
            handle,
            registrar,
            transport,
        }
    }

    /// Return this call's opaque identifier.
    pub fn id(&self) -> EndpointCallId {
        EndpointCallId(self.handle.id().clone())
    }

    /// Return the underlying session handle for advanced operations that are
    /// not yet modeled directly on the endpoint facade.
    pub fn as_session_handle(&self) -> &SessionHandle {
        &self.handle
    }

    /// Wait for this outgoing call to be answered.
    pub async fn wait_for_answered(&self, timeout: Option<Duration>) -> Result<Self> {
        let handle = self.handle.wait_for_answered(timeout).await?;
        Ok(Self::new(handle, self.registrar.clone(), self.transport))
    }

    /// Wait for this call to end.
    pub async fn wait_for_end(&self, timeout: Option<Duration>) -> Result<String> {
        self.handle.wait_for_end(timeout).await
    }

    /// Hang up the call.
    pub async fn hangup(&self) -> Result<()> {
        self.handle.hangup().await
    }

    /// Hang up the call and wait for teardown.
    pub async fn hangup_and_wait(&self, timeout: Option<Duration>) -> Result<String> {
        self.handle.hangup_and_wait(timeout).await
    }

    /// Put the call on local hold.
    pub async fn hold(&self) -> Result<()> {
        self.handle.hold().await
    }

    /// Resume a locally held call.
    pub async fn resume(&self) -> Result<()> {
        self.handle.resume().await
    }

    /// Mute local microphone media for the call.
    pub async fn mute(&self) -> Result<()> {
        self.handle.mute().await
    }

    /// Unmute local microphone media for the call.
    pub async fn unmute(&self) -> Result<()> {
        self.handle.unmute().await
    }

    /// Send an RFC 4733 DTMF digit.
    pub async fn send_dtmf(&self, digit: char) -> Result<()> {
        self.handle.send_dtmf(digit).await
    }

    /// Blind-transfer the call using Endpoint target resolution.
    pub async fn transfer(&self, target: &str) -> Result<()> {
        let target = normalize_target(self.registrar.as_deref(), target, self.transport)?;
        self.handle.transfer_blind(&target).await
    }

    /// Open the call's bidirectional audio stream.
    pub async fn audio(&self) -> Result<EndpointAudio> {
        self.handle.audio().await.map(EndpointAudio::new)
    }
}

impl fmt::Debug for EndpointCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EndpointCall")
            .field("id", &self.id().to_string())
            .finish()
    }
}

/// Inbound call presented by [`EndpointEvent::IncomingCall`].
pub struct EndpointIncomingCall {
    incoming: IncomingCall,
    registrar: Option<String>,
    transport: EndpointTransport,
}

impl EndpointIncomingCall {
    fn new(
        incoming: IncomingCall,
        registrar: Option<String>,
        transport: EndpointTransport,
    ) -> Self {
        Self {
            incoming,
            registrar,
            transport,
        }
    }

    /// Return the inbound call identifier.
    pub fn id(&self) -> EndpointCallId {
        EndpointCallId(self.incoming.call_id.clone())
    }

    /// Return the caller URI.
    pub fn from(&self) -> &str {
        &self.incoming.from
    }

    /// Return the called URI.
    pub fn to(&self) -> &str {
        &self.incoming.to
    }

    /// Answer the incoming call.
    pub async fn answer(self) -> Result<EndpointCall> {
        let handle = self.incoming.accept().await?;
        Ok(EndpointCall::new(handle, self.registrar, self.transport))
    }

    /// Alias for [`answer`](Self::answer).
    pub async fn accept(self) -> Result<EndpointCall> {
        self.answer().await
    }

    /// Defer the incoming call decision and return a guard.
    pub fn defer(self, watchdog: Duration) -> IncomingCallGuard {
        self.incoming.defer(watchdog)
    }

    /// Reject the call with 603 Decline.
    pub async fn decline(self) -> Result<()> {
        self.reject(603, "Decline").await
    }

    /// Reject the call with 486 Busy Here.
    pub async fn busy(self) -> Result<()> {
        self.reject(486, "Busy Here").await
    }

    /// Reject the call with an explicit SIP status and reason phrase.
    pub async fn reject(self, status: u16, reason: &str) -> Result<()> {
        self.incoming.reject(status, reason);
        Ok(())
    }

    /// Redirect the caller to another SIP URI with `302 Moved Temporarily`.
    pub async fn redirect_to(self, target: impl Into<String>) -> Result<()> {
        self.incoming.redirect_to(target).await
    }

    /// Redirect the caller with an explicit 3xx status and Contact list.
    pub async fn redirect_with_contacts<I, S>(self, status: u16, contacts: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.incoming.redirect_with_contacts(status, contacts).await
    }
}

impl fmt::Debug for EndpointIncomingCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EndpointIncomingCall")
            .field("id", &self.id().to_string())
            .field("from", &self.from())
            .field("to", &self.to())
            .finish()
    }
}

/// Bidirectional endpoint audio stream for a call.
pub struct EndpointAudio {
    stream: AudioStream,
}

impl EndpointAudio {
    fn new(stream: AudioStream) -> Self {
        Self { stream }
    }

    /// Split the audio stream into sender and receiver halves.
    pub fn split(self) -> (EndpointAudioSender, EndpointAudioReceiver) {
        let (sender, receiver) = self.stream.split();
        (
            EndpointAudioSender { sender },
            EndpointAudioReceiver { receiver },
        )
    }
}

/// Send half of endpoint call audio.
#[derive(Clone)]
pub struct EndpointAudioSender {
    sender: AudioSender,
}

impl EndpointAudioSender {
    /// Send one audio frame to the remote party.
    pub async fn send(&self, frame: EndpointAudioFrame) -> Result<()> {
        self.sender.send(frame.into()).await
    }

    /// Return whether the underlying audio channel is open.
    pub fn is_open(&self) -> bool {
        self.sender.is_open()
    }
}

/// Receive half of endpoint call audio.
pub struct EndpointAudioReceiver {
    receiver: AudioReceiver,
}

impl EndpointAudioReceiver {
    /// Wait for the next audio frame from the remote party.
    pub async fn recv(&mut self) -> Option<EndpointAudioFrame> {
        self.receiver.recv().await.map(EndpointAudioFrame::from)
    }

    /// Try to receive an audio frame without blocking.
    pub fn try_recv(&mut self) -> Option<EndpointAudioFrame> {
        self.receiver.try_recv().map(EndpointAudioFrame::from)
    }
}

/// Mono or interleaved PCM16 audio frame used by Endpoint audio.
#[derive(Debug, Clone, Deserialize)]
pub struct EndpointAudioFrame {
    /// PCM16 samples, interleaved when channels is greater than one.
    pub samples: Vec<i16>,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Number of channels.
    pub channels: u8,
    /// RTP-style timestamp.
    pub timestamp: u32,
}

impl EndpointAudioFrame {
    /// Create a new endpoint audio frame.
    pub fn new(samples: Vec<i16>, sample_rate: u32, channels: u8, timestamp: u32) -> Self {
        Self {
            samples,
            sample_rate,
            channels,
            timestamp,
        }
    }

    /// Create a 20 ms, 8 kHz mono PCM16 frame.
    pub fn pcmu_sized_mono_8khz(samples: Vec<i16>, timestamp: u32) -> Self {
        Self::new(samples, 8_000, 1, timestamp)
    }

    /// Return samples per channel.
    pub fn samples_per_channel(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }
}

impl From<EndpointAudioFrame> for rvoip_media_core::types::AudioFrame {
    fn from(frame: EndpointAudioFrame) -> Self {
        rvoip_media_core::types::AudioFrame::new(
            frame.samples,
            frame.sample_rate,
            frame.channels,
            frame.timestamp,
        )
    }
}

impl From<rvoip_media_core::types::AudioFrame> for EndpointAudioFrame {
    fn from(frame: rvoip_media_core::types::AudioFrame) -> Self {
        Self {
            samples: frame.samples,
            sample_rate: frame.sample_rate,
            channels: frame.channels,
            timestamp: frame.timestamp,
        }
    }
}

/// Registration state exposed by Endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointRegistrationStatus {
    /// REGISTER is in progress.
    Registering,
    /// The registrar accepted the binding.
    Registered,
    /// Unregister is in progress.
    Unregistering,
    /// No active binding is known.
    Unregistered,
    /// The most recent registration operation failed.
    Failed,
}

/// Registration lifecycle snapshot exposed by Endpoint.
#[derive(Debug, Clone)]
pub struct EndpointRegistrationInfo {
    /// Coarse registration status.
    pub status: EndpointRegistrationStatus,
    /// Registrar URI.
    pub registrar: Option<String>,
    /// Contact URI currently registered.
    pub contact: Option<String>,
    /// Requested expiry.
    pub expires_secs: Option<u32>,
    /// Registrar-accepted expiry.
    pub accepted_expires_secs: Option<u32>,
    /// Duration until the next automatic refresh.
    pub next_refresh_in: Option<Duration>,
    /// Retry count for the current or last registration flow.
    pub retry_count: u32,
    /// Last failure, if any.
    pub last_failure: Option<String>,
}

impl From<RegistrationInfo> for EndpointRegistrationInfo {
    fn from(info: RegistrationInfo) -> Self {
        Self {
            status: match info.status {
                RegistrationStatus::Registering => EndpointRegistrationStatus::Registering,
                RegistrationStatus::Registered => EndpointRegistrationStatus::Registered,
                RegistrationStatus::Unregistering => EndpointRegistrationStatus::Unregistering,
                RegistrationStatus::Unregistered => EndpointRegistrationStatus::Unregistered,
                RegistrationStatus::Failed => EndpointRegistrationStatus::Failed,
            },
            registrar: info.registrar,
            contact: info.contact,
            expires_secs: info.expires_secs,
            accepted_expires_secs: info.accepted_expires_secs,
            next_refresh_in: info.next_refresh_in,
            retry_count: info.retry_count,
            last_failure: info.last_failure,
        }
    }
}

/// Account information used by [`EndpointBuilder`].
///
/// `EndpointAccount` describes the SIP registrar credentials and optional
/// identity overrides. It maps directly to [`Registration`] plus the default
/// INVITE digest credentials stored on [`Config`].
#[derive(Debug, Clone)]
pub struct EndpointAccount {
    /// SIP URI of the registrar, for example `sip:pbx.example.com` or
    /// `sips:pbx.example.com:5061`.
    pub registrar: String,
    /// Address-of-record user, usually the extension or SIP username.
    pub username: String,
    /// Optional digest-auth username when it differs from [`username`](Self::username).
    pub auth_username: Option<String>,
    /// Digest-auth password.
    pub password: String,
    /// Registration expiry in seconds.
    pub expires: u32,
    /// Optional From/AoR URI override.
    pub from_uri: Option<String>,
    /// Optional Contact URI override.
    pub contact_uri: Option<String>,
}

impl EndpointAccount {
    /// Create a complete endpoint account.
    ///
    /// # Examples
    ///
    /// ```
    /// let account = rvoip_sip::EndpointAccount::new(
    ///     "sip:pbx.example.com",
    ///     "1001",
    ///     "secret",
    /// );
    /// assert_eq!(account.expires, 3600);
    /// ```
    pub fn new(
        registrar: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            registrar: registrar.into(),
            username: username.into(),
            auth_username: None,
            password: password.into(),
            expires: 3600,
            from_uri: None,
            contact_uri: None,
        }
    }

    /// Set the digest-auth username.
    pub fn auth_username(mut self, username: impl Into<String>) -> Self {
        self.auth_username = Some(username.into());
        self
    }

    /// Set the registration expiry in seconds.
    pub fn expires(mut self, seconds: u32) -> Self {
        self.expires = seconds;
        self
    }

    /// Override the SIP From/AoR URI.
    pub fn from_uri(mut self, uri: impl Into<String>) -> Self {
        self.from_uri = Some(uri.into());
        self
    }

    /// Override the SIP Contact URI.
    pub fn contact_uri(mut self, uri: impl Into<String>) -> Self {
        self.contact_uri = Some(uri.into());
        self
    }
}

/// Serde-friendly endpoint configuration for CLI tools and simple apps.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EndpointConfig {
    /// Display/configuration name.
    pub name: Option<String>,
    /// Deployment profile shortcut.
    pub profile: Option<EndpointProfileName>,
    /// Top-level bind shortcut.
    pub bind: Option<SocketAddr>,
    /// Top-level advertised SIP address shortcut.
    pub advertise: Option<SocketAddr>,
    /// SIP account configuration.
    pub account: Option<EndpointAccountConfig>,
    /// Network and signalling settings.
    pub network: Option<EndpointNetworkConfig>,
    /// Media settings.
    pub media: Option<EndpointMediaConfig>,
    /// SIP trace diagnostics.
    pub sip_trace: Option<crate::api::events::SipTraceConfig>,
    /// Whether an application should register immediately after startup.
    pub register_on_start: Option<bool>,
}

/// Serde-friendly SIP account settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EndpointAccountConfig {
    /// SIP registrar URI.
    pub registrar: String,
    /// SIP username or extension.
    pub username: String,
    /// Optional digest username when it differs from username.
    pub auth_username: Option<String>,
    /// Digest password.
    pub password: String,
    /// Registration expiry in seconds.
    pub expires: Option<u32>,
    /// Optional From/AoR URI override.
    pub from_uri: Option<String>,
    /// Optional Contact URI override.
    pub contact_uri: Option<String>,
}

impl TryFrom<EndpointAccountConfig> for EndpointAccount {
    type Error = SessionError;

    fn try_from(config: EndpointAccountConfig) -> Result<Self> {
        let mut account = EndpointAccount::new(config.registrar, config.username, config.password);
        if let Some(auth_username) = config.auth_username {
            account = account.auth_username(auth_username);
        }
        if let Some(expires) = config.expires {
            account = account.expires(expires);
        }
        if let Some(from_uri) = config.from_uri {
            account = account.from_uri(from_uri);
        }
        if let Some(contact_uri) = config.contact_uri {
            account = account.contact_uri(contact_uri);
        }
        Ok(account)
    }
}

/// Serde-friendly network and signalling settings.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EndpointNetworkConfig {
    /// SIP bind address.
    pub bind: Option<SocketAddr>,
    /// Advertised SIP address.
    pub advertise: Option<SocketAddr>,
    /// Preferred signalling transport.
    pub transport: Option<EndpointTransport>,
    /// STUN server for media public-address discovery.
    pub stun: Option<String>,
    /// Outbound proxy URI.
    pub outbound_proxy: Option<String>,
    /// SIP instance URN for registered-flow profiles.
    pub sip_instance: Option<String>,
    /// TLS listener bind address.
    pub tls_bind: Option<SocketAddr>,
    /// TLS certificate path.
    pub tls_cert_path: Option<PathBuf>,
    /// TLS private key path.
    pub tls_key_path: Option<PathBuf>,
}

/// Serde-friendly media settings.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EndpointMediaConfig {
    /// Public media address as an IP address or socket address string.
    pub public_address: Option<String>,
    /// RTP media port range start.
    pub port_start: Option<u16>,
    /// RTP media port range end.
    pub port_end: Option<u16>,
    /// SRTP negotiation policy.
    pub srtp: Option<EndpointSrtpMode>,
}

/// Serde-friendly deployment profile names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EndpointProfileName {
    /// Local loopback development.
    Local,
    /// Directly reachable LAN/PBX endpoint.
    LanPbx,
    /// UDP Asterisk/PBX endpoint.
    AsteriskUdp,
    /// Asterisk TLS and mandatory SRTP registered flow.
    AsteriskTlsSrtp,
    /// FreeSWITCH internal profile.
    FreeswitchInternal,
    /// FreeSWITCH TLS and SRTP reachable-contact profile.
    FreeswitchTlsSrtp,
    /// Carrier/SBC profile.
    CarrierSbc,
}

impl From<EndpointProfileName> for EndpointProfile {
    fn from(profile: EndpointProfileName) -> Self {
        match profile {
            EndpointProfileName::Local => EndpointProfile::Local,
            EndpointProfileName::LanPbx => EndpointProfile::LanPbx,
            EndpointProfileName::AsteriskUdp => EndpointProfile::AsteriskUdp,
            EndpointProfileName::AsteriskTlsSrtp => EndpointProfile::AsteriskTlsSrtpRegisteredFlow,
            EndpointProfileName::FreeswitchInternal => EndpointProfile::FreeSwitchInternal,
            EndpointProfileName::FreeswitchTlsSrtp => {
                EndpointProfile::FreeSwitchTlsSrtpReachableContact
            }
            EndpointProfileName::CarrierSbc => EndpointProfile::CarrierSbc,
        }
    }
}

/// Preferred signalling transport for endpoint-generated SIP URIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EndpointTransport {
    /// UDP signalling.
    Udp,
    /// TCP signalling.
    Tcp,
    /// TLS signalling with `sips:` targets.
    Tls,
}

/// SRTP policy for endpoint media negotiation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EndpointSrtpMode {
    /// Do not offer SRTP.
    Off,
    /// Offer SRTP but allow RTP fallback.
    Offer,
    /// Require SRTP.
    Required,
}

/// Deployment profile used by [`EndpointBuilder`].
///
/// These variants intentionally mirror the existing [`Config`] profile
/// constructors so `Endpoint` remains a convenience layer, not a second SIP
/// configuration system.
#[derive(Debug, Clone)]
pub enum EndpointProfile {
    /// Local loopback development profile.
    Local,
    /// Directly reachable LAN PBX endpoint.
    LanPbx,
    /// UDP Asterisk/PBX endpoint profile.
    AsteriskUdp,
    /// Asterisk TLS + mandatory SDES-SRTP with symmetric registered-flow reuse.
    AsteriskTlsSrtpRegisteredFlow,
    /// FreeSWITCH/Sofia internal LAN profile.
    FreeSwitchInternal,
    /// FreeSWITCH TLS + mandatory SDES-SRTP with a directly reachable TLS Contact.
    FreeSwitchTlsSrtpReachableContact,
    /// Carrier/SBC style TLS registered-flow operation with outbound proxy.
    CarrierSbc,
    /// Fully custom config; builder account and registration conveniences still apply.
    Custom(Config),
}

impl Default for EndpointProfile {
    fn default() -> Self {
        Self::Local
    }
}

/// Builder for [`Endpoint`].
///
/// The builder first selects a deployment profile, then applies account,
/// registration, media-port, and custom configuration overrides before
/// starting the wrapped [`StreamPeer`].
pub struct EndpointBuilder {
    name: Option<String>,
    profile: EndpointProfile,
    bind_addr: Option<SocketAddr>,
    advertised_addr: Option<SocketAddr>,
    tls_bind_addr: Option<SocketAddr>,
    tls_cert_path: Option<std::path::PathBuf>,
    tls_key_path: Option<std::path::PathBuf>,
    media_port_start: Option<u16>,
    media_port_end: Option<u16>,
    media_public_addr: Option<SocketAddr>,
    stun_server: Option<String>,
    outbound_proxy_uri: Option<String>,
    sip_instance: Option<String>,
    transport: EndpointTransport,
    srtp_mode: Option<EndpointSrtpMode>,
    account_username: Option<String>,
    auth_username: Option<String>,
    password: Option<String>,
    registrar: Option<String>,
    expires: u32,
    sip_trace: Option<crate::api::events::SipTraceConfig>,
    from_uri: Option<String>,
    contact_uri: Option<String>,
    configurators: Vec<Box<dyn FnOnce(&mut Config) + Send>>,
}

impl EndpointBuilder {
    /// Create a builder with the local profile.
    pub fn new() -> Self {
        Self {
            name: None,
            profile: EndpointProfile::Local,
            bind_addr: None,
            advertised_addr: None,
            tls_bind_addr: None,
            tls_cert_path: None,
            tls_key_path: None,
            media_port_start: None,
            media_port_end: None,
            media_public_addr: None,
            stun_server: None,
            outbound_proxy_uri: None,
            sip_instance: None,
            transport: EndpointTransport::Udp,
            srtp_mode: None,
            account_username: None,
            auth_username: None,
            password: None,
            registrar: None,
            expires: 3600,
            sip_trace: None,
            from_uri: None,
            contact_uri: None,
            configurators: Vec::new(),
        }
    }

    /// Create a builder from a serde-friendly endpoint configuration object.
    pub fn from_config(config: EndpointConfig) -> Result<Self> {
        let mut builder = EndpointBuilder::new();

        if let Some(name) = config.name {
            builder = builder.name(name);
        }
        if let Some(profile) = config.profile {
            builder = builder.profile(profile.into());
        }
        if let Some(bind) = config.bind.or(config.network.as_ref().and_then(|n| n.bind)) {
            builder = builder.bind_addr(bind);
        }
        if let Some(advertise) = config
            .advertise
            .or(config.network.as_ref().and_then(|n| n.advertise))
        {
            builder = builder.advertised_addr(advertise);
        }

        if let Some(account) = config.account {
            builder = builder.endpoint_account(account.try_into()?);
        }

        if let Some(network) = config.network {
            if let Some(transport) = network.transport {
                builder = builder.transport(transport);
            }
            if let Some(stun) = network.stun {
                builder = builder.stun_server(stun);
            }
            if let Some(proxy) = network.outbound_proxy {
                builder = builder.outbound_proxy(proxy);
            }
            if let Some(instance) = network.sip_instance {
                builder = builder.sip_instance(instance);
            }
            if let Some(tls_bind) = network.tls_bind {
                builder = builder.tls_bind_addr(tls_bind);
            }
            if let Some(path) = network.tls_cert_path {
                builder = builder.tls_cert_path(path);
            }
            if let Some(path) = network.tls_key_path {
                builder = builder.tls_key_path(path);
            }
        }

        if let Some(media) = config.media {
            if let Some(public) = media.public_address {
                builder = builder.media_public_addr(parse_media_public_address(&public)?);
            }
            if let Some(start) = media.port_start {
                let end = media.port_end.unwrap_or(start);
                builder = builder.media_ports(start, end);
            } else if let Some(end) = media.port_end {
                builder = builder.media_ports(10_000, end);
            }
            if let Some(srtp) = media.srtp {
                builder = builder.srtp(srtp);
            }
        }

        if let Some(sip_trace) = config.sip_trace {
            builder = builder.sip_trace(sip_trace);
        }

        Ok(builder)
    }

    /// Set the display/configuration name.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the SIP account username or extension.
    pub fn account(mut self, username: impl Into<String>) -> Self {
        self.account_username = Some(username.into());
        self
    }

    /// Set all account fields at once.
    pub fn endpoint_account(mut self, account: EndpointAccount) -> Self {
        self.registrar = Some(account.registrar);
        self.account_username = Some(account.username);
        self.auth_username = account.auth_username;
        self.password = Some(account.password);
        self.expires = account.expires;
        self.from_uri = account.from_uri;
        self.contact_uri = account.contact_uri;
        self
    }

    /// Set the digest-auth username when it differs from the account username.
    pub fn auth_username(mut self, username: impl Into<String>) -> Self {
        self.auth_username = Some(username.into());
        self
    }

    /// Set the digest-auth password.
    pub fn password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }

    /// Set the SIP registrar URI.
    pub fn registrar(mut self, registrar: impl Into<String>) -> Self {
        self.registrar = Some(registrar.into());
        self
    }

    /// Set the registration expiry in seconds.
    pub fn expires(mut self, seconds: u32) -> Self {
        self.expires = seconds;
        self
    }

    /// Select a deployment profile.
    pub fn profile(mut self, profile: EndpointProfile) -> Self {
        self.profile = profile;
        self
    }

    /// Apply a serde-friendly configuration object to this builder.
    pub fn config(self, config: EndpointConfig) -> Result<Self> {
        let mut configured = EndpointBuilder::from_config(config)?;
        if self.name.is_some() {
            configured.name = self.name;
        }
        if self.sip_trace.is_some() {
            configured.sip_trace = self.sip_trace;
        }
        Ok(configured)
    }

    /// Set the SIP bind address.
    pub fn bind_addr(mut self, addr: SocketAddr) -> Self {
        self.bind_addr = Some(addr);
        self
    }

    /// Set the SIP advertised/public address.
    pub fn advertised_addr(mut self, addr: SocketAddr) -> Self {
        self.advertised_addr = Some(addr);
        self
    }

    /// Set the SIP TLS listener bind address.
    pub fn tls_bind_addr(mut self, addr: SocketAddr) -> Self {
        self.tls_bind_addr = Some(addr);
        self
    }

    /// Set the TLS listener certificate path.
    pub fn tls_cert_path(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.tls_cert_path = Some(path.into());
        self
    }

    /// Set the TLS listener private-key path.
    pub fn tls_key_path(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.tls_key_path = Some(path.into());
        self
    }

    /// Set the RTP media port range.
    pub fn media_ports(mut self, start: u16, end: u16) -> Self {
        self.media_port_start = Some(start);
        self.media_port_end = Some(end);
        self
    }

    /// Set the public RTP media address advertised in SDP.
    pub fn media_public_addr(mut self, addr: SocketAddr) -> Self {
        self.media_public_addr = Some(addr);
        self
    }

    /// Set a public RTP media IP address, leaving the negotiated media port dynamic.
    pub fn media_public_ip(mut self, addr: IpAddr) -> Self {
        self.media_public_addr = Some(SocketAddr::new(addr, 0));
        self
    }

    /// Set a STUN server for best-effort media public-address discovery.
    pub fn stun_server(mut self, server: impl Into<String>) -> Self {
        self.stun_server = Some(server.into());
        self
    }

    /// Set an outbound proxy URI for carrier/SBC-style operation.
    pub fn outbound_proxy(mut self, uri: impl Into<String>) -> Self {
        self.outbound_proxy_uri = Some(uri.into());
        self
    }

    /// Set the RFC 5626 SIP instance URN used by registered-flow profiles.
    pub fn sip_instance(mut self, urn: impl Into<String>) -> Self {
        self.sip_instance = Some(urn.into());
        self
    }

    /// Set the preferred signalling transport for generated SIP URIs.
    pub fn transport(mut self, transport: EndpointTransport) -> Self {
        self.transport = transport;
        self
    }

    /// Set the SRTP offer policy.
    pub fn srtp(mut self, mode: EndpointSrtpMode) -> Self {
        self.srtp_mode = Some(mode);
        self
    }

    /// Enable SIP transport-boundary tracing with default redaction.
    pub fn enable_sip_trace(mut self) -> Self {
        self.sip_trace = Some(crate::api::events::SipTraceConfig::enabled());
        self
    }

    /// Set SIP transport-boundary trace policy.
    pub fn sip_trace(mut self, config: crate::api::events::SipTraceConfig) -> Self {
        self.sip_trace = Some(config);
        self
    }

    /// Override the From/AoR URI used for registration and outgoing calls.
    pub fn from_uri(mut self, uri: impl Into<String>) -> Self {
        self.from_uri = Some(uri.into());
        self
    }

    /// Override the Contact URI used for registration and dialog Contact generation.
    pub fn contact_uri(mut self, uri: impl Into<String>) -> Self {
        self.contact_uri = Some(uri.into());
        self
    }

    /// Mutate the generated [`Config`] immediately before the endpoint starts.
    pub fn configure(mut self, f: impl FnOnce(&mut Config) + Send + 'static) -> Self {
        self.configurators.push(Box::new(f));
        self
    }

    /// Build and start the endpoint.
    pub async fn build(self) -> Result<Endpoint> {
        let parts = self.build_parts()?;
        let peer = StreamPeer::with_config(parts.config).await?;
        Ok(Endpoint {
            peer,
            registration: parts.registration,
            registration_handle: Arc::new(Mutex::new(None)),
            registrar: parts.registrar,
            transport: parts.transport,
        })
    }

    fn build_parts(self) -> Result<EndpointParts> {
        let mut config = self.profile_config()?;
        let registrar = self
            .registrar
            .clone()
            .map(|uri| apply_transport_to_uri(&uri, self.transport, true));
        let account_username = self.account_username.clone();

        if let (Some(username), Some(password)) = (&account_username, &self.password) {
            let auth_username = self.auth_username.as_deref().unwrap_or(username);
            config.credentials = Some(Credentials::new(auth_username, password));
        }

        if let Some(start) = self.media_port_start {
            config.media_port_start = start;
        }
        if let Some(end) = self.media_port_end {
            config.media_port_end = end;
        }
        if let Some(addr) = self.media_public_addr {
            config.media_public_addr = Some(addr);
        }
        if let Some(stun) = self.stun_server {
            config.stun_server = Some(stun);
        }
        if let Some(outbound_proxy) = self.outbound_proxy_uri.as_ref() {
            config.outbound_proxy_uri =
                Some(apply_transport_to_uri(outbound_proxy, self.transport, true));
        }
        if let Some(srtp_mode) = self.srtp_mode {
            match srtp_mode {
                EndpointSrtpMode::Off => {
                    config.offer_srtp = false;
                    config.srtp_required = false;
                }
                EndpointSrtpMode::Offer => {
                    config.offer_srtp = true;
                    config.srtp_required = false;
                }
                EndpointSrtpMode::Required => {
                    config.offer_srtp = true;
                    config.srtp_required = true;
                }
            }
        }
        if let Some(sip_trace) = self.sip_trace {
            config.sip_trace = sip_trace;
        }
        if self.transport == EndpointTransport::Tls && config.sip_tls_mode == SipTlsMode::Disabled {
            config.sip_tls_mode = SipTlsMode::ClientOnly;
        }

        let derived_from_uri = match (&self.from_uri, &account_username, &registrar) {
            (Some(uri), _, _) => Some(uri.clone()),
            (None, Some(username), Some(registrar)) => Some(account_aor_uri(registrar, username)?),
            _ => None,
        };
        if let Some(from_uri) = &derived_from_uri {
            config.local_uri = from_uri.clone();
        }

        if let Some(contact_uri) = &self.contact_uri {
            config.contact_uri = Some(contact_uri.clone());
        }

        for configure in self.configurators {
            configure(&mut config);
        }

        let registration = match (
            registrar.as_ref(),
            account_username.as_ref(),
            self.password.as_ref(),
        ) {
            (Some(registrar), Some(username), Some(password)) => {
                let auth_username = self.auth_username.as_deref().unwrap_or(username);
                let mut registration = Registration::new(
                    registrar.clone(),
                    auth_username.to_string(),
                    password.clone(),
                )
                .expires(self.expires);
                if let Some(from_uri) = derived_from_uri {
                    registration = registration.from_uri(from_uri);
                }
                if let Some(contact_uri) = self.contact_uri {
                    registration = registration.contact_uri(contact_uri);
                }
                Some(registration)
            }
            _ => None,
        };

        Ok(EndpointParts {
            config,
            registration,
            registrar,
            transport: self.transport,
        })
    }

    fn profile_config(&self) -> Result<Config> {
        let name = self
            .name
            .as_deref()
            .or(self.account_username.as_deref())
            .unwrap_or("endpoint");

        match &self.profile {
            EndpointProfile::Local => {
                let bind = self
                    .bind_addr
                    .unwrap_or_else(|| SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 5060));
                if bind.ip().is_loopback() {
                    Ok(Config::local(name, bind.port()))
                } else {
                    let mut config = Config::on(name, bind.ip(), bind.port());
                    config.bind_addr = bind;
                    Ok(config)
                }
            }
            EndpointProfile::LanPbx => {
                let bind = self.bind_addr.unwrap_or_else(default_udp_bind);
                let advertised = self.advertised_addr.ok_or_else(|| {
                    SessionError::ConfigError(
                        "EndpointProfile::LanPbx requires advertised_addr".to_string(),
                    )
                })?;
                Ok(Config::lan_pbx(name, bind, advertised))
            }
            EndpointProfile::AsteriskUdp => {
                let bind = self.bind_addr.unwrap_or_else(default_udp_bind);
                if let Some(advertised) = self.advertised_addr {
                    Ok(Config::lan_pbx(name, bind, advertised))
                } else if bind.ip().is_loopback() {
                    let mut config = Config::local(name, bind.port());
                    config.bind_addr = bind;
                    Ok(config)
                } else if bind.ip().is_unspecified() {
                    Err(SessionError::ConfigError(
                        "EndpointProfile::AsteriskUdp with an unspecified bind address requires advertised_addr"
                            .to_string(),
                    ))
                } else {
                    let mut config = Config::on(name, bind.ip(), bind.port());
                    config.bind_addr = bind;
                    Ok(config)
                }
            }
            EndpointProfile::AsteriskTlsSrtpRegisteredFlow => {
                let bind = self.bind_addr.unwrap_or_else(default_tls_bind);
                Ok(Config::asterisk_tls_registered_flow(
                    name,
                    bind,
                    self.sip_instance
                        .clone()
                        .unwrap_or_else(generate_sip_instance),
                ))
            }
            EndpointProfile::FreeSwitchInternal => {
                let bind = self.bind_addr.unwrap_or_else(default_udp_bind);
                Ok(Config::freeswitch_internal(name, bind))
            }
            EndpointProfile::FreeSwitchTlsSrtpReachableContact => {
                let bind = self.bind_addr.unwrap_or_else(default_udp_bind);
                let tls_bind = self.tls_bind_addr.unwrap_or_else(default_tls_bind);
                let cert = self.tls_cert_path.clone().ok_or_else(|| {
                    SessionError::ConfigError(
                        "EndpointProfile::FreeSwitchTlsSrtpReachableContact requires tls_cert_path"
                            .to_string(),
                    )
                })?;
                let key = self.tls_key_path.clone().ok_or_else(|| {
                    SessionError::ConfigError(
                        "EndpointProfile::FreeSwitchTlsSrtpReachableContact requires tls_key_path"
                            .to_string(),
                    )
                })?;
                Ok(Config::freeswitch_tls_srtp_reachable_contact(
                    name, bind, tls_bind, cert, key,
                ))
            }
            EndpointProfile::CarrierSbc => {
                let bind = self.bind_addr.unwrap_or_else(default_tls_bind);
                let public = self.advertised_addr.ok_or_else(|| {
                    SessionError::ConfigError(
                        "EndpointProfile::CarrierSbc requires advertised_addr".to_string(),
                    )
                })?;
                let outbound_proxy = self.outbound_proxy_uri.clone().ok_or_else(|| {
                    SessionError::ConfigError(
                        "EndpointProfile::CarrierSbc requires outbound_proxy".to_string(),
                    )
                })?;
                Ok(Config::carrier_sbc(
                    name,
                    bind,
                    public,
                    outbound_proxy,
                    self.sip_instance
                        .clone()
                        .unwrap_or_else(generate_sip_instance),
                ))
            }
            EndpointProfile::Custom(config) => Ok(config.clone()),
        }
    }
}

impl Default for EndpointBuilder {
    fn default() -> Self {
        Self::new()
    }
}

struct EndpointParts {
    config: Config,
    registration: Option<Registration>,
    registrar: Option<String>,
    transport: EndpointTransport,
}

fn default_udp_bind() -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 5060)
}

fn default_tls_bind() -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 5061)
}

fn generate_sip_instance() -> String {
    format!("urn:uuid:{}", uuid::Uuid::new_v4())
}

async fn wait_for_registration_result(
    events: &mut EndpointEvents,
    handle: &RegistrationHandle,
    timeout: Option<Duration>,
) -> Result<EndpointRegistrationInfo> {
    let coordinator = events.control.coordinator().clone();
    let registrar = coordinator
        .registration_info(handle)
        .await?
        .registrar
        .unwrap_or_default();
    let fut = async {
        loop {
            match events.next().await? {
                Some(EndpointEvent::RegistrationChanged(info))
                    if registrar.is_empty()
                        || info.registrar.as_deref() == Some(registrar.as_str()) =>
                {
                    if info.status == EndpointRegistrationStatus::Registered {
                        return coordinator
                            .registration_info(handle)
                            .await
                            .map(EndpointRegistrationInfo::from);
                    }
                    if info.status == EndpointRegistrationStatus::Failed {
                        return Err(SessionError::Other(format!(
                            "registration failed for {}: {}",
                            info.registrar.unwrap_or_default(),
                            info.last_failure
                                .unwrap_or_else(|| "unknown error".to_string())
                        )));
                    }
                }
                Some(_) => {}
                None => {
                    return Err(SessionError::Other(
                        "event stream closed while waiting for registration".to_string(),
                    ))
                }
            }
        }
    };

    match timeout {
        Some(duration) => tokio::time::timeout(duration, fut)
            .await
            .map_err(|_| SessionError::Timeout("register_and_wait timed out".to_string()))?,
        None => fut.await,
    }
}

fn normalize_target(
    registrar: Option<&str>,
    target: &str,
    transport: EndpointTransport,
) -> Result<String> {
    let target = target.trim();
    if target.is_empty() {
        return Err(SessionError::InvalidInput(
            "call target must not be empty".to_string(),
        ));
    }

    let lower = target.to_ascii_lowercase();
    if lower.starts_with("sip:") || lower.starts_with("sips:") || lower.starts_with("tel:") {
        return Ok(apply_transport_to_uri(target, transport, false));
    }

    let registrar = registrar.ok_or_else(|| {
        SessionError::ConfigError(
            "bare call targets require EndpointBuilder::registrar".to_string(),
        )
    })?;
    let registrar = apply_transport_to_uri(registrar, transport, true);
    let mut registrar_uri = parse_uri(&registrar, "registrar")?;

    if target.contains('@') {
        return Ok(format!("{}:{}", registrar_uri.scheme, target));
    }

    registrar_uri.user = Some(target.to_string());
    registrar_uri.password = None;
    registrar_uri.headers.clear();
    Ok(registrar_uri.to_string())
}

fn apply_transport_to_uri(
    uri: &str,
    transport: EndpointTransport,
    registrar_or_proxy: bool,
) -> String {
    match transport {
        EndpointTransport::Udp => uri.to_string(),
        EndpointTransport::Tcp => {
            if uri.contains(";transport=") {
                uri.to_string()
            } else {
                format!("{uri};transport=tcp")
            }
        }
        EndpointTransport::Tls => {
            let tls_uri = if uri.to_ascii_lowercase().starts_with("sip:") {
                format!("sips:{}", &uri[4..])
            } else {
                uri.to_string()
            };
            if registrar_or_proxy || tls_uri.contains(";transport=") {
                tls_uri
            } else {
                format!("{tls_uri};transport=tls")
            }
        }
    }
}

fn parse_media_public_address(value: &str) -> Result<SocketAddr> {
    if let Ok(addr) = value.parse::<SocketAddr>() {
        return Ok(addr);
    }
    let ip = value.parse::<IpAddr>().map_err(|err| {
        SessionError::InvalidInput(format!("invalid media public address '{value}': {err}"))
    })?;
    Ok(SocketAddr::new(ip, 0))
}

fn account_aor_uri(registrar: &str, username: &str) -> Result<String> {
    let mut uri = parse_uri(registrar, "registrar")?;
    uri.user = Some(username.to_string());
    uri.password = None;
    uri.port = None;
    uri.parameters.clear();
    uri.headers.clear();
    Ok(uri.to_string())
}

fn parse_uri(value: &str, label: &str) -> Result<Uri> {
    let uri = Uri::from_str(value).map_err(|err| {
        SessionError::InvalidInput(format!("invalid {label} URI '{value}': {err}"))
    })?;
    match uri.scheme {
        Scheme::Sip | Scheme::Sips => Ok(uri),
        _ => Err(SessionError::InvalidInput(format!(
            "{label} URI must use sip: or sips:"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::unified::{SipContactMode, SipTlsMode};

    #[test]
    fn endpoint_builder_maps_asterisk_tls_profile() {
        let parts = Endpoint::builder()
            .name("alice")
            .account("1001")
            .password("secret")
            .registrar("sips:pbx.example.test:5061;transport=tls")
            .profile(EndpointProfile::AsteriskTlsSrtpRegisteredFlow)
            .sip_instance("urn:uuid:00000000-0000-0000-0000-000000000001")
            .build_parts()
            .unwrap();

        assert_eq!(parts.config.sip_tls_mode, SipTlsMode::ClientOnly);
        assert_eq!(
            parts.config.sip_contact_mode,
            SipContactMode::RegisteredFlowSymmetric
        );
        assert!(parts.config.offer_srtp);
        assert!(parts.config.srtp_required);
        assert_eq!(parts.config.local_uri, "sips:1001@pbx.example.test");
        assert!(parts.registration.is_some());
    }

    #[test]
    fn endpoint_builder_creates_registration_defaults() {
        let parts = Endpoint::builder()
            .account("1001")
            .auth_username("auth1001")
            .password("secret")
            .registrar("sip:pbx.example.test")
            .contact_uri("sip:1001@192.0.2.10:5060")
            .expires(600)
            .build_parts()
            .unwrap();

        let registration = parts.registration.unwrap();
        assert_eq!(registration.registrar, "sip:pbx.example.test");
        assert_eq!(registration.username, "auth1001");
        assert_eq!(registration.password, "secret");
        assert_eq!(registration.expires, 600);
        assert_eq!(
            registration.from_uri.as_deref(),
            Some("sip:1001@pbx.example.test")
        );
        assert_eq!(
            registration.contact_uri.as_deref(),
            Some("sip:1001@192.0.2.10:5060")
        );
    }

    #[test]
    fn endpoint_normalizes_bare_extension_through_registrar() {
        let target = normalize_target(
            Some("sips:pbx.example.test:5061;transport=tls"),
            "1002",
            EndpointTransport::Udp,
        )
        .unwrap();
        assert_eq!(target, "sips:1002@pbx.example.test:5061;transport=tls");
    }

    #[test]
    fn endpoint_leaves_full_sip_uri_unchanged() {
        let target = normalize_target(
            Some("sips:pbx.example.test:5061"),
            "sip:bob@example.test",
            EndpointTransport::Udp,
        )
        .unwrap();
        assert_eq!(target, "sip:bob@example.test");
    }

    #[test]
    fn endpoint_requires_registrar_for_bare_target() {
        let err = normalize_target(None, "1002", EndpointTransport::Udp).unwrap_err();
        assert!(err.to_string().contains("registrar"));
    }

    #[test]
    fn endpoint_transport_rewrites_tls_target() {
        let target = normalize_target(
            Some("sip:pbx.example.test:5060"),
            "1002",
            EndpointTransport::Tls,
        )
        .unwrap();
        assert_eq!(target, "sips:1002@pbx.example.test:5060");
    }

    #[test]
    fn endpoint_json_config_maps_builder_fields() {
        let config = serde_json::from_str::<EndpointConfig>(
            r#"{
                "name": "alice",
                "profile": "asterisk-udp",
                "account": {
                    "username": "1001",
                    "password": "secret",
                    "registrar": "sip:pbx.example.test"
                },
                "network": {
                    "bind": "127.0.0.1:5060",
                    "transport": "tcp",
                    "stun": "stun.example.test:3478"
                },
                "media": {
                    "publicAddress": "192.0.2.10",
                    "srtp": "offer"
                }
            }"#,
        )
        .unwrap();

        let parts = EndpointBuilder::from_config(config)
            .unwrap()
            .build_parts()
            .unwrap();
        assert_eq!(parts.transport, EndpointTransport::Tcp);
        assert_eq!(
            parts.config.stun_server.as_deref(),
            Some("stun.example.test:3478")
        );
        assert!(parts.config.offer_srtp);
        assert!(!parts.config.srtp_required);
        assert_eq!(
            parts.config.media_public_addr,
            Some("192.0.2.10:0".parse().unwrap())
        );
        assert_eq!(
            parts.registrar.as_deref(),
            Some("sip:pbx.example.test;transport=tcp")
        );
    }

    #[test]
    fn endpoint_json_config_accepts_partial_sip_trace_config() {
        let config = serde_json::from_str::<EndpointConfig>(
            r#"{
                "sipTrace": {
                    "enabled": true,
                    "redactSensitiveHeaders": false
                }
            }"#,
        )
        .unwrap();

        let trace = config.sip_trace.unwrap();
        assert!(trace.enabled);
        assert_eq!(
            trace.capacity,
            crate::api::events::SipTraceConfig::DEFAULT_CAPACITY
        );
        assert!(!trace.redact_sensitive_headers);
        assert!(trace.include_body);
    }

    #[test]
    fn sip_client_example_stays_on_endpoint_surface() {
        let source = [
            include_str!("../../examples/sip_client/main.rs"),
            include_str!("../../examples/sip_client/audio.rs"),
            include_str!("../../examples/sip_client/config.rs"),
            include_str!("../../examples/sip_client/runtime.rs"),
            include_str!("../../examples/sip_client/smoke.rs"),
            include_str!("../../examples/sip_client/ui.rs"),
        ]
        .join("\n");
        for banned in [
            "StreamPeer",
            "PeerControl",
            "UnifiedCoordinator",
            "RegistrationHandle",
            "SessionHandle",
            "SipTlsMode",
            "rvoip_media_core",
        ] {
            assert!(
                !source.contains(banned),
                "sip_client example must not reference lower-level API {banned}"
            );
        }
    }

    #[test]
    fn endpoint_audio_roundtrip_stays_on_endpoint_surface() {
        let source = include_str!("../../examples/endpoint/04_audio_roundtrip/main.rs");
        for banned in [
            "StreamPeer",
            "PeerControl",
            "UnifiedCoordinator",
            "RegistrationHandle",
            "SessionHandle",
            "as_session_handle",
            "rvoip_media_core",
        ] {
            assert!(
                !source.contains(banned),
                "endpoint audio roundtrip example must not reference lower-level API {banned}"
            );
        }
    }
}
