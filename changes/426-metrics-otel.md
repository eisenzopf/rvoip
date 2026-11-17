---
section: Breaking changes
breaking: true
---
**metrics 0.23 → 0.24 and OpenTelemetry 0.32 → 0.33.** rvoip emits into
whatever metrics recorder and OpenTelemetry global provider the
application installs, and those are keyed to the crate's semver line.
Migration: install a metrics 0.24 recorder (metrics-exporter-prometheus
0.18) and an OpenTelemetry 0.33 provider (tracing-opentelemetry 0.34), or
rvoip's metrics and spans silently disappear.
