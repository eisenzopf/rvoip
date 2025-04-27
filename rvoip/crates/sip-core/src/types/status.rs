//! # SIP Status Codes
//!
//! This module provides an implementation of SIP status codes as defined in
//! [RFC 3261 Section 21](https://datatracker.ietf.org/doc/html/rfc3261#section-21) and its extensions.
//!
//! SIP status codes are three-digit integers that indicate the outcome of a SIP request.
//! They are similar in concept to HTTP status codes and follow the same general pattern:
//!
//! - `1xx`: Provisional — Request received, continuing to process the request
//! - `2xx`: Success — The action was successfully received, understood, and accepted
//! - `3xx`: Redirection — Further action needs to be taken to complete the request
//! - `4xx`: Client Error — The request contains bad syntax or cannot be fulfilled at this server
//! - `5xx`: Server Error — The server failed to fulfill an apparently valid request
//! - `6xx`: Global Failure — The request cannot be fulfilled at any server
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create a status code enum variant
//! let status = StatusCode::Ok;
//! assert_eq!(status.as_u16(), 200);
//! assert_eq!(status.reason_phrase(), "OK");
//!
//! // Check status code classification
//! assert!(status.is_success());
//! assert!(!status.is_error());
//!
//! // Create from a numeric value
//! let status = StatusCode::from_u16(404).unwrap();
//! assert_eq!(status, StatusCode::NotFound);
//!
//! // Parse from a string
//! let status = StatusCode::from_str("486").unwrap();
//! assert_eq!(status, StatusCode::BusyHere);
//! ```

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// SIP status codes as defined in RFC 3261 and extensions
///
/// SIP response status codes are used to indicate the outcome of a SIP request.
/// These status codes are organized into six classes:
///
/// - `1xx` (100-199): Provisional — Request received, continuing to process the request
/// - `2xx` (200-299): Success — The action was successfully received, understood, and accepted
/// - `3xx` (300-399): Redirection — Further action needs to be taken to complete the request
/// - `4xx` (400-499): Client Error — The request contains bad syntax or cannot be fulfilled at this server
/// - `5xx` (500-599): Server Error — The server failed to fulfill an apparently valid request
/// - `6xx` (600-699): Global Failure — The request cannot be fulfilled at any server
///
/// Each status code has a standard reason phrase associated with it, but the numeric value
/// is what determines the meaning of the response.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
///
/// // Check status code classes
/// let trying = StatusCode::Trying;
/// assert!(trying.is_provisional());  // 1xx class
///
/// let ok = StatusCode::Ok;
/// assert!(ok.is_success());  // 2xx class
///
/// let not_found = StatusCode::NotFound;
/// assert!(not_found.is_client_error());  // 4xx class
/// assert!(not_found.is_error());  // Any 4xx, 5xx, or 6xx
///
/// // Format status code
/// assert_eq!(ok.to_string(), "200 OK");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u16)]
pub enum StatusCode {
    // 1xx: Provisional
    /// 100 Trying
    Trying = 100,
    /// 180 Ringing
    Ringing = 180,
    /// 181 Call Is Being Forwarded
    CallIsBeingForwarded = 181,
    /// 182 Queued
    Queued = 182,
    /// 183 Session Progress
    SessionProgress = 183,

    // 2xx: Success
    /// 200 OK
    Ok = 200,
    /// 202 Accepted
    Accepted = 202,

    // 3xx: Redirection
    /// 300 Multiple Choices
    MultipleChoices = 300,
    /// 301 Moved Permanently
    MovedPermanently = 301,
    /// 302 Moved Temporarily
    MovedTemporarily = 302,
    /// 305 Use Proxy
    UseProxy = 305,
    /// 380 Alternative Service
    AlternativeService = 380,

    // 4xx: Client Error
    /// 400 Bad Request
    BadRequest = 400,
    /// 401 Unauthorized
    Unauthorized = 401,
    /// 402 Payment Required
    PaymentRequired = 402,
    /// 403 Forbidden
    Forbidden = 403,
    /// 404 Not Found
    NotFound = 404,
    /// 405 Method Not Allowed
    MethodNotAllowed = 405,
    /// 406 Not Acceptable
    NotAcceptable = 406,
    /// 407 Proxy Authentication Required
    ProxyAuthenticationRequired = 407,
    /// 408 Request Timeout
    RequestTimeout = 408,
    /// 410 Gone
    Gone = 410,
    /// 413 Request Entity Too Large
    RequestEntityTooLarge = 413,
    /// 414 Request-URI Too Long
    RequestUriTooLong = 414,
    /// 415 Unsupported Media Type
    UnsupportedMediaType = 415,
    /// 416 Unsupported URI Scheme
    UnsupportedUriScheme = 416,
    /// 420 Bad Extension
    BadExtension = 420,
    /// 421 Extension Required
    ExtensionRequired = 421,
    /// 423 Interval Too Brief
    IntervalTooBrief = 423,
    /// 480 Temporarily Unavailable
    TemporarilyUnavailable = 480,
    /// 481 Call/Transaction Does Not Exist
    CallOrTransactionDoesNotExist = 481,
    /// 482 Loop Detected
    LoopDetected = 482,
    /// 483 Too Many Hops
    TooManyHops = 483,
    /// 484 Address Incomplete
    AddressIncomplete = 484,
    /// 485 Ambiguous
    Ambiguous = 485,
    /// 486 Busy Here
    BusyHere = 486,
    /// 487 Request Terminated
    RequestTerminated = 487,
    /// 488 Not Acceptable Here
    NotAcceptableHere = 488,
    /// 491 Request Pending
    RequestPending = 491,
    /// 493 Undecipherable
    Undecipherable = 493,

    // 5xx: Server Error
    /// 500 Server Internal Error
    ServerInternalError = 500,
    /// 501 Not Implemented
    NotImplemented = 501,
    /// 502 Bad Gateway
    BadGateway = 502,
    /// 503 Service Unavailable
    ServiceUnavailable = 503,
    /// 504 Server Time-out
    ServerTimeout = 504,
    /// 505 Version Not Supported
    VersionNotSupported = 505,
    /// 513 Message Too Large
    MessageTooLarge = 513,

    // 6xx: Global Failure
    /// 600 Busy Everywhere
    BusyEverywhere = 600,
    /// 603 Decline
    Decline = 603,
    /// 604 Does Not Exist Anywhere
    DoesNotExistAnywhere = 604,
    /// 606 Not Acceptable
    NotAcceptable606 = 606,

    /// Custom status code (with value)
    Custom(u16),
}

impl StatusCode {
    /// Creates a status code from a raw u16 value
    ///
    /// This method converts a numeric status code into the corresponding
    /// `StatusCode` enum variant. If the numeric code matches a known
    /// status code, the specific variant is returned. Otherwise, if the
    /// code is within the valid range (100-699), a `Custom` variant is
    /// returned. If the code is outside this range, an error is returned.
    ///
    /// # Parameters
    ///
    /// - `code`: The numeric status code value
    ///
    /// # Returns
    ///
    /// - `Ok(StatusCode)`: If the code is valid (100-699)
    /// - `Err(Error::InvalidStatusCode)`: If the code is outside the valid range
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create known status code
    /// let status = StatusCode::from_u16(200).unwrap();
    /// assert_eq!(status, StatusCode::Ok);
    ///
    /// // Create custom status code
    /// let status = StatusCode::from_u16(599).unwrap();
    /// match status {
    ///     StatusCode::Custom(code) => assert_eq!(code, 599),
    ///     _ => panic!("Expected Custom variant"),
    /// }
    ///
    /// // Invalid status code
    /// assert!(StatusCode::from_u16(99).is_err());  // Too low
    /// assert!(StatusCode::from_u16(700).is_err()); // Too high
    /// ```
    pub fn from_u16(code: u16) -> Result<Self> {
        match code {
            100 => Ok(StatusCode::Trying),
            180 => Ok(StatusCode::Ringing),
            181 => Ok(StatusCode::CallIsBeingForwarded),
            182 => Ok(StatusCode::Queued),
            183 => Ok(StatusCode::SessionProgress),

            200 => Ok(StatusCode::Ok),
            202 => Ok(StatusCode::Accepted),

            300 => Ok(StatusCode::MultipleChoices),
            301 => Ok(StatusCode::MovedPermanently),
            302 => Ok(StatusCode::MovedTemporarily),
            305 => Ok(StatusCode::UseProxy),
            380 => Ok(StatusCode::AlternativeService),

            400 => Ok(StatusCode::BadRequest),
            401 => Ok(StatusCode::Unauthorized),
            402 => Ok(StatusCode::PaymentRequired),
            403 => Ok(StatusCode::Forbidden),
            404 => Ok(StatusCode::NotFound),
            405 => Ok(StatusCode::MethodNotAllowed),
            406 => Ok(StatusCode::NotAcceptable),
            407 => Ok(StatusCode::ProxyAuthenticationRequired),
            408 => Ok(StatusCode::RequestTimeout),
            410 => Ok(StatusCode::Gone),
            413 => Ok(StatusCode::RequestEntityTooLarge),
            414 => Ok(StatusCode::RequestUriTooLong),
            415 => Ok(StatusCode::UnsupportedMediaType),
            416 => Ok(StatusCode::UnsupportedUriScheme),
            420 => Ok(StatusCode::BadExtension),
            421 => Ok(StatusCode::ExtensionRequired),
            423 => Ok(StatusCode::IntervalTooBrief),
            480 => Ok(StatusCode::TemporarilyUnavailable),
            481 => Ok(StatusCode::CallOrTransactionDoesNotExist),
            482 => Ok(StatusCode::LoopDetected),
            483 => Ok(StatusCode::TooManyHops),
            484 => Ok(StatusCode::AddressIncomplete),
            485 => Ok(StatusCode::Ambiguous),
            486 => Ok(StatusCode::BusyHere),
            487 => Ok(StatusCode::RequestTerminated),
            488 => Ok(StatusCode::NotAcceptableHere),
            491 => Ok(StatusCode::RequestPending),
            493 => Ok(StatusCode::Undecipherable),

            500 => Ok(StatusCode::ServerInternalError),
            501 => Ok(StatusCode::NotImplemented),
            502 => Ok(StatusCode::BadGateway),
            503 => Ok(StatusCode::ServiceUnavailable),
            504 => Ok(StatusCode::ServerTimeout),
            505 => Ok(StatusCode::VersionNotSupported),
            513 => Ok(StatusCode::MessageTooLarge),

            600 => Ok(StatusCode::BusyEverywhere),
            603 => Ok(StatusCode::Decline),
            604 => Ok(StatusCode::DoesNotExistAnywhere),
            606 => Ok(StatusCode::NotAcceptable606),

            _ if code >= 100 && code < 700 => Ok(StatusCode::Custom(code)),
            _ => Err(Error::InvalidStatusCode(code)),
        }
    }

    /// Returns the numeric value of this status code
    ///
    /// Converts the `StatusCode` enum variant back to its underlying numeric value.
    ///
    /// # Returns
    ///
    /// The numeric value of the status code as a `u16`
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// assert_eq!(StatusCode::Ok.as_u16(), 200);
    /// assert_eq!(StatusCode::NotFound.as_u16(), 404);
    /// assert_eq!(StatusCode::Custom(499).as_u16(), 499);
    /// ```
    pub fn as_u16(&self) -> u16 {
        match self {
            StatusCode::Trying => 100,
            StatusCode::Ringing => 180,
            StatusCode::CallIsBeingForwarded => 181,
            StatusCode::Queued => 182,
            StatusCode::SessionProgress => 183,

            StatusCode::Ok => 200,
            StatusCode::Accepted => 202,

            StatusCode::MultipleChoices => 300,
            StatusCode::MovedPermanently => 301,
            StatusCode::MovedTemporarily => 302,
            StatusCode::UseProxy => 305,
            StatusCode::AlternativeService => 380,

            StatusCode::BadRequest => 400,
            StatusCode::Unauthorized => 401,
            StatusCode::PaymentRequired => 402,
            StatusCode::Forbidden => 403,
            StatusCode::NotFound => 404,
            StatusCode::MethodNotAllowed => 405,
            StatusCode::NotAcceptable => 406,
            StatusCode::ProxyAuthenticationRequired => 407,
            StatusCode::RequestTimeout => 408,
            StatusCode::Gone => 410,
            StatusCode::RequestEntityTooLarge => 413,
            StatusCode::RequestUriTooLong => 414,
            StatusCode::UnsupportedMediaType => 415,
            StatusCode::UnsupportedUriScheme => 416,
            StatusCode::BadExtension => 420,
            StatusCode::ExtensionRequired => 421,
            StatusCode::IntervalTooBrief => 423,
            StatusCode::TemporarilyUnavailable => 480,
            StatusCode::CallOrTransactionDoesNotExist => 481,
            StatusCode::LoopDetected => 482,
            StatusCode::TooManyHops => 483,
            StatusCode::AddressIncomplete => 484,
            StatusCode::Ambiguous => 485,
            StatusCode::BusyHere => 486,
            StatusCode::RequestTerminated => 487,
            StatusCode::NotAcceptableHere => 488,
            StatusCode::RequestPending => 491,
            StatusCode::Undecipherable => 493,

            StatusCode::ServerInternalError => 500,
            StatusCode::NotImplemented => 501,
            StatusCode::BadGateway => 502,
            StatusCode::ServiceUnavailable => 503,
            StatusCode::ServerTimeout => 504,
            StatusCode::VersionNotSupported => 505,
            StatusCode::MessageTooLarge => 513,

            StatusCode::BusyEverywhere => 600,
            StatusCode::Decline => 603,
            StatusCode::DoesNotExistAnywhere => 604,
            StatusCode::NotAcceptable606 => 606,

            StatusCode::Custom(code) => *code,
        }
    }

    /// Returns the canonical reason phrase for this status code
    ///
    /// Gets the standard reason phrase associated with this status code,
    /// as defined in RFC 3261. For custom status codes, "Unknown" is returned.
    ///
    /// # Returns
    ///
    /// A static string containing the reason phrase
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// assert_eq!(StatusCode::Ok.reason_phrase(), "OK");
    /// assert_eq!(StatusCode::NotFound.reason_phrase(), "Not Found");
    /// assert_eq!(StatusCode::Custom(499).reason_phrase(), "Unknown");
    /// ```
    pub fn reason_phrase(&self) -> &'static str {
        match self {
            StatusCode::Trying => "Trying",
            StatusCode::Ringing => "Ringing",
            StatusCode::CallIsBeingForwarded => "Call Is Being Forwarded",
            StatusCode::Queued => "Queued",
            StatusCode::SessionProgress => "Session Progress",

            StatusCode::Ok => "OK",
            StatusCode::Accepted => "Accepted",

            StatusCode::MultipleChoices => "Multiple Choices",
            StatusCode::MovedPermanently => "Moved Permanently",
            StatusCode::MovedTemporarily => "Moved Temporarily",
            StatusCode::UseProxy => "Use Proxy",
            StatusCode::AlternativeService => "Alternative Service",

            StatusCode::BadRequest => "Bad Request",
            StatusCode::Unauthorized => "Unauthorized",
            StatusCode::PaymentRequired => "Payment Required",
            StatusCode::Forbidden => "Forbidden",
            StatusCode::NotFound => "Not Found",
            StatusCode::MethodNotAllowed => "Method Not Allowed",
            StatusCode::NotAcceptable => "Not Acceptable",
            StatusCode::ProxyAuthenticationRequired => "Proxy Authentication Required",
            StatusCode::RequestTimeout => "Request Timeout",
            StatusCode::Gone => "Gone",
            StatusCode::RequestEntityTooLarge => "Request Entity Too Large",
            StatusCode::RequestUriTooLong => "Request-URI Too Long",
            StatusCode::UnsupportedMediaType => "Unsupported Media Type",
            StatusCode::UnsupportedUriScheme => "Unsupported URI Scheme",
            StatusCode::BadExtension => "Bad Extension",
            StatusCode::ExtensionRequired => "Extension Required",
            StatusCode::IntervalTooBrief => "Interval Too Brief",
            StatusCode::TemporarilyUnavailable => "Temporarily Unavailable",
            StatusCode::CallOrTransactionDoesNotExist => "Call/Transaction Does Not Exist",
            StatusCode::LoopDetected => "Loop Detected",
            StatusCode::TooManyHops => "Too Many Hops",
            StatusCode::AddressIncomplete => "Address Incomplete",
            StatusCode::Ambiguous => "Ambiguous",
            StatusCode::BusyHere => "Busy Here",
            StatusCode::RequestTerminated => "Request Terminated",
            StatusCode::NotAcceptableHere => "Not Acceptable Here",
            StatusCode::RequestPending => "Request Pending",
            StatusCode::Undecipherable => "Undecipherable",

            StatusCode::ServerInternalError => "Server Internal Error",
            StatusCode::NotImplemented => "Not Implemented",
            StatusCode::BadGateway => "Bad Gateway",
            StatusCode::ServiceUnavailable => "Service Unavailable",
            StatusCode::ServerTimeout => "Server Time-out",
            StatusCode::VersionNotSupported => "Version Not Supported",
            StatusCode::MessageTooLarge => "Message Too Large",

            StatusCode::BusyEverywhere => "Busy Everywhere",
            StatusCode::Decline => "Decline",
            StatusCode::DoesNotExistAnywhere => "Does Not Exist Anywhere",
            StatusCode::NotAcceptable606 => "Not Acceptable",

            StatusCode::Custom(_) => "Unknown",
        }
    }

    /// Returns true if this status code is provisional (1xx)
    ///
    /// Checks if the status code is in the provisional (1xx) range.
    /// Provisional responses indicate that the request was received and is being processed.
    ///
    /// # Returns
    ///
    /// `true` if the status code is in the range 100-199, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// assert!(StatusCode::Trying.is_provisional());      // 100
    /// assert!(StatusCode::Ringing.is_provisional());     // 180
    /// assert!(!StatusCode::Ok.is_provisional());         // 200
    /// assert!(!StatusCode::NotFound.is_provisional());   // 404
    /// ```
    pub fn is_provisional(&self) -> bool {
        let code = self.as_u16();
        code >= 100 && code < 200
    }

    /// Returns true if this status code is success (2xx)
    ///
    /// Checks if the status code is in the success (2xx) range.
    /// Success responses indicate that the request was successfully received,
    /// understood, and accepted.
    ///
    /// # Returns
    ///
    /// `true` if the status code is in the range 200-299, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// assert!(StatusCode::Ok.is_success());                // 200
    /// assert!(StatusCode::Accepted.is_success());          // 202
    /// assert!(!StatusCode::Trying.is_success());           // 100
    /// assert!(!StatusCode::MovedTemporarily.is_success()); // 302
    /// ```
    pub fn is_success(&self) -> bool {
        let code = self.as_u16();
        code >= 200 && code < 300
    }

    /// Returns true if this status code is redirection (3xx)
    ///
    /// Checks if the status code is in the redirection (3xx) range.
    /// Redirection responses indicate that further action needs to be
    /// taken in order to complete the request.
    ///
    /// # Returns
    ///
    /// `true` if the status code is in the range 300-399, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// assert!(StatusCode::MovedTemporarily.is_redirection());   // 302
    /// assert!(StatusCode::MultipleChoices.is_redirection());    // 300
    /// assert!(!StatusCode::Ok.is_redirection());                // 200
    /// assert!(!StatusCode::BadRequest.is_redirection());        // 400
    /// ```
    pub fn is_redirection(&self) -> bool {
        let code = self.as_u16();
        code >= 300 && code < 400
    }

    /// Returns true if this status code is client error (4xx)
    ///
    /// Checks if the status code is in the client error (4xx) range.
    /// Client error responses indicate that the request contains bad syntax
    /// or cannot be fulfilled at this server.
    ///
    /// # Returns
    ///
    /// `true` if the status code is in the range 400-499, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// assert!(StatusCode::BadRequest.is_client_error());         // 400
    /// assert!(StatusCode::NotFound.is_client_error());           // 404
    /// assert!(StatusCode::BusyHere.is_client_error());           // 486
    /// assert!(!StatusCode::Ok.is_client_error());                // 200
    /// assert!(!StatusCode::ServerInternalError.is_client_error()); // 500
    /// ```
    pub fn is_client_error(&self) -> bool {
        let code = self.as_u16();
        code >= 400 && code < 500
    }

    /// Returns true if this status code is server error (5xx)
    ///
    /// Checks if the status code is in the server error (5xx) range.
    /// Server error responses indicate that the server failed to fulfill
    /// an apparently valid request.
    ///
    /// # Returns
    ///
    /// `true` if the status code is in the range 500-599, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// assert!(StatusCode::ServerInternalError.is_server_error()); // 500
    /// assert!(StatusCode::ServiceUnavailable.is_server_error());  // 503
    /// assert!(!StatusCode::NotFound.is_server_error());           // 404
    /// assert!(!StatusCode::BusyEverywhere.is_server_error());     // 600
    /// ```
    pub fn is_server_error(&self) -> bool {
        let code = self.as_u16();
        code >= 500 && code < 600
    }

    /// Returns true if this status code is global failure (6xx)
    ///
    /// Checks if the status code is in the global failure (6xx) range.
    /// Global failure responses indicate that the request cannot be
    /// fulfilled at any server.
    ///
    /// # Returns
    ///
    /// `true` if the status code is in the range 600-699, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// assert!(StatusCode::BusyEverywhere.is_global_failure());      // 600
    /// assert!(StatusCode::Decline.is_global_failure());             // 603
    /// assert!(!StatusCode::ServiceUnavailable.is_global_failure()); // 503
    /// assert!(!StatusCode::NotFound.is_global_failure());           // 404
    /// ```
    pub fn is_global_failure(&self) -> bool {
        let code = self.as_u16();
        code >= 600 && code < 700
    }

    /// Returns true if this status code indicates an error (4xx, 5xx, 6xx)
    ///
    /// Checks if the status code is in any of the error ranges:
    /// client error (4xx), server error (5xx), or global failure (6xx).
    ///
    /// # Returns
    ///
    /// `true` if the status code is 400 or greater, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// assert!(StatusCode::NotFound.is_error());           // 404 (4xx)
    /// assert!(StatusCode::ServerInternalError.is_error()); // 500 (5xx)
    /// assert!(StatusCode::BusyEverywhere.is_error());     // 600 (6xx)
    /// assert!(!StatusCode::Ok.is_error());                // 200 (2xx)
    /// assert!(!StatusCode::MovedTemporarily.is_error());  // 302 (3xx)
    /// assert!(!StatusCode::Trying.is_error());            // 100 (1xx)
    /// ```
    pub fn is_error(&self) -> bool {
        let code = self.as_u16();
        code >= 400 && code < 700
    }

    /// Get the textual reason phrase for the status code
    ///
    /// This is an alias for `reason_phrase()` that provides
    /// the standard reason phrase associated with this status code.
    ///
    /// # Returns
    ///
    /// A static string containing the reason phrase
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// assert_eq!(StatusCode::Ok.as_reason(), "OK");
    /// assert_eq!(StatusCode::NotFound.as_reason(), "Not Found");
    /// assert_eq!(StatusCode::Custom(499).as_reason(), "Custom Status Code");
    /// ```
    pub fn as_reason(&self) -> &'static str {
        match self {
            Self::Trying => "Trying",
            Self::Ringing => "Ringing",
            Self::CallIsBeingForwarded => "Call Is Being Forwarded",
            Self::Queued => "Queued",
            Self::SessionProgress => "Session Progress",
            Self::Ok => "OK",
            Self::Accepted => "Accepted",
            Self::MultipleChoices => "Multiple Choices",
            Self::MovedPermanently => "Moved Permanently",
            Self::MovedTemporarily => "Moved Temporarily",
            Self::UseProxy => "Use Proxy",
            Self::AlternativeService => "Alternative Service",
            Self::BadRequest => "Bad Request",
            Self::Unauthorized => "Unauthorized",
            Self::PaymentRequired => "Payment Required",
            Self::Forbidden => "Forbidden",
            Self::NotFound => "Not Found",
            Self::MethodNotAllowed => "Method Not Allowed",
            Self::NotAcceptable => "Not Acceptable",
            Self::ProxyAuthenticationRequired => "Proxy Authentication Required",
            Self::RequestTimeout => "Request Timeout",
            Self::Gone => "Gone",
            Self::RequestEntityTooLarge => "Request Entity Too Large",
            Self::RequestUriTooLong => "Request-URI Too Long",
            Self::UnsupportedMediaType => "Unsupported Media Type",
            Self::UnsupportedUriScheme => "Unsupported URI Scheme",
            Self::BadExtension => "Bad Extension",
            Self::ExtensionRequired => "Extension Required",
            Self::IntervalTooBrief => "Interval Too Brief",
            Self::TemporarilyUnavailable => "Temporarily Unavailable",
            Self::CallOrTransactionDoesNotExist => "Call/Transaction Does Not Exist",
            Self::LoopDetected => "Loop Detected",
            Self::TooManyHops => "Too Many Hops",
            Self::AddressIncomplete => "Address Incomplete",
            Self::Ambiguous => "Ambiguous",
            Self::BusyHere => "Busy Here",
            Self::RequestTerminated => "Request Terminated",
            Self::NotAcceptableHere => "Not Acceptable Here",
            Self::RequestPending => "Request Pending",
            Self::Undecipherable => "Undecipherable",
            Self::ServerInternalError => "Server Internal Error",
            Self::NotImplemented => "Not Implemented",
            Self::BadGateway => "Bad Gateway",
            Self::ServiceUnavailable => "Service Unavailable",
            Self::ServerTimeout => "Server Time-out",
            Self::VersionNotSupported => "Version Not Supported",
            Self::MessageTooLarge => "Message Too Large",
            Self::BusyEverywhere => "Busy Everywhere",
            Self::Decline => "Decline",
            Self::DoesNotExistAnywhere => "Does Not Exist Anywhere",
            Self::NotAcceptable606 => "Not Acceptable",
            Self::Custom(_) => "Custom Status Code",
        }
    }
}

impl fmt::Display for StatusCode {
    /// Formats the status code as a string.
    ///
    /// Formats the status code as "<numeric code> <reason phrase>",
    /// following the standard SIP protocol format.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::fmt::Display;
    ///
    /// assert_eq!(StatusCode::Ok.to_string(), "200 OK");
    /// assert_eq!(StatusCode::NotFound.to_string(), "404 Not Found");
    /// assert_eq!(StatusCode::Custom(499).to_string(), "499 Unknown");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.as_u16(), self.reason_phrase())
    }
}

impl FromStr for StatusCode {
    type Err = Error;

    /// Parses a string into a StatusCode.
    ///
    /// Parses a string containing a numeric status code into the corresponding
    /// StatusCode enum variant. The string should contain just the numeric
    /// value (e.g., "200", "404").
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse, containing a numeric status code
    ///
    /// # Returns
    ///
    /// - `Ok(StatusCode)`: If parsing is successful
    /// - `Err(Error::InvalidStatusCode)`: If the string cannot be parsed as a valid status code
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Parse known status codes
    /// let ok = StatusCode::from_str("200").unwrap();
    /// assert_eq!(ok, StatusCode::Ok);
    ///
    /// let not_found = StatusCode::from_str("404").unwrap();
    /// assert_eq!(not_found, StatusCode::NotFound);
    ///
    /// // Parse custom status code
    /// let custom = StatusCode::from_str("499").unwrap();
    /// match custom {
    ///     StatusCode::Custom(code) => assert_eq!(code, 499),
    ///     _ => panic!("Expected Custom variant"),
    /// }
    ///
    /// // Invalid input
    /// assert!(StatusCode::from_str("abc").is_err());
    /// assert!(StatusCode::from_str("99").is_err());
    /// ```
    fn from_str(s: &str) -> Result<Self> {
        let code = s.parse::<u16>().map_err(|_| Error::InvalidStatusCode(0))?;
        StatusCode::from_u16(code)
    }
} 