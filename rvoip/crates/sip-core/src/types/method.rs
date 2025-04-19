use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// SIP request methods as defined in RFC 3261 and extensions
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Method {
    /// INVITE: Initiates a session
    Invite,
    /// ACK: Confirms a final response to an INVITE
    Ack,
    /// BYE: Terminates a session
    Bye,
    /// CANCEL: Cancels a pending request
    Cancel,
    /// REGISTER: Registers contact information
    Register,
    /// OPTIONS: Queries capabilities
    Options,
    /// SUBSCRIBE: Requests notification of an event
    Subscribe,
    /// NOTIFY: Sends notification of an event
    Notify,
    /// UPDATE: Modifies state of a session without changing dialog state
    Update,
    /// REFER: Asks recipient to issue a request
    Refer,
    /// INFO: Sends mid-session information
    Info,
    /// MESSAGE: Transports instant messages
    Message,
    /// PRACK: Provisional acknowledgment
    Prack,
    /// PUBLISH: Publishes event state
    Publish,
    /// Custom extension method
    Extension(String),
}

impl Method {
    /// Returns true if the method can establish a dialog
    pub fn creates_dialog(&self) -> bool {
        matches!(self, Method::Invite | Method::Subscribe)
    }

    /// Returns true if the method requires a response
    pub fn requires_response(&self) -> bool {
        !matches!(self, Method::Ack | Method::Cancel)
    }

    /// Returns true if the method is standardized (not an extension)
    pub fn is_standard(&self) -> bool {
        !matches!(self, Method::Extension(_))
    }

    /// Converts the method to its string representation
    pub fn as_str(&self) -> &str {
        match self {
            Method::Invite => "INVITE",
            Method::Ack => "ACK",
            Method::Bye => "BYE",
            Method::Cancel => "CANCEL",
            Method::Register => "REGISTER",
            Method::Options => "OPTIONS",
            Method::Subscribe => "SUBSCRIBE",
            Method::Notify => "NOTIFY",
            Method::Update => "UPDATE",
            Method::Refer => "REFER",
            Method::Info => "INFO",
            Method::Message => "MESSAGE",
            Method::Prack => "PRACK",
            Method::Publish => "PUBLISH",
            Method::Extension(method) => method,
        }
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Method {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "INVITE" => Ok(Method::Invite),
            "ACK" => Ok(Method::Ack),
            "BYE" => Ok(Method::Bye),
            "CANCEL" => Ok(Method::Cancel),
            "REGISTER" => Ok(Method::Register),
            "OPTIONS" => Ok(Method::Options),
            "SUBSCRIBE" => Ok(Method::Subscribe),
            "NOTIFY" => Ok(Method::Notify),
            "UPDATE" => Ok(Method::Update),
            "REFER" => Ok(Method::Refer),
            "INFO" => Ok(Method::Info),
            "MESSAGE" => Ok(Method::Message),
            "PRACK" => Ok(Method::Prack),
            "PUBLISH" => Ok(Method::Publish),
            _ if !s.is_empty() => Ok(Method::Extension(s.to_string())),
            _ => Err(Error::InvalidMethod),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_method_from_str() {
        assert_eq!(Method::from_str("INVITE").unwrap(), Method::Invite);
        assert_eq!(Method::from_str("ACK").unwrap(), Method::Ack);
        assert_eq!(Method::from_str("BYE").unwrap(), Method::Bye);
        assert_eq!(Method::from_str("REGISTER").unwrap(), Method::Register);
        assert_eq!(Method::from_str("OPTIONS").unwrap(), Method::Options);

        // Extension method
        let custom = Method::from_str("CUSTOM").unwrap();
        assert!(matches!(custom, Method::Extension(s) if s == "CUSTOM"));

        // Empty method is invalid
        assert!(Method::from_str("").is_err());
    }

    #[test]
    fn test_method_display() {
        assert_eq!(Method::Invite.to_string(), "INVITE");
        assert_eq!(Method::Ack.to_string(), "ACK");
        assert_eq!(Method::Bye.to_string(), "BYE");
        assert_eq!(Method::Register.to_string(), "REGISTER");
        assert_eq!(Method::Extension("CUSTOM".to_string()).to_string(), "CUSTOM");
    }

    #[test]
    fn test_method_properties() {
        assert!(Method::Invite.creates_dialog());
        assert!(Method::Subscribe.creates_dialog());
        assert!(!Method::Register.creates_dialog());

        assert!(Method::Invite.requires_response());
        assert!(!Method::Ack.requires_response());

        assert!(Method::Invite.is_standard());
        assert!(!Method::Extension("CUSTOM".to_string()).is_standard());
    }
} 