use std::fmt;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rvoip_infra_common::events::coordinator::GlobalEventCoordinator;
use rvoip_infra_common::events::cross_crate::{
    format_sip_trace_message, format_sip_trace_start_line, RvoipCrossCrateEvent, SipTraceConfig,
    SipTraceDirection, SipTraceEvent,
};
use rvoip_sip_core::Message;
use rvoip_sip_transport::transport::TransportType;

/// SIP_API_DESIGN_2 §12.4 — pluggable trace redactor hook.
///
/// The redactor takes the rendered SIP message text and returns a
/// trace-friendly variant. Consulted in
/// `SipTraceRuntime::publish` before the static
/// `format_sip_trace_message` transform runs, so application-specific
/// scrubs (e.g. drop `X-Customer-Token`) compose with the built-in
/// auth-header redaction.
///
/// The wire form (the bytes actually sent) is untouched.
pub type TraceRedactorFn = Arc<dyn Fn(&str) -> String + Send + Sync>;

#[derive(Clone)]
pub(crate) struct SipTraceRuntime {
    owner_id: String,
    config: SipTraceConfig,
    coordinator: Arc<GlobalEventCoordinator>,
    redactor: Option<TraceRedactorFn>,
}

impl SipTraceRuntime {
    pub(crate) fn new(
        owner_id: String,
        config: SipTraceConfig,
        coordinator: Arc<GlobalEventCoordinator>,
    ) -> Option<Arc<Self>> {
        Self::new_with_redactor(owner_id, config, coordinator, None)
    }

    pub(crate) fn new_with_redactor(
        owner_id: String,
        config: SipTraceConfig,
        coordinator: Arc<GlobalEventCoordinator>,
        redactor: Option<TraceRedactorFn>,
    ) -> Option<Arc<Self>> {
        config.enabled.then(|| {
            Arc::new(Self {
                owner_id,
                config,
                coordinator,
                redactor,
            })
        })
    }

    pub(crate) fn publish(
        &self,
        direction: SipTraceDirection,
        transport_type: TransportType,
        local_addr: SocketAddr,
        remote_addr: SocketAddr,
        message: &Message,
    ) {
        if !self.config.enabled {
            return;
        }

        let raw = String::from_utf8_lossy(&message.to_bytes()).into_owned();
        // Phase 7 — consult the configured `TraceRedactor` (if any)
        // before the static `format_sip_trace_message` pipeline. The
        // wire form is unaffected.
        let (rendered, formatting_config) = match &self.redactor {
            Some(redact) => {
                // An explicitly supplied whole-message policy is authoritative
                // for headers and body. Sanitize the independently retained raw
                // start line here, then disable only the lower static pass so it
                // cannot undo deliberate custom Keep/Redact decisions.
                let rendered = sanitize_rendered_start_line(&redact(&raw), &self.config);
                let mut formatting_config = self.config.clone();
                formatting_config.redact_sensitive_headers = false;
                (rendered, formatting_config)
            }
            // The public redactor-less API receives the complete conservative
            // static policy in `format_sip_trace_message`.
            None => (raw.clone(), self.config.clone()),
        };
        let redactor_changed_message = rendered != raw;
        let original_len = raw.len();
        let (raw_message, truncated) = format_sip_trace_message(&rendered, &formatting_config);
        // Derive every separately indexed trace surface from the already
        // redacted diagnostic representation. Re-reading the original Message
        // here would bypass a custom start-line or Call-ID decision even when
        // `raw_message` itself is safe.
        let start_line = format_sip_trace_start_line(trace_start_line(&raw_message), &self.config);
        let sip_call_id = trace_call_id(&raw_message);
        let event = SipTraceEvent {
            owner_id: self.owner_id.clone(),
            direction,
            transport: transport_type.to_string(),
            local_addr: local_addr.to_string(),
            remote_addr: remote_addr.to_string(),
            timestamp_unix_millis: timestamp_unix_millis(),
            start_line,
            sip_call_id,
            session_id: None,
            raw_message,
            original_len,
            truncated,
            redacted: self.config.redact_sensitive_headers
                || !self.config.include_body
                || redactor_changed_message,
        };
        let coordinator = self.coordinator.clone();
        tokio::spawn(async move {
            if let Err(err) = coordinator
                .publish(Arc::new(RvoipCrossCrateEvent::TransportToSession(event)))
                .await
            {
                tracing::warn!("Failed to publish SIP trace event: {}", err);
            }
        });
    }
}

fn sanitize_rendered_start_line(raw: &str, config: &SipTraceConfig) -> String {
    if !config.redact_sensitive_headers {
        return raw.to_string();
    }
    let (start_line_with_cr, remainder, newline_present) = match raw.split_once('\n') {
        Some((start_line, remainder)) => (start_line, remainder, true),
        None => (raw, "", false),
    };
    let (start_line, carriage_return) = match start_line_with_cr.strip_suffix('\r') {
        Some(start_line) => (start_line, true),
        None => (start_line_with_cr, false),
    };
    let mut sanitized = format_sip_trace_start_line(start_line, config);
    if carriage_return {
        sanitized.push('\r');
    }
    if newline_present {
        sanitized.push('\n');
        sanitized.push_str(remainder);
    }
    sanitized
}

impl fmt::Debug for SipTraceRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SipTraceRuntime")
            .field("owner_id", &self.owner_id)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

fn timestamp_unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

fn trace_start_line(rendered: &str) -> &str {
    rendered
        .split_once('\n')
        .map_or(rendered, |(start_line, _)| start_line)
        .trim_end_matches('\r')
}

fn trace_call_id(rendered: &str) -> Option<String> {
    for line in rendered.lines().skip(1) {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            break;
        }
        if matches!(line.as_bytes().first(), Some(b' ' | b'\t')) {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("call-id") || name.trim().eq_ignore_ascii_case("i") {
            let value = value.trim();
            return (!value.is_empty()).then(|| value.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{SipTraceRuntime, TraceRedactorFn};
    use crate::transaction::transport::TransportManager;
    use rvoip_infra_common::events::config::EventCoordinatorConfig;
    use rvoip_infra_common::events::coordinator::GlobalEventCoordinator;
    use rvoip_infra_common::events::cross_crate::{
        RvoipCrossCrateEvent, SipTraceConfig, SipTraceDirection,
    };
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    use rvoip_sip_core::types::headers::{HeaderName, HeaderValue, TypedHeader};
    use rvoip_sip_core::types::CallId;
    use rvoip_sip_core::{Message, Method, Response, StatusCode};
    use rvoip_sip_transport::transport::TransportType;
    use std::net::SocketAddr;
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn publishes_outbound_sip_trace_event() {
        let trace = publish_trace(SipTraceDirection::Outbound).await;

        assert_eq!(trace.owner_id, "trace-owner");
        assert_eq!(trace.direction, SipTraceDirection::Outbound);
        assert_eq!(trace.transport, "UDP");
        assert_eq!(trace.local_addr, "127.0.0.1:5060");
        assert_eq!(trace.remote_addr, "127.0.0.1:5080");
        assert_eq!(trace.start_line, "REGISTER <redacted-request-uri> SIP/2.0");
        assert_eq!(trace.sip_call_id.as_deref(), Some("trace-call"));
        assert!(trace
            .raw_message
            .contains("REGISTER <redacted-request-uri> SIP/2.0"));
    }

    #[tokio::test]
    async fn publishes_inbound_sip_trace_event() {
        let trace = publish_trace(SipTraceDirection::Inbound).await;

        assert_eq!(trace.direction, SipTraceDirection::Inbound);
        assert_eq!(trace.sip_call_id.as_deref(), Some("trace-call"));
    }

    #[tokio::test]
    async fn explicit_lower_development_override_preserves_request_target() {
        let trace = publish_trace_with_config(
            SipTraceDirection::Outbound,
            SipTraceConfig::enabled().verbatim_for_development(),
        )
        .await;

        assert_eq!(trace.start_line, "REGISTER sip:example.com SIP/2.0");
        assert!(trace
            .raw_message
            .contains("REGISTER sip:example.com SIP/2.0"));
        assert!(!trace.redacted);
    }

    #[tokio::test]
    async fn explicit_lower_development_override_preserves_response_reason() {
        let response = Response::new(StatusCode::BusyHere)
            .with_reason("visible-development-reason")
            .with_header(TypedHeader::CallId(CallId::new(
                "development-response-call",
            )));
        let trace = publish_message(
            Message::Response(response),
            SipTraceConfig::enabled().verbatim_for_development(),
            None,
        )
        .await;

        assert_eq!(trace.start_line, "SIP/2.0 486 visible-development-reason");
        assert!(trace
            .raw_message
            .starts_with("SIP/2.0 486 visible-development-reason\n"));
        assert_eq!(
            trace.sip_call_id.as_deref(),
            Some("development-response-call")
        );
        assert!(!trace.redacted);
    }

    #[tokio::test]
    async fn custom_body_redaction_sets_trace_event_redacted_flag() {
        let coordinator = Arc::new(
            GlobalEventCoordinator::new(EventCoordinatorConfig::monolithic())
                .await
                .unwrap(),
        );
        let mut receiver = coordinator.subscribe("transport_to_session").await.unwrap();
        let config = SipTraceConfig {
            enabled: true,
            redact_sensitive_headers: false,
            include_body: true,
            ..SipTraceConfig::default()
        };
        let redactor: TraceRedactorFn =
            Arc::new(|raw| raw.replace("trace-body-secret", "<redacted body>"));
        let runtime = SipTraceRuntime::new_with_redactor(
            "trace-owner".into(),
            config,
            coordinator,
            Some(redactor),
        )
        .unwrap();
        let message = Message::Request(
            SimpleRequestBuilder::new(Method::Message, "sip:example.com")
                .unwrap()
                .from("alice", "sip:alice@example.com", Some("tag-a"))
                .to("bob", "sip:bob@example.com", None)
                .call_id("trace-body-call")
                .cseq(1)
                .content_type("application/json")
                .body("trace-body-secret")
                .build(),
        );

        runtime.publish(
            SipTraceDirection::Outbound,
            TransportType::Udp,
            "127.0.0.1:5060".parse::<SocketAddr>().unwrap(),
            "127.0.0.1:5080".parse::<SocketAddr>().unwrap(),
            &message,
        );

        let event = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        let event = event
            .as_any()
            .downcast_ref::<RvoipCrossCrateEvent>()
            .unwrap();
        let RvoipCrossCrateEvent::TransportToSession(trace) = event else {
            panic!("expected transport_to_session trace event");
        };
        assert!(trace.redacted);
        assert!(trace.raw_message.contains("<redacted body>"));
        assert!(!trace.raw_message.contains("trace-body-secret"));
        assert!(trace.original_len > trace.raw_message.len());
    }

    #[tokio::test]
    async fn custom_runtime_policy_keeps_explicit_headers_and_body_without_target_leakage() {
        let coordinator = Arc::new(
            GlobalEventCoordinator::new(EventCoordinatorConfig::monolithic())
                .await
                .unwrap(),
        );
        let mut receiver = coordinator.subscribe("transport_to_session").await.unwrap();
        let config = SipTraceConfig {
            enabled: true,
            redact_sensitive_headers: true,
            include_body: true,
            ..SipTraceConfig::default()
        };
        let redactor: TraceRedactorFn =
            Arc::new(|raw| raw.replace("custom-auth-secret", "<custom-auth-redacted>"));
        let runtime = SipTraceRuntime::new_with_redactor(
            "custom-policy-owner".into(),
            config,
            coordinator,
            Some(redactor),
        )
        .unwrap();
        let message = Message::Request(
            SimpleRequestBuilder::new(
                Method::Message,
                "sip:custom-uri-secret@example.test;opaque=custom-param-secret",
            )
            .unwrap()
            .from("alice", "sip:alice@example.com", Some("tag-a"))
            .to("bob", "sip:bob@example.com", None)
            .call_id("custom-policy-call")
            .cseq(1)
            .header(TypedHeader::Other(
                HeaderName::Authorization,
                HeaderValue::Raw(b"Bearer custom-auth-secret".to_vec()),
            ))
            .header(TypedHeader::Other(
                HeaderName::Other("X-Custom-Visible".into()),
                HeaderValue::Raw(b"explicit-visible-header".to_vec()),
            ))
            .content_type("text/plain")
            .body("explicit-visible-body")
            .build(),
        );

        runtime.publish(
            SipTraceDirection::Outbound,
            TransportType::Udp,
            "127.0.0.1:5060".parse::<SocketAddr>().unwrap(),
            "127.0.0.1:5080".parse::<SocketAddr>().unwrap(),
            &message,
        );

        let event = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        let event = event
            .as_any()
            .downcast_ref::<RvoipCrossCrateEvent>()
            .unwrap();
        let RvoipCrossCrateEvent::TransportToSession(trace) = event else {
            panic!("expected transport_to_session trace event");
        };
        assert_eq!(trace.start_line, "MESSAGE <redacted-request-uri> SIP/2.0");
        assert!(trace
            .raw_message
            .starts_with("MESSAGE <redacted-request-uri> SIP/2.0"));
        assert!(trace
            .raw_message
            .contains("Authorization: Bearer <custom-auth-redacted>"));
        assert!(trace
            .raw_message
            .contains("X-Custom-Visible: explicit-visible-header"));
        assert!(trace.raw_message.ends_with("explicit-visible-body"));
        for secret in [
            "custom-uri-secret",
            "custom-param-secret",
            "custom-auth-secret",
        ] {
            assert!(!trace.raw_message.contains(secret), "trace leaked {secret}");
            assert!(
                !trace.start_line.contains(secret),
                "start line leaked {secret}"
            );
        }
    }

    #[tokio::test]
    async fn public_lower_level_trace_api_is_safe_without_a_redactor() {
        let coordinator = Arc::new(
            GlobalEventCoordinator::new(EventCoordinatorConfig::monolithic())
                .await
                .unwrap(),
        );
        let mut receiver = coordinator.subscribe("transport_to_session").await.unwrap();
        let (mut manager, _transport_events) = TransportManager::with_defaults().await.unwrap();
        manager.enable_sip_trace(
            "lower-public-trace".into(),
            SipTraceConfig::enabled(),
            coordinator,
        );
        let runtime = manager
            .sip_trace_runtime()
            .expect("public API installs trace runtime");
        let message = Message::Request(
            SimpleRequestBuilder::new(
                Method::Message,
                "sip:uri-user:uri-password@example.test;opaque=uri-param-secret?X-Token=uri-query-secret",
            )
            .unwrap()
            .from("alice", "sip:alice@example.com", Some("tag-a"))
            .to("bob", "sip:bob@example.com", None)
            .call_id("operational-call-id")
            .cseq(1)
            .header(TypedHeader::Other(
                HeaderName::Authorization,
                HeaderValue::Raw(
                    b"Digest first-auth-secret\r\n\tsecond-auth-secret".to_vec(),
                ),
            ))
            .header(TypedHeader::Other(
                HeaderName::Other("X-Bridgefu-Context".into()),
                HeaderValue::Raw(b"application-header-secret".to_vec()),
            ))
            .content_type("application/json")
            .body("{\"token\":\"application-body-secret\"}")
            .build(),
        );

        runtime.publish(
            SipTraceDirection::Outbound,
            TransportType::Udp,
            "127.0.0.1:5060".parse::<SocketAddr>().unwrap(),
            "127.0.0.1:5080".parse::<SocketAddr>().unwrap(),
            &message,
        );

        let event = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        let event = event
            .as_any()
            .downcast_ref::<RvoipCrossCrateEvent>()
            .unwrap();
        let RvoipCrossCrateEvent::TransportToSession(trace) = event else {
            panic!("expected transport_to_session trace event");
        };
        assert_eq!(trace.start_line, "MESSAGE <redacted-request-uri> SIP/2.0");
        assert!(trace.redacted);
        assert!(trace.raw_message.contains("Authorization: <redacted>"));
        assert!(trace.raw_message.contains("X-Bridgefu-Context: <redacted>"));
        assert!(trace.raw_message.ends_with("<redacted body>"));
        for secret in [
            "uri-user",
            "uri-password",
            "uri-param-secret",
            "uri-query-secret",
            "first-auth-secret",
            "second-auth-secret",
            "application-header-secret",
            "application-body-secret",
        ] {
            assert!(
                !trace.start_line.contains(secret) && !trace.raw_message.contains(secret),
                "public trace API leaked {secret}: {trace:?}"
            );
        }
    }

    #[tokio::test]
    async fn lower_safe_runtime_retains_response_status_but_not_reason_in_any_event_surface() {
        let response = Response::new(StatusCode::BusyHere)
            .with_reason("private-upstream-response-reason")
            .with_header(TypedHeader::CallId(CallId::new("response-call")));
        let trace =
            publish_message(Message::Response(response), SipTraceConfig::enabled(), None).await;

        assert_eq!(trace.start_line, "SIP/2.0 486 <redacted-reason>");
        assert!(trace
            .raw_message
            .starts_with("SIP/2.0 486 <redacted-reason>\n"));
        assert_eq!(trace.sip_call_id.as_deref(), Some("response-call"));

        let serialized = serde_json::to_string(&trace).unwrap();
        let debug = format!("{trace:?}");
        for surface in [&trace.raw_message, &trace.start_line, &serialized, &debug] {
            assert!(!surface.contains("private-upstream-response-reason"));
            assert!(surface.contains("486"));
        }

        let custom_response = Response::new(StatusCode::BusyHere)
            .with_reason("custom-policy-original-reason")
            .with_header(TypedHeader::CallId(CallId::new("custom-response-call")));
        let identity_redactor: TraceRedactorFn = Arc::new(str::to_string);
        let custom_trace = publish_message(
            Message::Response(custom_response),
            SipTraceConfig::enabled(),
            Some(identity_redactor),
        )
        .await;
        assert_eq!(custom_trace.start_line, "SIP/2.0 486 <redacted-reason>");
        assert!(custom_trace
            .raw_message
            .starts_with("SIP/2.0 486 <redacted-reason>\n"));
        assert!(!format!("{custom_trace:?}").contains("custom-policy-original-reason"));
    }

    #[tokio::test]
    async fn custom_call_id_decision_controls_separate_serialized_and_debug_event_field() {
        let private_call_id = "private-event-call-id";
        let redactor: TraceRedactorFn = Arc::new(move |raw| {
            raw.replace(
                &format!("Call-ID: {private_call_id}"),
                "Call-ID: <call-id-redacted>",
            )
        });
        let trace = publish_message(
            trace_message_with_call_id(private_call_id),
            SipTraceConfig::enabled(),
            Some(redactor),
        )
        .await;

        assert_eq!(trace.sip_call_id.as_deref(), Some("<call-id-redacted>"));
        assert!(trace.raw_message.contains("Call-ID: <call-id-redacted>"));
        let serialized = serde_json::to_string(&trace).unwrap();
        let debug = format!("{trace:?}");
        assert!(!serialized.contains(private_call_id));
        assert!(!debug.contains(private_call_id));

        let drop_redactor: TraceRedactorFn =
            Arc::new(move |raw| raw.replace(&format!("Call-ID: {private_call_id}\r\n"), ""));
        let dropped = publish_message(
            trace_message_with_call_id(private_call_id),
            SipTraceConfig::enabled(),
            Some(drop_redactor),
        )
        .await;
        assert_eq!(dropped.sip_call_id, None);
        assert!(!dropped.raw_message.contains(private_call_id));
        assert!(!serde_json::to_string(&dropped)
            .unwrap()
            .contains(private_call_id));
        assert!(!format!("{dropped:?}").contains(private_call_id));
    }

    async fn publish_trace(
        direction: SipTraceDirection,
    ) -> rvoip_infra_common::events::cross_crate::SipTraceEvent {
        publish_trace_with_config(direction, SipTraceConfig::enabled()).await
    }

    async fn publish_trace_with_config(
        direction: SipTraceDirection,
        config: SipTraceConfig,
    ) -> rvoip_infra_common::events::cross_crate::SipTraceEvent {
        let coordinator = Arc::new(
            GlobalEventCoordinator::new(EventCoordinatorConfig::monolithic())
                .await
                .unwrap(),
        );
        let mut receiver = coordinator.subscribe("transport_to_session").await.unwrap();
        let runtime = SipTraceRuntime::new("trace-owner".into(), config, coordinator).unwrap();

        runtime.publish(
            direction,
            TransportType::Udp,
            "127.0.0.1:5060".parse::<SocketAddr>().unwrap(),
            "127.0.0.1:5080".parse::<SocketAddr>().unwrap(),
            &trace_message(),
        );

        let event = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        let event = event
            .as_any()
            .downcast_ref::<RvoipCrossCrateEvent>()
            .unwrap();
        let RvoipCrossCrateEvent::TransportToSession(trace) = event else {
            panic!("expected transport_to_session trace event");
        };
        trace.clone()
    }

    async fn publish_message(
        message: Message,
        config: SipTraceConfig,
        redactor: Option<TraceRedactorFn>,
    ) -> rvoip_infra_common::events::cross_crate::SipTraceEvent {
        let coordinator = Arc::new(
            GlobalEventCoordinator::new(EventCoordinatorConfig::monolithic())
                .await
                .unwrap(),
        );
        let mut receiver = coordinator.subscribe("transport_to_session").await.unwrap();
        let runtime = SipTraceRuntime::new_with_redactor(
            "surface-owner".into(),
            config,
            coordinator,
            redactor,
        )
        .unwrap();
        runtime.publish(
            SipTraceDirection::Inbound,
            TransportType::Tcp,
            "127.0.0.1:5060".parse::<SocketAddr>().unwrap(),
            "127.0.0.1:5080".parse::<SocketAddr>().unwrap(),
            &message,
        );
        let event = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        let event = event
            .as_any()
            .downcast_ref::<RvoipCrossCrateEvent>()
            .unwrap();
        let RvoipCrossCrateEvent::TransportToSession(trace) = event else {
            panic!("expected transport_to_session trace event");
        };
        trace.clone()
    }

    fn trace_message() -> Message {
        trace_message_with_call_id("trace-call")
    }

    fn trace_message_with_call_id(call_id: &str) -> Message {
        Message::Request(
            SimpleRequestBuilder::new(Method::Register, "sip:example.com")
                .unwrap()
                .from("alice", "sip:alice@example.com", Some("tag-a"))
                .to("alice", "sip:alice@example.com", None)
                .call_id(call_id)
                .cseq(1)
                .build(),
        )
    }
}
