use crate::errors::types::{Error, Result};
use std::str::FromStr;
use tracing::Level;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::{fmt, EnvFilter};

/// Configuration for the logging system
#[derive(Debug, Clone)]
pub struct LoggingConfig {
    /// The log level to use
    pub level: Level,
    /// Whether to enable JSON formatting
    pub json: bool,
    /// Whether to include file and line information
    pub file_info: bool,
    /// Whether to log spans
    pub log_spans: bool,
    /// Application name to include in logs
    pub app_name: String,
    /// P12.7 — when `Some(endpoint)` AND the `otel` feature is on,
    /// install an OpenTelemetry OTLP layer that exports tracing spans
    /// to the given collector endpoint (e.g.
    /// `"http://localhost:4318"`). When the feature is off, the
    /// endpoint is silently ignored — keeps the surface stable across
    /// builds. PRD §10.2 / INTERFACE_DESIGN §5.
    pub otel_endpoint: Option<String>,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        LoggingConfig {
            level: Level::INFO,
            json: false,
            file_info: false,
            log_spans: false,
            app_name: "rvoip".to_string(),
            otel_endpoint: None,
        }
    }
}

impl LoggingConfig {
    /// Create a new logging configuration
    pub fn new(level: Level, app_name: impl Into<String>) -> Self {
        LoggingConfig {
            level,
            app_name: app_name.into(),
            ..Default::default()
        }
    }

    /// Enable JSON formatting
    pub fn with_json(mut self) -> Self {
        self.json = true;
        self
    }

    /// Enable file and line information in logs
    pub fn with_file_info(mut self) -> Self {
        self.file_info = true;
        self
    }

    /// Enable span logging
    pub fn with_spans(mut self) -> Self {
        self.log_spans = true;
        self
    }

    /// P12.7 — set the OTLP collector endpoint. Only effective when
    /// the `otel` feature is compiled in.
    pub fn with_otel_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.otel_endpoint = Some(endpoint.into());
        self
    }
}

/// Set up the logging system with the provided configuration. When
/// `LoggingConfig.otel_endpoint` is `Some(_)` and the `otel` feature
/// is enabled, an OpenTelemetry OTLP exporter layer is added
/// alongside the local subscriber so spans flow to both. Without
/// the feature, the endpoint is silently dropped — same call shape
/// works in any build profile.
pub fn setup_logging(config: LoggingConfig) -> Result<()> {
    let filter = EnvFilter::from_default_env().add_directive(config.level.into());

    let span_events = if config.log_spans {
        FmtSpan::ACTIVE
    } else {
        FmtSpan::NONE
    };

    #[cfg(feature = "otel")]
    {
        if let Some(endpoint) = config.otel_endpoint.as_ref() {
            return setup_logging_with_otel(filter, span_events, &config, endpoint);
        }
    }

    let mut subscriber = fmt::Subscriber::builder()
        .with_env_filter(filter)
        .with_span_events(span_events);

    if config.file_info {
        subscriber = subscriber.with_file(true).with_line_number(true);
    }

    if config.json {
        // Setup JSON formatting
        subscriber.with_writer(std::io::stdout).json().init();
    } else {
        subscriber.init();
    }

    Ok(())
}

/// P12.7 — `setup_logging` path that wires an OTLP exporter layer.
/// Only compiled with the `otel` feature.
#[cfg(feature = "otel")]
fn setup_logging_with_otel(
    filter: EnvFilter,
    span_events: FmtSpan,
    config: &LoggingConfig,
    endpoint: &str,
) -> Result<()> {
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_otlp::WithExportConfig;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(endpoint)
        .build()
        .map_err(|e| Error::Config(format!("OTLP exporter init failed: {}", e)))?;

    let provider = opentelemetry_sdk::trace::TracerProvider::builder()
        .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
        .build();
    let tracer = provider.tracer(config.app_name.clone());

    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    let fmt_layer = fmt::layer()
        .with_span_events(span_events)
        .with_file(config.file_info)
        .with_line_number(config.file_info);

    let registry = tracing_subscriber::registry()
        .with(filter)
        .with(otel_layer)
        .with(fmt_layer);
    registry.try_init().map_err(|e| {
        Error::Config(format!("tracing subscriber init failed: {}", e))
    })?;

    // Hand provider ownership to a global so spans flush at shutdown
    // (the user can re-fetch via opentelemetry::global::tracer_provider).
    opentelemetry::global::set_tracer_provider(provider);
    Ok(())
}

/// Parse a log level from a string
pub fn parse_log_level(level: &str) -> Result<Level> {
    Level::from_str(level).map_err(|_| Error::Config(format!("Invalid log level: {}", level)))
}

/// Log a welcome message with version info
pub fn log_welcome(app_name: &str, version: &str) {
    tracing::info!("Starting {} v{}", app_name, version);
}
