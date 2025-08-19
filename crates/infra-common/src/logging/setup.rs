use crate::errors::types::{Error, Result};
use tracing::Level;
use tracing_subscriber::{fmt, EnvFilter};
use tracing_subscriber::fmt::format::FmtSpan;
use std::str::FromStr;

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
}

impl Default for LoggingConfig {
    fn default() -> Self {
        LoggingConfig {
            level: Level::INFO,
            json: false,
            file_info: false,
            log_spans: false,
            app_name: "rvoip".to_string(),
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
}

/// Set up the logging system with the provided configuration
pub fn setup_logging(config: LoggingConfig) -> Result<()> {
    let filter = EnvFilter::from_default_env()
        .add_directive(config.level.into());
    
    let span_events = if config.log_spans {
        FmtSpan::ACTIVE
    } else {
        FmtSpan::NONE
    };
    
    let mut subscriber = fmt::Subscriber::builder()
        .with_env_filter(filter)
        .with_span_events(span_events);
    
    if config.file_info {
        subscriber = subscriber.with_file(true).with_line_number(true);
    }
    
    if config.json {
        // Setup JSON formatting
        subscriber.with_writer(std::io::stdout)
            .json()
            .init();
    } else {
        subscriber.init();
    }
    
    Ok(())
}

/// Parse a log level from a string
pub fn parse_log_level(level: &str) -> Result<Level> {
    Level::from_str(level)
        .map_err(|_| Error::Config(format!("Invalid log level: {}", level)))
}

/// Log a welcome message with version info
pub fn log_welcome(app_name: &str, version: &str) {
    tracing::info!("Starting {} v{}", app_name, version);
} 