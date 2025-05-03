use base64::{Engine as _, engine::general_purpose};
use crate::error::Result;
use crate::types::{
    auth::{
        Authorization,
        Challenge,
        DigestParam,
        AuthParam,
        Algorithm,
        Qop,
        Credentials,
    },
    TypedHeader,
    header::{TypedHeaderTrait, Header, HeaderName},
    headers::header_access::HeaderAccess,
}; 

impl Authorization {
    fn authorization_digest(
        self,
        username: &str,
        realm: &str,
        nonce: &str,
        response: &str,
        cnonce: Option<&str>,
        qop: Option<&str>,
        nc: Option<&str>,
        method: Option<&str>,
        uri: Option<&str>,
        algorithm: Option<&str>,
        opaque: Option<&str>,
    ) -> Self {
        // Create the params starting with mandatory fields
        let mut params = vec![
            AuthParam {
                name: "username".to_string(),
                value: username.to_string(),
            },
            AuthParam {
                name: "realm".to_string(),
                value: realm.to_string(),
            },
            AuthParam {
                name: "nonce".to_string(),
                value: nonce.to_string(),
            },
            AuthParam {
                name: "response".to_string(),
                value: response.to_string(),
            },
        ];

        // Add the URI if provided
        if let Some(uri_value) = uri {
            params.push(AuthParam {
                name: "uri".to_string(),
                value: uri_value.to_string(),
            });
        }

        // Add algorithm if provided
        if let Some(alg) = algorithm {
            params.push(AuthParam {
                name: "algorithm".to_string(),
                value: alg.to_string(),
            });
        }

        // Add qop, cnonce, and nc if provided (required by qop)
        if let Some(qop_value) = qop {
            params.push(AuthParam {
                name: "qop".to_string(),
                value: qop_value.to_string(),
            });

            // When qop is used, cnonce and nc must be provided
            if let Some(cnonce_value) = cnonce {
                params.push(AuthParam {
                    name: "cnonce".to_string(),
                    value: cnonce_value.to_string(),
                });
            }

            if let Some(nc_value) = nc {
                params.push(AuthParam {
                    name: "nc".to_string(),
                    value: nc_value.to_string(),
                });
            }
        }

        // Add opaque if provided
        if let Some(opaque_value) = opaque {
            params.push(AuthParam {
                name: "opaque".to_string(),
                value: opaque_value.to_string(),
            });
        }

        // Create the digest challenge response
        let credentials = Credentials::Digest { params };
        let header_value = Authorization(credentials);
        
        // Use the HeaderSetter trait to set the header
        self.set_header(header_value)
    }

    fn authorization_basic(self, username: &str, password: &str) -> Self {
        // Create the credentials string and base64 encode it
        let credentials = format!("{}:{}", username, password);
        let encoded = general_purpose::STANDARD.encode(credentials);
        
        // Create a Basic authorization with the encoded credentials
        let params = vec![
            AuthParam {
                name: "credentials".to_string(),
                value: encoded,
            },
        ];
        
        let credentials = Credentials::Basic { token: encoded };
        let header_value = Authorization(credentials);
        
        // Use the HeaderSetter trait to set the header
        self.set_header(header_value)
    }
} 