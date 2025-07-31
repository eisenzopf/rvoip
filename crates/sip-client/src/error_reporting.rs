//! Enhanced error reporting with actionable messages
//!
//! This module provides user-friendly error messages with suggested actions
//! for common error scenarios.

use crate::error::SipClientError;
use std::fmt::Write;

/// Error context information for enhanced reporting
#[derive(Debug, Clone)]
pub struct ErrorContext {
    /// Error string representation
    pub error_string: String,
    
    /// User-friendly description
    pub description: String,
    
    /// Suggested actions for the user
    pub actions: Vec<String>,
    
    /// Technical details for debugging
    pub technical_details: Option<String>,
    
    /// Error code for programmatic handling
    pub error_code: String,
}

/// Error reporter that generates user-friendly error messages
pub struct ErrorReporter;

impl ErrorReporter {
    /// Generate enhanced error context from a SipClientError
    pub fn enhance_error(error: &SipClientError) -> ErrorContext {
        match error {
            SipClientError::Network { message } => {
                ErrorContext {
                    error_string: error.to_string(),
                    description: "Network connectivity issue detected".to_string(),
                    actions: vec![
                        "Check your internet connection".to_string(),
                        "Verify firewall settings allow SIP traffic (UDP/TCP ports 5060-5061)".to_string(),
                        "Try disabling VPN if connected".to_string(),
                        "Contact your network administrator if in a corporate environment".to_string(),
                    ],
                    technical_details: Some(message.clone()),
                    error_code: "NETWORK_ERROR".to_string(),
                }
            }
            
            SipClientError::AudioDevice { message } => {
                ErrorContext {
                    error_string: error.to_string(),
                    description: "Audio device problem".to_string(),
                    actions: vec![
                        "Check that your microphone and speakers are properly connected".to_string(),
                        "Ensure no other application is using the audio devices".to_string(),
                        "Try selecting a different audio device in settings".to_string(),
                        "On macOS: Check System Preferences > Security & Privacy > Microphone permissions".to_string(),
                        "On Windows: Check sound settings and device permissions".to_string(),
                    ],
                    technical_details: Some(message.clone()),
                    error_code: "AUDIO_DEVICE_ERROR".to_string(),
                }
            }
            
            SipClientError::RegistrationFailed { reason } => {
                let mut actions = vec![
                    "Verify your SIP credentials (username and password)".to_string(),
                    "Check the SIP server address is correct".to_string(),
                    "Ensure your account is active and not suspended".to_string(),
                ];
                
                // Add specific actions based on the reason
                if reason.contains("401") || reason.contains("Unauthorized") {
                    actions.insert(0, "Your credentials appear to be incorrect".to_string());
                } else if reason.contains("404") || reason.contains("Not Found") {
                    actions.insert(0, "The SIP account may not exist on this server".to_string());
                } else if reason.contains("timeout") {
                    actions.insert(0, "The SIP server is not responding - it may be down or blocked".to_string());
                }
                
                ErrorContext {
                    error_string: error.to_string(),
                    description: "Failed to register with SIP server".to_string(),
                    actions,
                    technical_details: Some(reason.clone()),
                    error_code: "REGISTRATION_FAILED".to_string(),
                }
            }
            
            SipClientError::CallFailed { call_id, reason } => {
                let mut actions = vec![
                    "Check that the number you're calling is correct".to_string(),
                    "Verify the recipient is available and online".to_string(),
                    "Try calling again in a few moments".to_string(),
                ];
                
                if reason.contains("486") || reason.contains("Busy") {
                    actions.insert(0, "The person you're calling is busy - try again later".to_string());
                } else if reason.contains("404") || reason.contains("Not Found") {
                    actions.insert(0, "The number you're calling doesn't exist or is not registered".to_string());
                } else if reason.contains("488") || reason.contains("Not Acceptable") {
                    actions.insert(0, "No compatible audio codecs - contact your administrator".to_string());
                }
                
                ErrorContext {
                    error_string: error.to_string(),
                    description: format!("Call {} failed", call_id),
                    actions,
                    technical_details: Some(reason.clone()),
                    error_code: "CALL_FAILED".to_string(),
                }
            }
            
            SipClientError::CodecError { codec, details } => {
                ErrorContext {
                    error_string: error.to_string(),
                    description: format!("Audio codec '{}' error", codec),
                    actions: vec![
                        "Try using a different audio codec in settings".to_string(),
                        "Ensure your system has sufficient CPU resources".to_string(),
                        "Update to the latest version of the application".to_string(),
                    ],
                    technical_details: Some(details.clone()),
                    error_code: "CODEC_ERROR".to_string(),
                }
            }
            
            SipClientError::InvalidState { message } => {
                // Try to extract expected and actual from message if present
                let (expected, actual) = if message.contains("Expected state:") && message.contains("but was:") {
                    let parts: Vec<&str> = message.split(", but was: ").collect();
                    if parts.len() == 2 {
                        let expected = parts[0].replace("Expected state: ", "");
                        let actual = parts[1].to_string();
                        (Some(expected), Some(actual))
                    } else {
                        (None, None)
                    }
                } else {
                    (None, None)
                };
                
                let mut actions = vec![
                    "Check the call status before performing this action".to_string(),
                ];
                
                if let Some(expected_state) = &expected {
                    actions.insert(0, format!("Wait for the call to be in '{}' state", expected_state));
                }
                
                ErrorContext {
                    error_string: error.to_string(),
                    description: "Operation not allowed in current state".to_string(),
                    actions,
                    technical_details: if expected.is_some() && actual.is_some() {
                        Some(format!("Expected: {}, Actual: {}", expected.unwrap(), actual.unwrap()))
                    } else {
                        Some(message.clone())
                    },
                    error_code: "INVALID_STATE".to_string(),
                }
            }
            
            SipClientError::Timeout { seconds } => {
                ErrorContext {
                    error_string: error.to_string(),
                    description: format!("Operation timed out after {} seconds", seconds),
                    actions: vec![
                        "Check your network connection".to_string(),
                        "Try the operation again".to_string(),
                        "Contact support if the problem persists".to_string(),
                    ],
                    technical_details: None,
                    error_code: "TIMEOUT".to_string(),
                }
            }
            
            SipClientError::Internal { message } => {
                ErrorContext {
                    error_string: error.to_string(),
                    description: "An internal error occurred".to_string(),
                    actions: vec![
                        "Try restarting the application".to_string(),
                        "Check for available updates".to_string(),
                        "Contact support with the error details".to_string(),
                    ],
                    technical_details: Some(message.clone()),
                    error_code: "INTERNAL_ERROR".to_string(),
                }
            }
            
            _ => {
                // Generic error handling for any unmatched errors
                ErrorContext {
                    error_string: error.to_string(),
                    description: "An unexpected error occurred".to_string(),
                    actions: vec![
                        "Try the operation again".to_string(),
                        "Restart the application if the problem persists".to_string(),
                        "Contact support for assistance".to_string(),
                    ],
                    technical_details: Some(error.to_string()),
                    error_code: "UNKNOWN_ERROR".to_string(),
                }
            }
        }
    }
    
    /// Format error context as a user-friendly message
    pub fn format_user_message(context: &ErrorContext) -> String {
        let mut message = String::new();
        
        // Header
        writeln!(&mut message, "❌ {}", context.description).unwrap();
        writeln!(&mut message).unwrap();
        
        // Suggested actions
        if !context.actions.is_empty() {
            writeln!(&mut message, "💡 What you can do:").unwrap();
            for (i, action) in context.actions.iter().enumerate() {
                writeln!(&mut message, "   {}. {}", i + 1, action).unwrap();
            }
            writeln!(&mut message).unwrap();
        }
        
        // Error code for support
        writeln!(&mut message, "📋 Error code: {}", context.error_code).unwrap();
        
        // Technical details (if in debug mode)
        if cfg!(debug_assertions) {
            if let Some(details) = &context.technical_details {
                writeln!(&mut message).unwrap();
                writeln!(&mut message, "🔧 Technical details:").unwrap();
                writeln!(&mut message, "   {}", details).unwrap();
            }
        }
        
        message
    }
    
    /// Format error context as a JSON object for programmatic handling
    pub fn format_json(context: &ErrorContext) -> serde_json::Value {
        serde_json::json!({
            "error_code": context.error_code,
            "description": context.description,
            "actions": context.actions,
            "technical_details": context.technical_details,
        })
    }
}

/// Extension trait for SipClientError to add enhanced reporting
pub trait ErrorReportingExt {
    /// Get enhanced error context
    fn enhance(&self) -> ErrorContext;
    
    /// Get user-friendly error message
    fn user_message(&self) -> String;
}

impl ErrorReportingExt for SipClientError {
    fn enhance(&self) -> ErrorContext {
        ErrorReporter::enhance_error(self)
    }
    
    fn user_message(&self) -> String {
        let context = self.enhance();
        ErrorReporter::format_user_message(&context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_network_error_enhancement() {
        let error = SipClientError::Network {
            message: "Connection refused".to_string(),
        };
        
        let context = error.enhance();
        assert_eq!(context.error_code, "NETWORK_ERROR");
        assert!(!context.actions.is_empty());
        assert!(context.actions[0].contains("internet connection"));
    }
    
    #[test]
    fn test_registration_failed_401() {
        let error = SipClientError::RegistrationFailed {
            reason: "401 Unauthorized".to_string(),
        };
        
        let context = error.enhance();
        assert_eq!(context.error_code, "REGISTRATION_FAILED");
        assert!(context.actions[0].contains("credentials"));
    }
    
    #[test]
    fn test_call_failed_busy() {
        let error = SipClientError::CallFailed {
            call_id: "test-call".to_string(),
            reason: "486 Busy Here".to_string(),
        };
        
        let context = error.enhance();
        assert_eq!(context.error_code, "CALL_FAILED");
        assert!(context.actions[0].contains("busy"));
    }
    
    #[test]
    fn test_user_message_formatting() {
        let error = SipClientError::AudioDevice {
            message: "No input device found".to_string(),
        };
        
        let message = error.user_message();
        assert!(message.contains("❌"));
        assert!(message.contains("💡"));
        assert!(message.contains("📋"));
        assert!(message.contains("AUDIO_DEVICE_ERROR"));
    }
}