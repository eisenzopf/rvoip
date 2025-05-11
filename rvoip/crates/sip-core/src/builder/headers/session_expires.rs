use crate::types::session_expires::{Refresher, SessionExpires};
use crate::types::param::Param; // Using Param::new which uses GenericValue internally
use crate::error::{Error, Result};
use super::HeaderSetter;

/// # SIP Session-Expires Header Extension
///
/// This module provides extension traits for easily adding Session-Expires headers
/// to SIP requests and responses. The Session-Expires header is defined in 
/// [RFC 4028](https://datatracker.ietf.org/doc/html/rfc4028) and is used
/// for session timer functionality in SIP.
///
/// ## Purpose of Session-Expires Header
///
/// The Session-Expires header serves several important purposes in SIP:
///
/// 1. It defines the maximum time between session refreshes.
/// 2. It specifies which party (UAC or UAS) is responsible for refreshing the session.
/// 3. It helps detect and recover from network or endpoint failures.
/// 4. It prevents "zombie" sessions after unclean disconnections.
///
/// ## Common Use Cases
///
/// - **Long-Running Calls**: Ensure calls are terminated if a party becomes unreachable.
/// - **Media Session Management**: Prevent orphaned media sessions.
/// - **Resource Conservation**: Cleanup resources for terminated but not properly closed sessions.
/// - **High Availability Systems**: Detect failed endpoints and assist in recovery.
///
/// ## Relationship with other headers
///
/// - **Session-Expires vs. Min-SE**: `Session-Expires` specifies the negotiated session interval, 
///   while `Min-SE` (Minimum Session Expires) in a request indicates the minimum interval the sender
///   is willing to accept. A 422 (Session Interval Too Small) response may include Min-SE.
/// - **Session-Expires vs. Expires**: `Session-Expires` applies to the entire SIP session (dialog)
///   and includes the refresher parameter. The `Expires` header, on the other hand, typically
///   applies to registrations (duration of a registration) or subscriptions (duration of a subscription)
///   and does not have a refresher parameter.
///
/// ## Example Usage
///
/// ```rust
/// # use rvoip_sip_core::RequestBuilder;
/// # use rvoip_sip_core::types::Method;
/// # use rvoip_sip_core::types::session_expires::Refresher;
/// # use rvoip_sip_core::builder::headers::SessionExpiresExt; // Import the trait
/// let request = RequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
///     .session_expires(1800, Some(Refresher::Uac)) // Set Session-Expires to 30 minutes, UAC refreshes
///     .build();
/// // The request now has a Session-Expires header: "Session-Expires: 1800;refresher=uac"
/// ```

pub trait SessionExpiresExt {
    /// Sets the Session-Expires header with the given interval and optional refresher.
    ///
    /// This method adds a Session-Expires header with the specified timeout value in seconds (delta-seconds),
    /// and optionally sets which party (UAC or UAS) should refresh the session.
    ///
    /// # Arguments
    ///
    /// * `delta_seconds`: The session interval in seconds (e.g., 1800 for 30 minutes).
    /// * `refresher`: An `Option<Refresher>`. 
    ///   - `Some(Refresher::Uac)`: UAC is responsible for refreshes.
    ///   - `Some(Refresher::Uas)`: UAS is responsible for refreshes.
    ///   - `None`: No refresher parameter is added; the default (UAC) applies as per RFC 4028.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use rvoip_sip_core::RequestBuilder;
    /// # use rvoip_sip_core::types::Method;
    /// # use rvoip_sip_core::types::session_expires::Refresher;
    /// # use rvoip_sip_core::builder::headers::SessionExpiresExt;
    /// // INVITE with Session-Expires, UAS refreshes
    /// let invite = RequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .session_expires(3600, Some(Refresher::Uas))
    ///     .build();
    /// // Header will be: "Session-Expires: 3600;refresher=uas"
    ///
    /// // INVITE with Session-Expires, default refresher (UAC)
    /// let invite_default_refresher = RequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .session_expires(1800, None)
    ///     .build();
    /// // Header will be: "Session-Expires: 1800"
    /// ```
    fn session_expires(self, delta_seconds: u32, refresher: Option<Refresher>) -> Self;

    /// Sets the Session-Expires header with interval, optional refresher, and additional generic parameters.
    ///
    /// Allows for full specification of the Session-Expires header, including any non-standard parameters.
    ///
    /// # Arguments
    ///
    /// * `delta_seconds`: The session interval in seconds.
    /// * `refresher`: An optional `Refresher` (Uac or Uas).
    /// * `params`: A vector of `Param` objects for any additional parameters.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use rvoip_sip_core::RequestBuilder;
    /// # use rvoip_sip_core::types::Method;
    /// # use rvoip_sip_core::types::session_expires::Refresher;
    /// # use rvoip_sip_core::types::param::Param;
    /// # use rvoip_sip_core::builder::headers::SessionExpiresExt;
    /// let custom_params = vec![Param::new("x-custom-se-flag", None), Param::new("x-info", Some("timer-A"))];
    /// let request = RequestBuilder::new(Method::Invite, "sip:carol@example.com").unwrap()
    ///     .session_expires_with_params(1200, Some(Refresher::Uac), custom_params)
    ///     .build();
    /// // Header might be: "Session-Expires: 1200;refresher=uac;x-custom-se-flag;x-info=timer-A"
    /// ```
    fn session_expires_with_params(self, delta_seconds: u32, refresher: Option<Refresher>, params: Vec<Param>) -> Self;

    /// Convenience method to set Session-Expires with UAC (User Agent Client) as the refresher.
    ///
    /// # Arguments
    ///
    /// * `delta_seconds`: The session interval in seconds.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use rvoip_sip_core::RequestBuilder;
    /// # use rvoip_sip_core::types::Method;
    /// # use rvoip_sip_core::builder::headers::SessionExpiresExt;
    /// let invite = RequestBuilder::new(Method::Invite, "sip:dave@example.com").unwrap()
    ///     .session_expires_uac(900) // 15 minutes, UAC refreshes
    ///     .build();
    /// // Header will be: "Session-Expires: 900;refresher=uac"
    /// ```
    fn session_expires_uac(self, delta_seconds: u32) -> Self;

    /// Convenience method to set Session-Expires with UAS (User Agent Server) as the refresher.
    ///
    /// # Arguments
    ///
    /// * `delta_seconds`: The session interval in seconds.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use rvoip_sip_core::RequestBuilder;
    /// # use rvoip_sip_core::types::Method;
    /// # use rvoip_sip_core::builder::headers::SessionExpiresExt;
    /// let response_ok = rvoip_sip_core::ResponseBuilder::new(rvoip_sip_core::types::StatusCode::Ok, None)
    ///     .session_expires_uas(2400) // 40 minutes, UAS refreshes
    ///     .build();
    /// // Header will be: "Session-Expires: 2400;refresher=uas"
    /// ```
    fn session_expires_uas(self, delta_seconds: u32) -> Self;
}

impl<T> SessionExpiresExt for T
where
    T: HeaderSetter,
{
    fn session_expires(self, delta_seconds: u32, refresher: Option<Refresher>) -> Self {
        let se_header = SessionExpires::new(delta_seconds, refresher);
        self.set_header(se_header)
    }

    fn session_expires_with_params(self, delta_seconds: u32, refresher: Option<Refresher>, params: Vec<Param>) -> Self {
        let se_header = SessionExpires::new_with_params(delta_seconds, refresher, params);
        self.set_header(se_header)
    }

    fn session_expires_uac(self, delta_seconds: u32) -> Self {
        self.session_expires(delta_seconds, Some(Refresher::Uac))
    }

    fn session_expires_uas(self, delta_seconds: u32) -> Self {
        self.session_expires(delta_seconds, Some(Refresher::Uas))
    }
}

/// Builder for `SessionExpires` headers.
///
/// Provides a fluent interface to construct `SessionExpires` instances.
/// The `delta_seconds` field is mandatory and must be set before calling `build()`.
#[derive(Debug, Default, Clone)]
pub struct SessionExpiresBuilder {
    delta_seconds: Option<u32>,
    refresher: Option<Refresher>,
    params: Vec<Param>,
}

impl SessionExpiresBuilder {
    /// Creates a new `SessionExpiresBuilder`.
    ///
    /// Initializes an empty builder. The `delta_seconds` must be set
    /// using the `delta_seconds()` method before `build()` can be called successfully.
    pub fn new() -> Self {
        SessionExpiresBuilder::default()
    }

    /// Sets the session interval (delta-seconds).
    ///
    /// This value indicates the lifetime of the session in seconds.
    /// It is a mandatory field for the `SessionExpires` header.
    ///
    /// # Arguments
    ///
    /// * `seconds`: The session interval in seconds.
    pub fn delta_seconds(mut self, seconds: u32) -> Self {
        self.delta_seconds = Some(seconds);
        self
    }

    /// Sets the refresher entity (UAC or UAS).
    ///
    /// This indicates which party is responsible for refreshing the session.
    /// If not set, the default behavior (typically UAC) applies as per RFC 4028.
    ///
    /// # Arguments
    ///
    /// * `refresher`: The `Refresher` enum variant (Uac or Uas).
    pub fn refresher(mut self, refresher: Refresher) -> Self {
        self.refresher = Some(refresher);
        self
    }

    /// Clears the refresher entity.
    ///
    /// Removes any previously set refresher, falling back to default behavior.
    pub fn clear_refresher(mut self) -> Self {
        self.refresher = None;
        self
    }

    /// Adds a generic parameter to the Session-Expires header.
    ///
    /// Parameters provide additional information or context.
    /// This method takes a pre-constructed `Param` object.
    ///
    /// # Arguments
    ///
    /// * `param`: The `Param` to add.
    pub fn param(mut self, param: Param) -> Self {
        self.params.push(param);
        self
    }

    /// Adds a key-value generic parameter.
    ///
    /// Convenience method to add a parameter by providing its key and optional value.
    ///
    /// # Arguments
    ///
    /// * `key`: The parameter name.
    /// * `value`: An optional parameter value. If `None`, the parameter is a flag.
    pub fn generic_param(mut self, key: impl Into<String>, value: Option<impl Into<String>>) -> Self {
        self.params.push(Param::new(key, value));
        self
    }
    
    /// Sets multiple generic parameters, replacing any existing ones.
    ///
    /// # Arguments
    ///
    /// * `params`: A vector of `Param` objects.
    pub fn params(mut self, params: Vec<Param>) -> Self {
        self.params = params;
        self
    }

    /// Clears all generic parameters from the builder.
    pub fn clear_params(mut self) -> Self {
        self.params.clear();
        self
    }

    /// Builds the `SessionExpires` header.
    ///
    /// Constructs the `SessionExpires` instance from the builder's current state.
    ///
    /// # Returns
    ///
    /// * `Ok(SessionExpires)` if `delta_seconds` has been set.
    /// * `Err(Error::BuilderError)` if `delta_seconds` is not set.
    pub fn build(&self) -> Result<SessionExpires> {
        let delta_seconds = self.delta_seconds.ok_or_else(|| {
            Error::BuilderError("Session-Expires: delta_seconds is mandatory".to_string())
        })?;

        Ok(SessionExpires::new_with_params(
            delta_seconds,
            self.refresher, // Option<Refresher> is fine here
            self.params.clone(), // Clone the params vector
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RequestBuilder; // Assuming this path is correct for tests
    use crate::types::{
        Method,
        headers::HeaderName,
        headers::TypedHeader,
    };
    use crate::types::session_expires::Refresher;
    use crate::types::param::Param;

    #[test]
    fn test_session_expires_set() {
        let request = RequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
            .session_expires(1800, Some(Refresher::Uac))
            .build();
        
        match request.header(&HeaderName::SessionExpires) {
            Some(TypedHeader::SessionExpires(se)) => {
                assert_eq!(se.delta_seconds(), 1800);
                assert_eq!(se.refresher(), Some(Refresher::Uac));
                assert!(se.params().is_empty());
            }
            _ => panic!("Session-Expires header not found or incorrect type"),
        }
    }

    #[test]
    fn test_session_expires_with_params_set() {
        let params = vec![Param::new("custom", Some("value"))];
        let request = RequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
            .session_expires_with_params(3600, None, params.clone())
            .build();

        match request.header(&HeaderName::SessionExpires) {
            Some(TypedHeader::SessionExpires(se)) => {
                assert_eq!(se.delta_seconds(), 3600);
                assert_eq!(se.refresher(), None);
                assert_eq!(se.params().len(), 1);
                assert_eq!(se.params()[0].key(), "custom");
                assert_eq!(se.params()[0].value(), Some("value".to_string()));
            }
            _ => panic!("Session-Expires header not found or incorrect type"),
        }
    }

    #[test]
    fn test_session_expires_uac_convenience() {
        let request = RequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
            .session_expires_uac(900)
            .build();

        match request.header(&HeaderName::SessionExpires) {
            Some(TypedHeader::SessionExpires(se)) => {
                assert_eq!(se.delta_seconds(), 900);
                assert_eq!(se.refresher(), Some(Refresher::Uac));
            }
            _ => panic!("Session-Expires header not found or incorrect type"),
        }
    }

    #[test]
    fn test_session_expires_uas_convenience() {
        let request = RequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
            .session_expires_uas(7200)
            .build();

        match request.header(&HeaderName::SessionExpires) {
            Some(TypedHeader::SessionExpires(se)) => {
                assert_eq!(se.delta_seconds(), 7200);
                assert_eq!(se.refresher(), Some(Refresher::Uas));
            }
            _ => panic!("Session-Expires header not found or incorrect type"),
        }
    }

    #[test]
    fn test_override_session_expires() {
        let request = RequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
            .session_expires_uac(100)
            .session_expires_uas(200) // This should override the previous one
            .build();

        match request.header(&HeaderName::SessionExpires) {
            Some(TypedHeader::SessionExpires(se)) => {
                assert_eq!(se.delta_seconds(), 200);
                assert_eq!(se.refresher(), Some(Refresher::Uas));
            }
            _ => panic!("Session-Expires header not found or incorrect type"),
        }
    }
} 