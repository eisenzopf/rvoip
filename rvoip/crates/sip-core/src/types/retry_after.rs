// Retry-After header type for SIP messages
// Format defined in RFC 3261 Section 20.33

use std::fmt;
use std::time::Duration;
use std::collections::HashMap;
use serde::{Serialize, Deserialize};

/// RetryAfter represents a Retry-After header value
/// Used to indicate how long a service is expected to be unavailable
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryAfter {
    /// The delay in seconds
    pub delay: u32,
    
    /// Optional comment (e.g., explaining why retry is needed)
    pub comment: Option<String>,
    
    /// Optional parameters (like duration-specified)
    pub parameters: HashMap<String, String>,
}

impl RetryAfter {
    /// Create a new RetryAfter with just a delay
    pub fn new(delay: u32) -> Self {
        RetryAfter {
            delay,
            comment: None,
            parameters: HashMap::new(),
        }
    }
    
    /// Create a RetryAfter from a Duration
    pub fn from_duration(duration: Duration) -> Self {
        RetryAfter::new(duration.as_secs() as u32)
    }
    
    /// Add a comment to the RetryAfter
    pub fn with_comment(mut self, comment: &str) -> Self {
        self.comment = Some(comment.to_string());
        self
    }
    
    /// Add a parameter to the RetryAfter
    pub fn with_param(mut self, name: &str, value: &str) -> Self {
        self.parameters.insert(name.to_lowercase(), value.to_string());
        self
    }
    
    /// Get the delay as a Duration
    pub fn as_duration(&self) -> Duration {
        Duration::from_secs(self.delay.into())
    }
}

impl fmt::Display for RetryAfter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // First the delay
        write!(f, "{}", self.delay)?;
        
        // Optional comment
        if let Some(comment) = &self.comment {
            write!(f, " ({})", comment)?;
        }
        
        // Parameters if any
        for (name, value) in &self.parameters {
            write!(f, ";{}={}", name, value)?;
        }
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_retry_after_display_simple() {
        let retry = RetryAfter::new(60);
        assert_eq!(retry.to_string(), "60");
    }
    
    #[test]
    fn test_retry_after_with_comment() {
        let retry = RetryAfter::new(120)
            .with_comment("Server maintenance");
        assert_eq!(retry.to_string(), "120 (Server maintenance)");
    }
    
    #[test]
    fn test_retry_after_with_params() {
        let retry = RetryAfter::new(60)
            .with_param("duration-specified", "true");
        assert_eq!(retry.to_string(), "60;duration-specified=true");
    }
    
    #[test]
    fn test_retry_after_complex() {
        let retry = RetryAfter::new(3600)
            .with_comment("System upgrade")
            .with_param("duration-specified", "true");
        assert_eq!(retry.to_string(), "3600 (System upgrade);duration-specified=true");
    }
    
    #[test]
    fn test_retry_after_from_duration() {
        let duration = Duration::from_secs(300);
        let retry = RetryAfter::from_duration(duration);
        assert_eq!(retry.delay, 300);
        assert_eq!(retry.as_duration(), duration);
    }
} 