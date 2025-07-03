//! # SIP Methods
//!
//! This module provides an implementation of SIP request methods as defined in
//! [RFC 3261](https://datatracker.ietf.org/doc/html/rfc3261) and its extensions.
//!
//! SIP methods define the purpose of a request within the SIP protocol. Each method
//! has specific semantics and behaviors, determining how the request should be 
//! processed by servers and user agents.
//!
//! ## Core Methods (RFC 3261)
//!
//! - **INVITE**: Initiates a session or modifies parameters of an existing session
//! - **ACK**: Confirms final response to an INVITE request
//! - **BYE**: Terminates a session
//! - **CANCEL**: Cancels a pending request
//! - **REGISTER**: Registers contact information with a SIP registrar
//! - **OPTIONS**: Queries capabilities of servers or user agents
//!
//! ## Extension Methods
//!
//! - **SUBSCRIBE**: Requests notification of an event or set of events (RFC 6665)
//! - **NOTIFY**: Sends notifications of events (RFC 6665)
//! - **MESSAGE**: Transports instant messages (RFC 3428)
//! - **UPDATE**: Modifies session state without changing dialog state (RFC 3311)
//! - **INFO**: Sends mid-session information (RFC 6086)
//! - **PRACK**: Provides reliability for provisional responses (RFC 3262)
//! - **REFER**: Asks recipient to issue a request (RFC 3515)
//! - **PUBLISH**: Publishes event state (RFC 3903)
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create a method
//! let invite = Method::Invite;
//! assert_eq!(invite.to_string(), "INVITE");
//! assert!(invite.creates_dialog());
//!
//! // Parse a method from a string
//! let bye = Method::from_str("BYE").unwrap();
//! assert_eq!(bye, Method::Bye);
//!
//! // Handle custom extension methods
//! let custom = Method::from_str("CUSTOM").unwrap();
//! assert!(matches!(custom, Method::Extension(s) if s == "CUSTOM"));
//! ```

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// SIP request methods as defined in RFC 3261 and extensions
///
/// This enum represents the standard SIP methods defined in RFC 3261 and common
/// extensions. It also supports custom extension methods through the `Extension` variant.
///
/// Each method has specific semantics in the SIP protocol:
/// - Some methods can establish dialogs (e.g., INVITE, SUBSCRIBE)
/// - Some methods require responses (most methods except ACK and CANCEL)
/// - Some methods are defined in the core specification, while others are extensions
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Core methods
/// let invite = Method::Invite;
/// let register = Method::Register;
///
/// // Extension methods
/// let subscribe = Method::Subscribe;
/// let message = Method::Message;
///
/// // Custom extension method
/// let custom = Method::Extension("CUSTOM".to_string());
///
/// // Converting to string
/// assert_eq!(invite.to_string(), "INVITE");
/// assert_eq!(custom.to_string(), "CUSTOM");
///
/// // Parsing from string
/// assert_eq!(Method::from_str("BYE").unwrap(), Method::Bye);
/// ```
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
    ///
    /// In SIP, only certain methods can create dialogs between user agents.
    /// The primary methods that establish dialogs are INVITE and SUBSCRIBE.
    ///
    /// # Returns
    ///
    /// `true` if the method can create a dialog, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Methods that create dialogs
    /// assert!(Method::Invite.creates_dialog());
    /// assert!(Method::Subscribe.creates_dialog());
    ///
    /// // Methods that don't create dialogs
    /// assert!(!Method::Register.creates_dialog());
    /// assert!(!Method::Message.creates_dialog());
    /// assert!(!Method::Extension("CUSTOM".to_string()).creates_dialog());
    /// ```
    pub fn creates_dialog(&self) -> bool {
        matches!(self, Method::Invite | Method::Subscribe)
    }

    /// Returns true if the method requires a response
    ///
    /// Most SIP methods require a response, but there are exceptions.
    /// ACK and CANCEL are special methods that do not require responses
    /// according to the SIP specification.
    ///
    /// # Returns
    ///
    /// `true` if the method requires a response, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Methods that require responses
    /// assert!(Method::Invite.requires_response());
    /// assert!(Method::Register.requires_response());
    /// assert!(Method::Subscribe.requires_response());
    ///
    /// // Methods that don't require responses
    /// assert!(!Method::Ack.requires_response());
    /// assert!(!Method::Cancel.requires_response());
    /// ```
    pub fn requires_response(&self) -> bool {
        !matches!(self, Method::Ack | Method::Cancel)
    }

    /// Returns true if the method is standardized (not an extension)
    ///
    /// This method distinguishes between methods defined in the SIP specifications
    /// and custom extension methods. It returns `true` for all enum variants except
    /// `Method::Extension`.
    ///
    /// # Returns
    ///
    /// `true` if the method is a standard SIP method, `false` for custom extensions
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Standard methods
    /// assert!(Method::Invite.is_standard());
    /// assert!(Method::Register.is_standard());
    /// assert!(Method::Subscribe.is_standard());
    ///
    /// // Custom extension
    /// assert!(!Method::Extension("CUSTOM".to_string()).is_standard());
    /// ```
    pub fn is_standard(&self) -> bool {
        !matches!(self, Method::Extension(_))
    }

    /// Converts the method to its string representation
    ///
    /// Returns the canonical string representation of the SIP method.
    /// For standard methods, this is the uppercase name as defined in the
    /// SIP specifications. For extension methods, it's the string stored
    /// in the `Extension` variant.
    ///
    /// # Returns
    ///
    /// A string slice representing the method
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// assert_eq!(Method::Invite.as_str(), "INVITE");
    /// assert_eq!(Method::Ack.as_str(), "ACK");
    /// assert_eq!(Method::Register.as_str(), "REGISTER");
    ///
    /// let custom = Method::Extension("CUSTOM".to_string());
    /// assert_eq!(custom.as_str(), "CUSTOM");
    /// ```
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
    /// Formats the Method as a string.
    ///
    /// This implementation uses `as_str()` to convert the method to its
    /// string representation.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// assert_eq!(Method::Invite.to_string(), "INVITE");
    /// assert_eq!(Method::Register.to_string(), "REGISTER");
    ///
    /// // In a formatted string
    /// let method = Method::Invite;
    /// let formatted = format!("SIP method: {}", method);
    /// assert_eq!(formatted, "SIP method: INVITE");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Method {
    type Err = Error;

    /// Parses a string into a Method.
    ///
    /// This method converts a string to the corresponding Method enum variant.
    /// It recognizes all standard SIP methods defined in RFC 3261 and common
    /// extensions. If the string doesn't match any standard method but is not empty,
    /// it creates a `Method::Extension` variant with the string.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse
    ///
    /// # Returns
    ///
    /// A Result containing the parsed Method, or an error if parsing fails
    ///
    /// # Errors
    ///
    /// Returns `Error::InvalidMethod` if the input string is empty.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Standard methods
    /// assert_eq!(Method::from_str("INVITE").unwrap(), Method::Invite);
    /// assert_eq!(Method::from_str("BYE").unwrap(), Method::Bye);
    ///
    /// // Extension method
    /// let custom = Method::from_str("CUSTOM").unwrap();
    /// assert!(matches!(custom, Method::Extension(s) if s == "CUSTOM"));
    ///
    /// // Error case
    /// assert!(Method::from_str("").is_err());
    /// ```
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