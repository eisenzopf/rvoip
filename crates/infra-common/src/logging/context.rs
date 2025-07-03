use std::collections::HashMap;
use std::fmt;
use tracing::{Level, Span};

/// Context information for logging
#[derive(Debug, Clone)]
pub struct LogContext {
    /// Component that is generating the log
    pub component: String,
    /// Operation or action being performed
    pub operation: Option<String>,
    /// Additional contextual fields
    pub fields: HashMap<String, String>,
}

impl LogContext {
    /// Create a new log context with just the component name
    pub fn new<S: Into<String>>(component: S) -> Self {
        LogContext {
            component: component.into(),
            operation: None,
            fields: HashMap::new(),
        }
    }
    
    /// Create a new log context with component and operation
    pub fn with_operation<S: Into<String>, T: Into<String>>(component: S, operation: T) -> Self {
        LogContext {
            component: component.into(),
            operation: Some(operation.into()),
            fields: HashMap::new(),
        }
    }
    
    /// Add a field to the context
    pub fn with_field<S: Into<String>, T: Into<String>>(mut self, key: S, value: T) -> Self {
        self.fields.insert(key.into(), value.into());
        self
    }
    
    /// Add multiple fields to the context
    pub fn with_fields<S: Into<String>, T: Into<String>>(mut self, fields: Vec<(S, T)>) -> Self {
        for (key, value) in fields {
            self.fields.insert(key.into(), value.into());
        }
        self
    }
    
    /// Create a span with this context's information
    pub fn span(&self, level: Level) -> Span {
        // Use a constant level to avoid the compile error
        match level {
            Level::TRACE => {
                if let Some(op) = &self.operation {
                    tracing::trace_span!("rvoip", component = %self.component, operation = %op)
                } else {
                    tracing::trace_span!("rvoip", component = %self.component)
                }
            },
            Level::DEBUG => {
                if let Some(op) = &self.operation {
                    tracing::debug_span!("rvoip", component = %self.component, operation = %op)
                } else {
                    tracing::debug_span!("rvoip", component = %self.component)
                }
            },
            Level::INFO => {
                if let Some(op) = &self.operation {
                    tracing::info_span!("rvoip", component = %self.component, operation = %op)
                } else {
                    tracing::info_span!("rvoip", component = %self.component)
                }
            },
            Level::WARN => {
                if let Some(op) = &self.operation {
                    tracing::warn_span!("rvoip", component = %self.component, operation = %op)
                } else {
                    tracing::warn_span!("rvoip", component = %self.component)
                }
            },
            Level::ERROR => {
                if let Some(op) = &self.operation {
                    tracing::error_span!("rvoip", component = %self.component, operation = %op)
                } else {
                    tracing::error_span!("rvoip", component = %self.component)
                }
            },
        }
    }
}

impl fmt::Display for LogContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}]", self.component)?;
        
        if let Some(op) = &self.operation {
            write!(f, "[{}]", op)?;
        }
        
        for (key, value) in &self.fields {
            write!(f, "[{}={}]", key, value)?;
        }
        
        Ok(())
    }
}

/// Enter a logging context for the duration of a closure
pub fn with_context<F, R>(context: &LogContext, level: Level, f: F) -> R
where
    F: FnOnce() -> R,
{
    let span = context.span(level);
    let _guard = span.enter();
    f()
}

/// Macro for logging with context
#[macro_export]
macro_rules! log_with_context {
    ($level:expr, $ctx:expr, $($arg:tt)+) => {
        let span = $ctx.span($level);
        let _guard = span.enter();
        tracing::event!($level, $($arg)+);
    };
}

/// Macro for logging with a new context
#[macro_export]
macro_rules! log_ctx {
    ($level:expr, $component:expr, $operation:expr, $($arg:tt)+) => {
        let ctx = $crate::logging::context::LogContext::with_operation($component, $operation);
        $crate::log_with_context!($level, &ctx, $($arg)+);
    };
} 