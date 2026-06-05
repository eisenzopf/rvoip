# rvoip-audit

Optional audit sink implementations for `rvoip-auth-core::AuthAuditSink`.

Current sinks:

- `JsonLinesAuditSink`: appends redacted auth events to a JSON-lines file;
- `TracingAuditSink`: emits redacted events through `tracing`;
- `OtlpAuditSink`: exports OpenTelemetry Logs over OTLP/HTTP JSON;
- `SiemAuditSink`: exports vendor-shaped JSON for generic webhooks, Splunk
  HEC, Elastic/ECS, Microsoft Sentinel, and Datadog Logs;
- `FanoutAuditSink`: sends each event to multiple sinks.

These sinks do not receive passwords, bearer tokens, API keys, full JWTs,
Authorization headers, or SIP Digest HA1 values. They serialize the redacted
`AuthAuditEvent` shape supplied by protocol/auth services.

Example:

```rust,no_run
use std::sync::Arc;
use rvoip_audit::{FanoutAuditSink, OtlpAuditSink, SiemAuditSink};

# fn build_sink() -> Result<FanoutAuditSink, Box<dyn std::error::Error>> {
let otlp = OtlpAuditSink::new("https://collector.example.com/v1/logs")?
    .with_service_name("rvoip-sbc")
    .with_bearer_token("collector-token")?;
let splunk = SiemAuditSink::splunk_hec(
    "https://splunk.example.com:8088/services/collector/event",
    "splunk-hec-token",
)?;

let sink = FanoutAuditSink::new()
    .with_sink(Arc::new(otlp))
    .with_sink(Arc::new(splunk));
# Ok(sink)
# }
```
