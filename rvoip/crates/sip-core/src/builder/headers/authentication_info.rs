use crate::error::{Error, Result};
use crate::types::{
    auth::{
        AuthenticationInfo,
        AuthenticationInfoParam,
        Qop
    },
    TypedHeader,
    header::TypedHeaderTrait,
    headers::header_access::HeaderAccess,
};
use super::HeaderSetter;

/// Extension trait for adding Authentication-Info header building capabilities
pub trait AuthenticationInfoExt {
    /// Add an Authentication-Info header to the response
    ///
    /// # Arguments
    ///
    /// * `nextnonce` - Optional next nonce that should be used for the next request
    /// * `qop` - Optional quality of protection used for the request
    /// * `rspauth` - Optional response authentication for mutual authentication
    /// * `cnonce` - Optional client nonce that was used
    /// * `nc` - Optional nonce count that was used
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::AuthenticationInfoExt};
    /// use rvoip_sip_core::types::{Method, StatusCode};
    /// 
    /// let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
    ///     .from("Alice", "sip:alice@example.com", Some("1928301774"))
    ///     .to("Bob", "sip:bob@example.com", Some("a6c85cf"))
    ///     .authentication_info(
    ///         Some("dcd98b7102dd2f0e8b11d0f600bfb0c099"),
    ///         Some("auth"),
    ///         Some("6629fae49393a05397450978507c4ef1"),
    ///         Some("8dd675a9"),
    ///         Some("00000001")
    ///     )
    ///     .build();
    /// ```
    fn authentication_info(
        self,
        nextnonce: Option<&str>,
        qop: Option<&str>,
        rspauth: Option<&str>,
        cnonce: Option<&str>,
        nc: Option<&str>,
    ) -> Self;
}

impl<T> AuthenticationInfoExt for T 
where 
    T: HeaderSetter,
{
    fn authentication_info(
        self,
        nextnonce: Option<&str>,
        qop: Option<&str>,
        rspauth: Option<&str>,
        cnonce: Option<&str>,
        nc: Option<&str>,
    ) -> Self {
        let mut params = Vec::new();

        if let Some(nextnonce_val) = nextnonce {
            params.push(AuthenticationInfoParam::NextNonce(nextnonce_val.to_string()));
        }

        if let Some(qop_val) = qop {
            // Parse QOP type
            let qop_type = match qop_val.to_lowercase().as_str() {
                "auth" => Qop::Auth,
                "auth-int" => Qop::AuthInt,
                _ => Qop::Other(qop_val.to_string()),
            };
            params.push(AuthenticationInfoParam::Qop(qop_type));
        }

        if let Some(rspauth_val) = rspauth {
            params.push(AuthenticationInfoParam::ResponseAuth(rspauth_val.to_string()));
        }

        if let Some(cnonce_val) = cnonce {
            params.push(AuthenticationInfoParam::Cnonce(cnonce_val.to_string()));
        }

        if let Some(nc_val) = nc {
            if let Ok(nc_int) = u32::from_str_radix(nc_val.trim_start_matches("0x"), 16) {
                params.push(AuthenticationInfoParam::NonceCount(nc_int));
            }
        }

        // Only create and set header if at least one parameter is provided
        if !params.is_empty() {
            // Create the header using with_* methods
            let mut header = AuthenticationInfo::new();
            
            // Add all the parameters
            for param in params {
                match param {
                    AuthenticationInfoParam::NextNonce(val) => {
                        header = header.with_nextnonce(val);
                    },
                    AuthenticationInfoParam::Qop(val) => {
                        header = header.with_qop(val);
                    },
                    AuthenticationInfoParam::ResponseAuth(val) => {
                        header = header.with_rspauth(val);
                    },
                    AuthenticationInfoParam::Cnonce(val) => {
                        header = header.with_cnonce(val);
                    },
                    AuthenticationInfoParam::NonceCount(val) => {
                        header = header.with_nonce_count(val);
                    },
                }
            }
            
            self.set_header(header)
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::SimpleResponseBuilder;
    use crate::types::{Method, StatusCode};
    use crate::types::header::HeaderName;
    
    #[test]
    fn test_authentication_info() {
        let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
            .from("Alice", "sip:alice@example.com", Some("1928301774"))
            .to("Bob", "sip:bob@example.com", Some("a6c85cf"))
            .authentication_info(
                Some("dcd98b7102dd2f0e8b11d0f600bfb0c099"),
                Some("auth"),
                Some("6629fae49393a05397450978507c4ef1"),
                Some("8dd675a9"),
                Some("00000001")
            )
            .build();
            
        // Check if Authentication-Info header exists and has correct values
        let header = response.header(&HeaderName::AuthenticationInfo);
        assert!(header.is_some(), "Authentication-Info header not found");
        
        if let Some(TypedHeader::AuthenticationInfo(AuthenticationInfo(params))) = header {
            // Look for specific parameters in the output
            let has_nextnonce = params.iter().any(|p| {
                matches!(p, AuthenticationInfoParam::NextNonce(val) if val == "dcd98b7102dd2f0e8b11d0f600bfb0c099")
            });
            assert!(has_nextnonce, "NextNonce parameter not found or incorrect");
            
            let has_qop = params.iter().any(|p| {
                matches!(p, AuthenticationInfoParam::Qop(Qop::Auth))
            });
            assert!(has_qop, "Qop parameter not found or incorrect");
            
            let has_rspauth = params.iter().any(|p| {
                matches!(p, AuthenticationInfoParam::ResponseAuth(val) if val == "6629fae49393a05397450978507c4ef1")
            });
            assert!(has_rspauth, "ResponseAuth parameter not found or incorrect");
            
            let has_cnonce = params.iter().any(|p| {
                matches!(p, AuthenticationInfoParam::Cnonce(val) if val == "8dd675a9")
            });
            assert!(has_cnonce, "Cnonce parameter not found or incorrect");
            
            let has_nc = params.iter().any(|p| {
                matches!(p, AuthenticationInfoParam::NonceCount(val) if *val == 1)
            });
            assert!(has_nc, "NonceCount parameter not found or incorrect");
        } else {
            panic!("Expected Authentication-Info header");
        }
    }
    
    #[test]
    fn test_authentication_info_empty() {
        let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
            .authentication_info(None, None, None, None, None)
            .build();
            
        // Check that Authentication-Info header is NOT added when no parameters are provided
        let header = response.header(&HeaderName::AuthenticationInfo);
        assert!(header.is_none(), "Authentication-Info header should not be present");
    }
    
    #[test]
    fn test_authentication_info_partial() {
        let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
            .authentication_info(
                Some("dcd98b7102dd2f0e8b11d0f600bfb0c099"),
                None,
                None,
                None,
                None
            )
            .build();
            
        // Check if Authentication-Info header exists with just the nextnonce parameter
        let header = response.header(&HeaderName::AuthenticationInfo);
        assert!(header.is_some(), "Authentication-Info header not found");
        
        if let Some(TypedHeader::AuthenticationInfo(AuthenticationInfo(params))) = header {
            assert_eq!(params.len(), 1, "Expected only one parameter");
            
            let has_nextnonce = params.iter().any(|p| {
                matches!(p, AuthenticationInfoParam::NextNonce(val) if val == "dcd98b7102dd2f0e8b11d0f600bfb0c099")
            });
            assert!(has_nextnonce, "NextNonce parameter not found or incorrect");
        } else {
            panic!("Expected Authentication-Info header");
        }
    }
} 