use std::fmt;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rvoip_infra_common::events::coordinator::GlobalEventCoordinator;
use rvoip_infra_common::events::cross_crate::{
    format_sip_trace_message, RvoipCrossCrateEvent, SipTraceConfig, SipTraceDirection,
    SipTraceEvent,
};
use rvoip_sip_core::Message;
use rvoip_sip_transport::transport::TransportType;

/// SIP_API_DESIGN_2 §12.4 — pluggable trace redactor hook.
///
/// The redactor takes the rendered SIP message text and returns a
/// trace-friendly variant. Consulted in
/// [`SipTraceRuntime::publish`] before the static
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
        let rendered = match &self.redactor {
            Some(redact) => redact(&raw),
            None => raw,
        };
        let original_len = rendered.len();
        let (raw_message, truncated) = format_sip_trace_message(&rendered, &self.config);
        let event = SipTraceEvent {
            owner_id: self.owner_id.clone(),
            direction,
            transport: transport_type.to_string(),
            local_addr: local_addr.to_string(),
            remote_addr: remote_addr.to_string(),
            timestamp_unix_millis: timestamp_unix_millis(),
            start_line: start_line(message),
            sip_call_id: message.call_id().map(|call_id| call_id.value().to_string()),
            session_id: None,
            raw_message,
            original_len,
            truncated,
            redacted: self.config.redact_sensitive_headers,
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

fn start_line(message: &Message) -> String {
    match message {
        Message::Request(request) => {
            format!("{} {} SIP/2.0", request.method(), request.uri())
        }
        Message::Response(response) => {
            format!(
                "SIP/2.0 {} {}",
                response.status_code(),
                response.reason_phrase()
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SipTraceRuntime;
    use rvoip_infra_common::events::config::EventCoordinatorConfig;
    use rvoip_infra_common::events::coordinator::GlobalEventCoordinator;
    use rvoip_infra_common::events::cross_crate::{
        RvoipCrossCrateEvent, SipTraceConfig, SipTraceDirection,
    };
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    use rvoip_sip_core::{Message, Method};
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
        assert_eq!(trace.start_line, "REGISTER sip:example.com SIP/2.0");
        assert_eq!(trace.sip_call_id.as_deref(), Some("trace-call"));
        assert!(trace
            .raw_message
            .contains("REGISTER sip:example.com SIP/2.0"));
    }

    #[tokio::test]
    async fn publishes_inbound_sip_trace_event() {
        let trace = publish_trace(SipTraceDirection::Inbound).await;

        assert_eq!(trace.direction, SipTraceDirection::Inbound);
        assert_eq!(trace.sip_call_id.as_deref(), Some("trace-call"));
    }

    async fn publish_trace(
        direction: SipTraceDirection,
    ) -> rvoip_infra_common::events::cross_crate::SipTraceEvent {
        let coordinator = Arc::new(
            GlobalEventCoordinator::new(EventCoordinatorConfig::monolithic())
                .await
                .unwrap(),
        );
        let mut receiver = coordinator.subscribe("transport_to_session").await.unwrap();
        let runtime =
            SipTraceRuntime::new("trace-owner".into(), SipTraceConfig::enabled(), coordinator)
                .unwrap();

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

    fn trace_message() -> Message {
        Message::Request(
            SimpleRequestBuilder::new(Method::Register, "sip:example.com")
                .unwrap()
                .from("alice", "sip:alice@example.com", Some("tag-a"))
                .to("alice", "sip:alice@example.com", None)
                .call_id("trace-call")
                .cseq(1)
                .build(),
        )
    }
}
