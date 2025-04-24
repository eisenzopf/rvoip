// Retry-After header type for SIP messages
// Format defined in RFC 3261 Section 20.33

use std::fmt;
use std::time::Duration;
use std::str::FromStr;
use nom::combinator::all_consuming;
use serde::{Serialize, Deserialize};
use crate::error::{Error, Result};
use crate::parser::headers::retry_after::{parse_retry_after, RetryParam};
use crate::types::param::Param;

/// RetryAfter represents a Retry-After header value
/// Used to indicate how long a service is expected to be unavailable
/// 
/// RFC 3261 Section 20.33:
/// Retry-After = "Retry-After" HCOLON delta-seconds [ comment ] *( SEMI retry-param )
/// retry-param = ("duration" EQUAL delta-seconds) / generic-param
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryAfter {
    /// The delay in seconds
    pub delay: u32,
    
    /// Optional comment (e.g., explaining why retry is needed)
    pub comment: Option<String>,
    
    /// Optional duration parameter (special case from retry-param)
    pub duration: Option<u32>,
    
    /// Other parameters
    pub parameters: Vec<Param>,
}

impl RetryAfter {
    /// Create a new RetryAfter with just a delay
    pub fn new(delay: u32) -> Self {
        RetryAfter {
            delay,
            comment: None,
            duration: None,
            parameters: Vec::new(),
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
    
    /// Set the duration parameter
    pub fn with_duration(mut self, duration: u32) -> Self {
        self.duration = Some(duration);
        self
    }
    
    /// Add a parameter to the RetryAfter
    pub fn with_param(mut self, param: Param) -> Self {
        self.parameters.push(param);
        self
    }
    
    /// Get the delay as a Duration
    pub fn as_duration(&self) -> Duration {
        Duration::from_secs(self.delay.into())
    }
    
    /// Get a parameter value by name (case-insensitive)
    pub fn get_param(&self, name: &str) -> Option<&str> {
        let name_lower = name.to_lowercase();
        for param in &self.parameters {
            if let Param::Other(key, Some(value)) = param {
                if key.to_lowercase() == name_lower {
                    return value.as_str();
                }
            }
        }
        None
    }
    
    /// Check if a parameter exists (case-insensitive)
    pub fn has_param(&self, name: &str) -> bool {
        self.parameters.iter().any(|p| {
            if let Param::Other(key, _) = p {
                key.to_lowercase() == name.to_lowercase()
            } else {
                false
            }
        })
    }
}

impl FromStr for RetryAfter {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let result = all_consuming(parse_retry_after)(s.as_bytes())
            .map(|(_, value)| {
                let mut retry_after = RetryAfter::new(value.delay);
                
                if let Some(comment) = value.comment {
                    retry_after.comment = Some(comment);
                }
                
                for param in value.params {
                    match param {
                        RetryParam::Duration(duration) => {
                            retry_after.duration = Some(duration);
                        },
                        RetryParam::Generic(param) => {
                            retry_after.parameters.push(param);
                        }
                    }
                }
                
                retry_after
            })
            .map_err(|e| Error::from(e.to_owned()));
            
        result
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
        
        // Duration parameter if present
        if let Some(duration) = self.duration {
            write!(f, ";duration={}", duration)?;
        }
        
        // Other parameters
        for param in &self.parameters {
            write!(f, ";{}", param)?;
        }
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::param::{GenericValue, Param};
    
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
    fn test_retry_after_with_duration() {
        let retry = RetryAfter::new(60)
            .with_duration(120);
        assert_eq!(retry.to_string(), "60;duration=120");
    }
    
    #[test]
    fn test_retry_after_with_params() {
        let retry = RetryAfter::new(60)
            .with_param(Param::Other("reason".to_string(), Some(GenericValue::Token("maintenance".to_string()))));
        assert_eq!(retry.to_string(), "60;reason=maintenance");
    }
    
    #[test]
    fn test_retry_after_complex() {
        let retry = RetryAfter::new(3600)
            .with_comment("System upgrade")
            .with_duration(7200)
            .with_param(Param::Other("reason".to_string(), Some(GenericValue::Token("maintenance".to_string()))));
        assert_eq!(retry.to_string(), "3600 (System upgrade);duration=7200;reason=maintenance");
    }
    
    #[test]
    fn test_retry_after_from_duration() {
        let duration = Duration::from_secs(300);
        let retry = RetryAfter::from_duration(duration);
        assert_eq!(retry.delay, 300);
        assert_eq!(retry.as_duration(), duration);
    }
    
    #[test]
    fn test_retry_after_from_str() {
        // Test basic parsing
        let retry = RetryAfter::from_str("60").unwrap();
        assert_eq!(retry.delay, 60);
        assert_eq!(retry.comment, None);
        assert_eq!(retry.duration, None);
        assert!(retry.parameters.is_empty());
        
        // Test with comment
        let retry = RetryAfter::from_str("120 (Server maintenance)").unwrap();
        assert_eq!(retry.delay, 120);
        assert_eq!(retry.comment, Some("Server maintenance".to_string()));
        
        // Test with duration
        let retry = RetryAfter::from_str("60;duration=120").unwrap();
        assert_eq!(retry.delay, 60);
        assert_eq!(retry.duration, Some(120));
        
        // Test with other parameters
        let retry = RetryAfter::from_str("60;reason=maintenance").unwrap();
        assert_eq!(retry.delay, 60);
        assert!(retry.has_param("reason"));
        assert_eq!(retry.get_param("reason"), Some("maintenance"));
        
        // Test complex case
        let retry = RetryAfter::from_str("3600 (System upgrade);duration=7200;reason=maintenance").unwrap();
        assert_eq!(retry.delay, 3600);
        assert_eq!(retry.comment, Some("System upgrade".to_string()));
        assert_eq!(retry.duration, Some(7200));
        assert!(retry.has_param("reason"));
    }
    
    #[test]
    fn test_retry_after_rfc_examples() {
        // Examples based on RFC 3261 Section 20.33
        let retry = RetryAfter::from_str("18000 (5 hours)").unwrap();
        assert_eq!(retry.delay, 18000);
        assert_eq!(retry.comment, Some("5 hours".to_string()));
        
        let retry = RetryAfter::from_str("120").unwrap();
        assert_eq!(retry.delay, 120);
        
        let retry = RetryAfter::from_str("3600;duration=1800").unwrap();
        assert_eq!(retry.delay, 3600);
        assert_eq!(retry.duration, Some(1800));
    }
    
    #[test]
    fn test_retry_after_invalid_input() {
        // Invalid delta-seconds
        assert!(RetryAfter::from_str("abc").is_err());
        
        // Invalid format
        assert!(RetryAfter::from_str("120 Server maintenance").is_err()); // Missing parentheses
        
        // Invalid parameter format
        assert!(RetryAfter::from_str("120;duration=abc").is_err()); // Non-numeric duration
        
        // Unclosed comment
        assert!(RetryAfter::from_str("120 (Server maintenance").is_err());
    }
} 