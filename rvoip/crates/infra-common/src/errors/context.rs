use crate::errors::types::Error;
use std::fmt;

/// Context information for an error
#[derive(Debug, Clone)]
pub struct ErrorContext {
    /// Component where the error occurred
    pub component: String,
    /// Operation that was being performed
    pub operation: String,
    /// Additional context information
    pub details: Option<String>,
}

impl ErrorContext {
    /// Create a new error context
    pub fn new<S: Into<String>, T: Into<String>>(component: S, operation: T) -> Self {
        ErrorContext {
            component: component.into(),
            operation: operation.into(),
            details: None,
        }
    }
    
    /// Add details to the context
    pub fn with_details<S: Into<String>>(mut self, details: S) -> Self {
        self.details = Some(details.into());
        self
    }
}

impl fmt::Display for ErrorContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "in component '{}' during operation '{}'", self.component, self.operation)?;
        if let Some(details) = &self.details {
            write!(f, " ({})", details)?;
        }
        Ok(())
    }
}

/// Extension trait for adding context to errors
pub trait ErrorExt {
    /// Add context to an error
    fn context(self, ctx: ErrorContext) -> Error;
    
    /// Add simple context with component and operation
    fn with_context<S: Into<String>, T: Into<String>>(self, component: S, operation: T) -> Error;
}

impl ErrorExt for Error {
    fn context(self, ctx: ErrorContext) -> Error {
        match self {
            Error::Custom(msg) => {
                Error::Custom(format!("{} [{}]", msg, ctx))
            },
            Error::Internal(msg) => {
                Error::Internal(format!("{} [{}]", msg, ctx))
            },
            other => {
                Error::Custom(format!("{} [{}]", other, ctx))
            }
        }
    }
    
    fn with_context<S: Into<String>, T: Into<String>>(self, component: S, operation: T) -> Error {
        self.context(ErrorContext::new(component, operation))
    }
}

impl<E> ErrorExt for Result<E, Error> {
    fn context(self, ctx: ErrorContext) -> Error {
        match self {
            Ok(_) => Error::Internal(format!("Called context() on Ok result [{}]", ctx)),
            Err(e) => e.context(ctx),
        }
    }
    
    fn with_context<S: Into<String>, T: Into<String>>(self, component: S, operation: T) -> Error {
        self.context(ErrorContext::new(component, operation))
    }
} 