//! Compiler-checked consumer fixture for the supported rvoip-sip API.
//!
//! Keep these imports explicit. A glob import would continue compiling after
//! a re-export disappeared and therefore would not protect downstream users.

#![allow(dead_code, unused_imports)]

use std::future::Future;

use rvoip_sip::api::{
    AcceptBuilder, AuthChallengeBuilder, AuthScheme, ByeBuilder, CallDecision, CallSession,
    CancelBuilder, DialogIdentity, GenericResponseBuilder, InDialogResponseBuilder, InfoBuilder,
    MediaInfo, MessageBuilder, MissingRequiredHeader, NotifyBuilder, OptionsBuilder,
    OutboundCallBuilder, ProvisionalBuilder, ReInviteBuilder, RedirectBuilder, ReferBuilder,
    RegisterBuilder, RegisterRefreshBuilder, RegisterResponseBuilder, RejectBuilder, Role, SdpInfo,
    SessionBuilder, SessionStats, SubscribeBuilder, SubscribeRefreshBuilder, Surface,
    SurfaceBuilder, UpdateBuilder,
};
use rvoip_sip::prelude::{
    Config as PreludeConfig, Endpoint as PreludeEndpoint, SessionHandle as PreludeSessionHandle,
    StreamPeer as PreludeStreamPeer,
};
use rvoip_sip::state_machine::executor::{PendingOptionsSlot, ProcessEventResult};
use rvoip_sip::state_table::EventType;
use rvoip_sip::{
    AudioFrame, AudioReceiver, AudioSender, AudioStream, AutoAnswerHandler, BodyRedactionDecision,
    BridgeError, BridgeHandle, BuilderHeaderState, BuilderStrictness, CallAnsweredInfo,
    CallAuthRetryDetails, CallHandler, CallHandlerDecision, CallId, CallLifecycleSnapshot,
    CallProgressInfo, CallState, CallTerminalInfo, CallbackPeer, CallbackPeerBuilder,
    CallbackPeerControl, ClosureHandler, Config, DefaultTraceRedactor, DiagnosticEvent, DialogInfo,
    DialogInfoDocument, DialogPackageEvent, DialogPackageState, DialogSubscriptionHandle,
    EndReason, Endpoint, EndpointAccount, EndpointAccountConfig, EndpointAudio, EndpointAudioFrame,
    EndpointAudioReceiver, EndpointAudioSender, EndpointBuilder, EndpointCall, EndpointCallId,
    EndpointConfig, EndpointControl, EndpointEvent, EndpointEvents, EndpointIncomingCall,
    EndpointMediaConfig, EndpointNetworkConfig, EndpointProfile, EndpointProfileName,
    EndpointRegistrationInfo, EndpointRegistrationStatus, EndpointSipTrace, EndpointSrtpMode,
    EndpointTransport, Event, EventReceiver, HeaderCarryThroughReport, HeaderName,
    HeaderPolicyViolation, IncomingCall, IncomingCallGuard, IncomingRegister, IncomingRequest,
    IncomingResponse, MediaMode, MediaPoolConfig, MediaSecurityKeying, MediaSecurityProfile,
    MediaSecurityState, MediaSessionControllerConfig, PassthroughRedactor, PeerControl,
    PerformanceConfig, PerformanceRecipe, PerformanceRecipeBook, ProfiledSipAdapter, QueueHandler,
    RedactionDecision, Registration, RegistrationHandle, RegistrationInfo, RegistrationStatus,
    RejectAllHandler, Result, RoutingAction, RoutingHandler, RoutingRule, RtpSessionBufferConfig,
    RtpTransportBufferConfig, SessionError, SessionHandle, SessionId, ShutdownHandle, SipAccount,
    SipContactMode, SipEgressProfilePolicy, SipEgressProfileRegistration, SipHeaderView,
    SipInitialHeaders, SipOriginateContext, SipProfileRevision, SipProfileSrtpPolicy, SipReason,
    SipRequestOptions, SipRuntimeConfig, SipTlsMode, SipTrace, SipTraceConfig, SipTraceDirection,
    SrtpSuitePolicy, StreamPeer, StreamPeerBuilder, SubscriptionState, SymmetricRtpPolicy,
    TraceRedactor, TransferDialogMatcher, TransferKind, TransferLifecycleOptions, TransferOutcome,
    TransferTargetEvidence, TransferWaitMode, TypedHeader, UnifiedCoordinator, ViolationReason,
};

fn assert_future_output<T>(_: impl Future<Output = T>) {}

// These bodies are type-checked without being called. They pin the two public
// compatibility-facade signatures that the cleanup must preserve.
fn assert_outbound_facade_signatures(
    coordinator: &UnifiedCoordinator,
    session_id: &SessionId,
    slot: PendingOptionsSlot,
    event: EventType,
) {
    assert_future_output::<Result<()>>(coordinator.stage_outbound_options(session_id, slot));
    assert_future_output::<Result<ProcessEventResult>>(
        coordinator.dispatch_outbound(session_id, event),
    );
}

fn assert_primary_constructor_signatures(config: Config) {
    assert_future_output::<Result<std::sync::Arc<UnifiedCoordinator>>>(UnifiedCoordinator::new(
        config.clone(),
    ));
    assert_future_output::<Result<StreamPeer>>(StreamPeer::with_config(config));
    let _: fn() -> EndpointBuilder = Endpoint::builder;
}

fn assert_call_session_facade_signatures(call: &CallSession) {
    assert_future_output::<Result<()>>(call.start_recording());
    assert_future_output::<Result<()>>(call.stop_recording());
    assert_future_output::<Result<()>>(call.play_audio("fixture.wav"));
    assert_future_output::<Result<()>>(call.start_media());
}

fn assert_config_compatibility_fields(config: &Config) {
    let _: &Option<String> = &config.state_table_path;
    let _: &String = &config.local_uri;
    let _: &std::net::IpAddr = &config.local_ip;
    let _: &std::net::SocketAddr = &config.bind_addr;
    let _: &u16 = &config.sip_port;
    let _: &u16 = &config.media_port_start;
    let _: &u16 = &config.media_port_end;
}

#[test]
fn crate_root_api_and_prelude_reexports_resolve() {
    // Explicit aliases above prove the paths resolve. Type names make the
    // imports observably used without constructing network-facing values.
    assert!(!std::any::type_name::<Config>().is_empty());
    assert_eq!(
        std::any::type_name::<PreludeConfig>(),
        std::any::type_name::<Config>()
    );
    assert_eq!(
        std::any::type_name::<PreludeEndpoint>(),
        std::any::type_name::<Endpoint>()
    );
    assert_eq!(
        std::any::type_name::<PreludeSessionHandle>(),
        std::any::type_name::<SessionHandle>()
    );
    assert_eq!(
        std::any::type_name::<PreludeStreamPeer>(),
        std::any::type_name::<StreamPeer>()
    );
}

#[test]
fn public_event_compatibility_variants_remain_available() {
    fn accept_event(event: Event) -> Event {
        match event {
            Event::CallProgress { .. } | Event::CallFailed { .. } | Event::CallAnswered { .. } => {
                event
            }
            Event::CallProgressDetailed(_)
            | Event::CallEstablishedDetailed(_)
            | Event::CallFailedDetailed(_) => event,
            other => other,
        }
    }
    let _ = accept_event;
}

#[test]
fn additive_runtime_and_diagnostic_surfaces_remain_available() {
    let runtime =
        SipRuntimeConfig::default().with_sdes_base64_mode(rvoip_sip::SdesBase64Mode::Strict);
    assert_eq!(
        runtime.sdes_base64_mode(),
        rvoip_sip::SdesBase64Mode::Strict
    );

    fn diagnostic_receiver(
        coordinator: &UnifiedCoordinator,
    ) -> tokio::sync::broadcast::Receiver<DiagnosticEvent> {
        coordinator.subscribe_diagnostics()
    }
    let _ = diagnostic_receiver;
    let _ = std::any::type_name::<CallAuthRetryDetails>();
}
