//! Audit sink implementations for RVoIP authentication events.
//!
//! This crate is optional. Protocol crates emit redacted
//! `rvoip-auth-core::AuthAuditEvent` values through the `AuthAuditSink` trait;
//! applications choose where those events go.

use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION};
use rvoip_auth_core::{AuthAuditEvent, AuthAuditSink, CredentialAuthError};
use serde::Serialize;
use serde_json::{json, Value};
use thiserror::Error;
use tokio::fs::{File, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

/// Error returned while constructing audit sinks.
#[derive(Debug, Error)]
pub enum AuditSinkError {
    /// File I/O failed.
    #[error("audit sink I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// HTTP/exporter configuration was invalid.
    #[error("audit sink configuration error: {0}")]
    Config(String),
}

/// JSON-lines audit event envelope.
#[derive(Debug, Clone, Serialize)]
pub struct JsonAuditEvent<'a> {
    /// Event timestamp as milliseconds since Unix epoch.
    pub timestamp_unix_ms: u128,
    /// Redacted auth event.
    pub event: &'a AuthAuditEvent,
}

/// Append-only JSON-lines audit sink.
///
/// Each line is a [`JsonAuditEvent`] containing a timestamp and the redacted
/// `AuthAuditEvent`. The sink flushes after each write so crash recovery loses
/// less audit state at the cost of throughput.
pub struct JsonLinesAuditSink {
    file: Mutex<File>,
}

impl JsonLinesAuditSink {
    /// Open or create a JSON-lines audit file.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, AuditSinkError> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await?;
        Ok(Self {
            file: Mutex::new(file),
        })
    }
}

#[async_trait]
impl AuthAuditSink for JsonLinesAuditSink {
    async fn record_auth_event(&self, event: AuthAuditEvent) -> Result<(), CredentialAuthError> {
        let envelope = JsonAuditEvent {
            timestamp_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis())
                .unwrap_or_default(),
            event: &event,
        };
        let mut line = serde_json::to_vec(&envelope)
            .map_err(|err| CredentialAuthError::Unavailable(err.to_string()))?;
        line.push(b'\n');

        let mut file = self.file.lock().await;
        file.write_all(&line)
            .await
            .map_err(|err| CredentialAuthError::Unavailable(err.to_string()))?;
        file.flush()
            .await
            .map_err(|err| CredentialAuthError::Unavailable(err.to_string()))
    }
}

/// Tracing log level for auth audit events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TracingAuditLevel {
    /// Emit events at `debug`.
    Debug,
    /// Emit events at `info`.
    Info,
    /// Emit events at `warn`.
    Warn,
}

/// Audit sink that emits redacted auth events through `tracing`.
#[derive(Debug, Clone)]
pub struct TracingAuditSink {
    level: TracingAuditLevel,
}

impl TracingAuditSink {
    /// Create a tracing audit sink at `info` level.
    pub fn new() -> Self {
        Self {
            level: TracingAuditLevel::Info,
        }
    }

    /// Create a tracing audit sink with an explicit level.
    pub fn with_level(level: TracingAuditLevel) -> Self {
        Self { level }
    }
}

impl Default for TracingAuditSink {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AuthAuditSink for TracingAuditSink {
    async fn record_auth_event(&self, event: AuthAuditEvent) -> Result<(), CredentialAuthError> {
        match self.level {
            TracingAuditLevel::Debug => tracing::debug!(
                scheme = ?event.scheme,
                outcome = ?event.outcome,
                subject = event.subject.as_deref(),
                realm = event.realm.as_deref(),
                peer = event.peer.as_deref(),
                metadata = ?event.metadata,
                "rvoip auth audit event"
            ),
            TracingAuditLevel::Info => tracing::info!(
                scheme = ?event.scheme,
                outcome = ?event.outcome,
                subject = event.subject.as_deref(),
                realm = event.realm.as_deref(),
                peer = event.peer.as_deref(),
                metadata = ?event.metadata,
                "rvoip auth audit event"
            ),
            TracingAuditLevel::Warn => tracing::warn!(
                scheme = ?event.scheme,
                outcome = ?event.outcome,
                subject = event.subject.as_deref(),
                realm = event.realm.as_deref(),
                peer = event.peer.as_deref(),
                metadata = ?event.metadata,
                "rvoip auth audit event"
            ),
        }
        Ok(())
    }
}

/// Audit sink that forwards each redacted event to multiple sinks.
#[derive(Clone, Default)]
pub struct FanoutAuditSink {
    sinks: Vec<Arc<dyn AuthAuditSink>>,
}

impl FanoutAuditSink {
    /// Create an empty fanout sink.
    pub fn new() -> Self {
        Self { sinks: Vec::new() }
    }

    /// Add an audit sink.
    pub fn with_sink(mut self, sink: Arc<dyn AuthAuditSink>) -> Self {
        self.sinks.push(sink);
        self
    }
}

#[async_trait]
impl AuthAuditSink for FanoutAuditSink {
    async fn record_auth_event(&self, event: AuthAuditEvent) -> Result<(), CredentialAuthError> {
        for sink in &self.sinks {
            sink.record_auth_event(event.clone()).await?;
        }
        Ok(())
    }
}

/// OpenTelemetry Logs exporter for redacted auth audit events.
///
/// Sends OTLP/HTTP JSON payloads to an endpoint such as
/// `https://collector.example.com/v1/logs`. The sink only serializes
/// [`AuthAuditEvent`] fields, which are already redacted by the auth-core
/// contract.
#[derive(Clone)]
pub struct OtlpAuditSink {
    client: reqwest::Client,
    endpoint: String,
    headers: HeaderMap,
    service_name: String,
}

impl OtlpAuditSink {
    /// Create an OTLP Logs sink using `service.name = rvoip`.
    pub fn new(endpoint: impl Into<String>) -> Result<Self, AuditSinkError> {
        let endpoint = endpoint.into();
        if endpoint.trim().is_empty() {
            return Err(AuditSinkError::Config(
                "OTLP endpoint must not be empty".to_string(),
            ));
        }
        Ok(Self {
            client: reqwest::Client::new(),
            endpoint,
            headers: HeaderMap::new(),
            service_name: "rvoip".to_string(),
        })
    }

    /// Override the OTLP resource `service.name` attribute.
    pub fn with_service_name(mut self, service_name: impl Into<String>) -> Self {
        self.service_name = service_name.into();
        self
    }

    /// Add a static HTTP header, for example an API key header required by a
    /// hosted collector.
    pub fn with_header(
        mut self,
        name: impl AsRef<str>,
        value: impl AsRef<str>,
    ) -> Result<Self, AuditSinkError> {
        insert_header(&mut self.headers, name.as_ref(), value.as_ref())?;
        Ok(self)
    }

    /// Add `Authorization: Bearer <token>`.
    pub fn with_bearer_token(mut self, token: impl AsRef<str>) -> Result<Self, AuditSinkError> {
        let value = format!("Bearer {}", token.as_ref());
        self.headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&value).map_err(|err| {
                AuditSinkError::Config(format!("invalid Authorization header: {err}"))
            })?,
        );
        Ok(self)
    }
}

#[async_trait]
impl AuthAuditSink for OtlpAuditSink {
    async fn record_auth_event(&self, event: AuthAuditEvent) -> Result<(), CredentialAuthError> {
        let payload = build_otlp_payload(&self.service_name, &event);
        post_json(&self.client, &self.endpoint, &self.headers, &payload).await
    }
}

/// SIEM/webhook payload preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SiemAuditFormat {
    /// Direct JSON event envelope suitable for generic webhooks.
    GenericJson,
    /// Splunk HTTP Event Collector payload.
    SplunkHec,
    /// Elastic-compatible ECS-flavored JSON.
    ElasticEcs,
    /// Microsoft Sentinel custom-log style JSON.
    MicrosoftSentinel,
    /// Datadog Logs intake JSON.
    Datadog,
}

/// HTTP SIEM exporter for redacted auth audit events.
#[derive(Clone)]
pub struct SiemAuditSink {
    client: reqwest::Client,
    endpoint: String,
    headers: HeaderMap,
    format: SiemAuditFormat,
    source: String,
}

impl SiemAuditSink {
    /// Generic JSON webhook sink.
    pub fn generic_webhook(endpoint: impl Into<String>) -> Result<Self, AuditSinkError> {
        Self::new(endpoint, SiemAuditFormat::GenericJson)
    }

    /// Splunk HEC sink. `endpoint` should be the HEC event endpoint.
    pub fn splunk_hec(
        endpoint: impl Into<String>,
        token: impl AsRef<str>,
    ) -> Result<Self, AuditSinkError> {
        Self::new(endpoint, SiemAuditFormat::SplunkHec)?
            .with_header("Authorization", format!("Splunk {}", token.as_ref()))
    }

    /// Elastic/Elasticsearch compatible sink using ECS-like field names.
    pub fn elastic(
        endpoint: impl Into<String>,
        api_key: impl AsRef<str>,
    ) -> Result<Self, AuditSinkError> {
        Self::new(endpoint, SiemAuditFormat::ElasticEcs)?
            .with_header("Authorization", format!("ApiKey {}", api_key.as_ref()))
    }

    /// Microsoft Sentinel custom-log webhook sink.
    pub fn microsoft_sentinel(endpoint: impl Into<String>) -> Result<Self, AuditSinkError> {
        Self::new(endpoint, SiemAuditFormat::MicrosoftSentinel)
    }

    /// Datadog Logs intake sink.
    pub fn datadog(
        endpoint: impl Into<String>,
        api_key: impl AsRef<str>,
    ) -> Result<Self, AuditSinkError> {
        Self::new(endpoint, SiemAuditFormat::Datadog)?.with_header("DD-API-KEY", api_key.as_ref())
    }

    /// Create a SIEM sink with an explicit format.
    pub fn new(
        endpoint: impl Into<String>,
        format: SiemAuditFormat,
    ) -> Result<Self, AuditSinkError> {
        let endpoint = endpoint.into();
        if endpoint.trim().is_empty() {
            return Err(AuditSinkError::Config(
                "SIEM endpoint must not be empty".to_string(),
            ));
        }
        Ok(Self {
            client: reqwest::Client::new(),
            endpoint,
            headers: HeaderMap::new(),
            format,
            source: "rvoip".to_string(),
        })
    }

    /// Override the source/service label used in vendor payloads.
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = source.into();
        self
    }

    /// Add a static HTTP header.
    pub fn with_header(
        mut self,
        name: impl AsRef<str>,
        value: impl AsRef<str>,
    ) -> Result<Self, AuditSinkError> {
        insert_header(&mut self.headers, name.as_ref(), value.as_ref())?;
        Ok(self)
    }
}

#[async_trait]
impl AuthAuditSink for SiemAuditSink {
    async fn record_auth_event(&self, event: AuthAuditEvent) -> Result<(), CredentialAuthError> {
        let payload = build_siem_payload(self.format, &self.source, &event);
        post_json(&self.client, &self.endpoint, &self.headers, &payload).await
    }
}

async fn post_json(
    client: &reqwest::Client,
    endpoint: &str,
    headers: &HeaderMap,
    payload: &Value,
) -> Result<(), CredentialAuthError> {
    let response = client
        .post(endpoint)
        .headers(headers.clone())
        .json(payload)
        .send()
        .await
        .map_err(|err| CredentialAuthError::Unavailable(err.to_string()))?;
    let status = response.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(CredentialAuthError::Unavailable(format!(
            "audit exporter returned HTTP {status}"
        )))
    }
}

fn insert_header(headers: &mut HeaderMap, name: &str, value: &str) -> Result<(), AuditSinkError> {
    let name = HeaderName::from_bytes(name.as_bytes())
        .map_err(|err| AuditSinkError::Config(format!("invalid header name: {err}")))?;
    let value = HeaderValue::from_str(value)
        .map_err(|err| AuditSinkError::Config(format!("invalid header value: {err}")))?;
    headers.insert(name, value);
    Ok(())
}

fn build_otlp_payload(service_name: &str, event: &AuthAuditEvent) -> Value {
    json!({
        "resourceLogs": [{
            "resource": {
                "attributes": [
                    otlp_attr("service.name", service_name),
                    otlp_attr("rvoip.component", "auth")
                ]
            },
            "scopeLogs": [{
                "scope": {
                    "name": "rvoip-audit",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "logRecords": [{
                    "timeUnixNano": current_unix_nanos().to_string(),
                    "severityText": otlp_severity(event),
                    "body": { "stringValue": "rvoip auth audit event" },
                    "attributes": otlp_event_attributes(event)
                }]
            }]
        }]
    })
}

fn build_siem_payload(format: SiemAuditFormat, source: &str, event: &AuthAuditEvent) -> Value {
    let timestamp_ms = current_unix_ms();
    match format {
        SiemAuditFormat::GenericJson => json!({
            "timestamp_unix_ms": timestamp_ms,
            "source": source,
            "event": event
        }),
        SiemAuditFormat::SplunkHec => json!({
            "time": (timestamp_ms as f64) / 1000.0,
            "source": source,
            "sourcetype": "rvoip:auth",
            "event": event
        }),
        SiemAuditFormat::ElasticEcs => json!({
            "@timestamp_unix_ms": timestamp_ms,
            "event.kind": "event",
            "event.category": ["authentication"],
            "event.type": [event_type(event)],
            "event.outcome": event_outcome(event),
            "service.name": source,
            "rvoip.auth": event
        }),
        SiemAuditFormat::MicrosoftSentinel => json!({
            "TimeGeneratedUnixMs": timestamp_ms,
            "Vendor": "RVoIP",
            "Product": "RVoIP Auth",
            "Name": "Authentication audit event",
            "Severity": sentinel_severity(event),
            "Source": source,
            "ExtendedProperties": event
        }),
        SiemAuditFormat::Datadog => json!({
            "timestamp": timestamp_ms,
            "service": source,
            "ddsource": "rvoip",
            "status": datadog_status(event),
            "message": "rvoip auth audit event",
            "rvoip_auth": event
        }),
    }
}

fn otlp_attr(key: &str, value: &str) -> Value {
    json!({ "key": key, "value": { "stringValue": value } })
}

fn otlp_event_attributes(event: &AuthAuditEvent) -> Vec<Value> {
    let mut attrs = vec![
        otlp_attr("auth.scheme", &format!("{:?}", event.scheme)),
        otlp_attr("auth.outcome", event_outcome(event)),
    ];
    if let Some(subject) = &event.subject {
        attrs.push(otlp_attr("auth.subject", subject));
    }
    if let Some(realm) = &event.realm {
        attrs.push(otlp_attr("auth.realm", realm));
    }
    if let Some(peer) = &event.peer {
        attrs.push(otlp_attr("net.peer", peer));
    }
    for (key, value) in &event.metadata {
        attrs.push(otlp_attr(&format!("rvoip.auth.{key}"), value));
    }
    attrs
}

fn current_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn current_unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

fn event_outcome(event: &AuthAuditEvent) -> &'static str {
    match &event.outcome {
        rvoip_auth_core::AuthAuditOutcome::Success => "success",
        rvoip_auth_core::AuthAuditOutcome::Failure(_) => "failure",
    }
}

fn event_type(event: &AuthAuditEvent) -> &'static str {
    match &event.outcome {
        rvoip_auth_core::AuthAuditOutcome::Success => "info",
        rvoip_auth_core::AuthAuditOutcome::Failure(_) => "denied",
    }
}

fn otlp_severity(event: &AuthAuditEvent) -> &'static str {
    match &event.outcome {
        rvoip_auth_core::AuthAuditOutcome::Success => "INFO",
        rvoip_auth_core::AuthAuditOutcome::Failure(_) => "WARN",
    }
}

fn sentinel_severity(event: &AuthAuditEvent) -> &'static str {
    match &event.outcome {
        rvoip_auth_core::AuthAuditOutcome::Success => "Informational",
        rvoip_auth_core::AuthAuditOutcome::Failure(_) => "Medium",
    }
}

fn datadog_status(event: &AuthAuditEvent) -> &'static str {
    match &event.outcome {
        rvoip_auth_core::AuthAuditOutcome::Success => "info",
        rvoip_auth_core::AuthAuditOutcome::Failure(_) => "warn",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use rvoip_auth_core::{
        AuthAuditOutcome, AuthAuditScheme, AuthFailureReason, AuthRateLimitKind,
    };

    use super::*;

    #[tokio::test]
    async fn json_lines_sink_writes_redacted_event() {
        let path = std::env::temp_dir().join(format!(
            "rvoip-audit-{}.jsonl",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let sink = JsonLinesAuditSink::open(&path).await.unwrap();
        let mut metadata = BTreeMap::new();
        metadata.insert("method".to_string(), "REGISTER".to_string());
        let event = AuthAuditEvent {
            scheme: AuthAuditScheme::Bearer,
            outcome: AuthAuditOutcome::Failure(AuthFailureReason::TokenRevoked),
            subject: Some("jti-123".to_string()),
            realm: Some("https://idp.example.test".to_string()),
            peer: Some("198.51.100.10".to_string()),
            metadata,
        };

        sink.record_auth_event(event).await.unwrap();

        let written = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(written.contains("\"scheme\":\"Bearer\""));
        assert!(written.contains("\"TokenRevoked\""));
        assert!(written.contains("\"method\":\"REGISTER\""));
        assert!(!written.contains("Authorization"));

        let _ = tokio::fs::remove_file(path).await;
    }

    #[tokio::test]
    async fn tracing_sink_accepts_event() {
        let sink = TracingAuditSink::default();
        sink.record_auth_event(AuthAuditEvent::new(
            AuthAuditScheme::Basic,
            AuthAuditOutcome::Failure(AuthFailureReason::PolicyRejected),
        ))
        .await
        .unwrap();
    }

    #[test]
    fn otlp_payload_contains_redacted_auth_attributes() {
        let mut metadata = BTreeMap::new();
        metadata.insert("method".to_string(), "INVITE".to_string());
        let event = AuthAuditEvent {
            scheme: AuthAuditScheme::Digest,
            outcome: AuthAuditOutcome::Success,
            subject: Some("alice".to_string()),
            realm: Some("pbx.example.test".to_string()),
            peer: Some("203.0.113.20".to_string()),
            metadata,
        };

        let payload = build_otlp_payload("rvoip-test", &event);
        let rendered = serde_json::to_string(&payload).unwrap();

        assert!(rendered.contains("service.name"));
        assert!(rendered.contains("rvoip-test"));
        assert!(rendered.contains("auth.scheme"));
        assert!(rendered.contains("Digest"));
        assert!(rendered.contains("rvoip.auth.method"));
        assert!(!rendered.contains("Authorization"));
        assert!(!rendered.contains("Bearer "));
        assert!(!rendered.contains("password"));
    }

    #[test]
    fn siem_payload_presets_are_redacted_and_vendor_shaped() {
        let event = AuthAuditEvent::new(
            AuthAuditScheme::Bearer,
            AuthAuditOutcome::Failure(AuthFailureReason::TokenRevoked),
        )
        .with_subject("token-jti-123")
        .with_realm("https://issuer.example.test")
        .with_peer("198.51.100.50");

        let splunk = build_siem_payload(SiemAuditFormat::SplunkHec, "rvoip-test", &event);
        let elastic = build_siem_payload(SiemAuditFormat::ElasticEcs, "rvoip-test", &event);
        let sentinel = build_siem_payload(SiemAuditFormat::MicrosoftSentinel, "rvoip-test", &event);
        let datadog = build_siem_payload(SiemAuditFormat::Datadog, "rvoip-test", &event);

        assert_eq!(splunk["sourcetype"], "rvoip:auth");
        assert_eq!(elastic["event.category"][0], "authentication");
        assert_eq!(sentinel["Product"], "RVoIP Auth");
        assert_eq!(datadog["ddsource"], "rvoip");

        let rendered = serde_json::to_string(&json!([splunk, elastic, sentinel, datadog])).unwrap();
        assert!(rendered.contains("TokenRevoked"));
        assert!(!rendered.contains("Authorization"));
        assert!(!rendered.contains("Bearer "));
        assert!(!rendered.contains("api_key_secret"));
    }

    #[test]
    fn rate_limit_kind_is_not_an_audit_payload_requirement() {
        assert_eq!(
            format!("{:?}", AuthRateLimitKind::SipRegister),
            "SipRegister"
        );
    }
}
