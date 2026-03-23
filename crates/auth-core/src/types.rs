//! Core types for authentication

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Represents an authenticated user context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserContext {
    /// Unique user identifier
    pub user_id: String,
    
    /// Username or email
    pub username: String,
    
    /// User's roles
    pub roles: Vec<String>,
    
    /// Additional claims from the token
    pub claims: HashMap<String, serde_json::Value>,
    
    /// Token expiration time (Unix timestamp)
    pub expires_at: Option<i64>,
    
    /// OAuth2 scopes
    pub scopes: Vec<String>,
}

/// Token types supported by the system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TokenType {
    Bearer,
    JWT,
    Opaque,
}

/// Authentication method
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthMethod {
    OAuth2,
    JWT,
    ApiKey,
    SIPDigest,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    // --- UserContext tests ---

    #[test]
    fn user_context_new_with_all_fields() {
        let ctx = UserContext {
            user_id: "u-123".to_string(),
            username: "alice@example.com".to_string(),
            roles: vec!["admin".to_string(), "agent".to_string()],
            claims: HashMap::from([
                ("org".to_string(), serde_json::json!("acme")),
            ]),
            expires_at: Some(1_700_000_000),
            scopes: vec!["read".to_string(), "write".to_string()],
        };
        assert_eq!(ctx.user_id, "u-123");
        assert_eq!(ctx.username, "alice@example.com");
        assert_eq!(ctx.roles.len(), 2);
        assert!(ctx.roles.contains(&"admin".to_string()));
        assert_eq!(ctx.expires_at, Some(1_700_000_000));
        assert_eq!(ctx.scopes.len(), 2);
    }

    #[test]
    fn user_context_with_no_expiration() {
        let ctx = UserContext {
            user_id: "u-456".to_string(),
            username: "bob".to_string(),
            roles: vec![],
            claims: HashMap::new(),
            expires_at: None,
            scopes: vec![],
        };
        assert!(ctx.expires_at.is_none());
        assert!(ctx.roles.is_empty());
        assert!(ctx.scopes.is_empty());
    }

    #[test]
    fn user_context_clone() {
        let ctx = UserContext {
            user_id: "u-1".to_string(),
            username: "user".to_string(),
            roles: vec!["role".to_string()],
            claims: HashMap::new(),
            expires_at: Some(999),
            scopes: vec![],
        };
        let cloned = ctx.clone();
        assert_eq!(cloned.user_id, ctx.user_id);
        assert_eq!(cloned.username, ctx.username);
        assert_eq!(cloned.roles, ctx.roles);
        assert_eq!(cloned.expires_at, ctx.expires_at);
    }

    #[test]
    fn user_context_debug_format() {
        let ctx = UserContext {
            user_id: "u-dbg".to_string(),
            username: "debug_user".to_string(),
            roles: vec![],
            claims: HashMap::new(),
            expires_at: None,
            scopes: vec![],
        };
        let debug = format!("{:?}", ctx);
        assert!(debug.contains("u-dbg"));
        assert!(debug.contains("debug_user"));
    }

    #[test]
    fn user_context_serializes_to_json() {
        let ctx = UserContext {
            user_id: "u-ser".to_string(),
            username: "serializer".to_string(),
            roles: vec!["viewer".to_string()],
            claims: HashMap::from([
                ("tenant".to_string(), serde_json::json!("t1")),
            ]),
            expires_at: Some(1_800_000_000),
            scopes: vec!["read".to_string()],
        };
        let json = serde_json::to_string(&ctx).unwrap();
        assert!(json.contains("\"user_id\":\"u-ser\""));
        assert!(json.contains("\"username\":\"serializer\""));
        assert!(json.contains("\"viewer\""));
        assert!(json.contains("1800000000"));
    }

    #[test]
    fn user_context_deserializes_from_json() {
        let json = r#"{
            "user_id": "u-de",
            "username": "deser",
            "roles": ["admin"],
            "claims": {"key": "value"},
            "expires_at": 12345,
            "scopes": ["scope1"]
        }"#;
        let ctx: UserContext = serde_json::from_str(json).unwrap();
        assert_eq!(ctx.user_id, "u-de");
        assert_eq!(ctx.username, "deser");
        assert_eq!(ctx.roles, vec!["admin"]);
        assert_eq!(ctx.claims.get("key"), Some(&serde_json::json!("value")));
        assert_eq!(ctx.expires_at, Some(12345));
        assert_eq!(ctx.scopes, vec!["scope1"]);
    }

    #[test]
    fn user_context_deserializes_with_null_expiration() {
        let json = r#"{
            "user_id": "u-null",
            "username": "nullexp",
            "roles": [],
            "claims": {},
            "expires_at": null,
            "scopes": []
        }"#;
        let ctx: UserContext = serde_json::from_str(json).unwrap();
        assert!(ctx.expires_at.is_none());
    }

    #[test]
    fn user_context_roundtrip_serialization() {
        let original = UserContext {
            user_id: "u-rt".to_string(),
            username: "roundtrip".to_string(),
            roles: vec!["r1".to_string(), "r2".to_string()],
            claims: HashMap::from([
                ("num".to_string(), serde_json::json!(42)),
                ("flag".to_string(), serde_json::json!(true)),
            ]),
            expires_at: Some(9_999_999),
            scopes: vec!["all".to_string()],
        };
        let json = serde_json::to_string(&original).unwrap();
        let restored: UserContext = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.user_id, original.user_id);
        assert_eq!(restored.username, original.username);
        assert_eq!(restored.roles, original.roles);
        assert_eq!(restored.expires_at, original.expires_at);
        assert_eq!(restored.scopes, original.scopes);
    }

    #[test]
    fn user_context_with_complex_claims() {
        let ctx = UserContext {
            user_id: "u-complex".to_string(),
            username: "complex".to_string(),
            roles: vec![],
            claims: HashMap::from([
                ("nested".to_string(), serde_json::json!({"a": 1, "b": [2, 3]})),
                ("array".to_string(), serde_json::json!([1, "two", null])),
                ("null_val".to_string(), serde_json::Value::Null),
            ]),
            expires_at: None,
            scopes: vec![],
        };
        assert_eq!(ctx.claims.len(), 3);
        let json = serde_json::to_string(&ctx).unwrap();
        let restored: UserContext = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.claims.len(), 3);
    }

    // --- TokenType tests ---

    #[test]
    fn token_type_bearer_debug() {
        let t = TokenType::Bearer;
        assert!(format!("{:?}", t).contains("Bearer"));
    }

    #[test]
    fn token_type_jwt_debug() {
        let t = TokenType::JWT;
        assert!(format!("{:?}", t).contains("JWT"));
    }

    #[test]
    fn token_type_opaque_debug() {
        let t = TokenType::Opaque;
        assert!(format!("{:?}", t).contains("Opaque"));
    }

    #[test]
    fn token_type_clone() {
        let t = TokenType::JWT;
        let cloned = t.clone();
        assert!(format!("{:?}", cloned).contains("JWT"));
    }

    #[test]
    fn token_type_serializes_to_json() {
        let bearer_json = serde_json::to_string(&TokenType::Bearer).unwrap();
        let jwt_json = serde_json::to_string(&TokenType::JWT).unwrap();
        let opaque_json = serde_json::to_string(&TokenType::Opaque).unwrap();
        assert_eq!(bearer_json, "\"Bearer\"");
        assert_eq!(jwt_json, "\"JWT\"");
        assert_eq!(opaque_json, "\"Opaque\"");
    }

    #[test]
    fn token_type_deserializes_from_json() {
        let bearer: TokenType = serde_json::from_str("\"Bearer\"").unwrap();
        assert!(matches!(bearer, TokenType::Bearer));
        let jwt: TokenType = serde_json::from_str("\"JWT\"").unwrap();
        assert!(matches!(jwt, TokenType::JWT));
        let opaque: TokenType = serde_json::from_str("\"Opaque\"").unwrap();
        assert!(matches!(opaque, TokenType::Opaque));
    }

    #[test]
    fn token_type_invalid_deserialization() {
        let result = serde_json::from_str::<TokenType>("\"Unknown\"");
        assert!(result.is_err());
    }

    // --- AuthMethod tests ---

    #[test]
    fn auth_method_all_variants_debug() {
        assert!(format!("{:?}", AuthMethod::OAuth2).contains("OAuth2"));
        assert!(format!("{:?}", AuthMethod::JWT).contains("JWT"));
        assert!(format!("{:?}", AuthMethod::ApiKey).contains("ApiKey"));
        assert!(format!("{:?}", AuthMethod::SIPDigest).contains("SIPDigest"));
    }

    #[test]
    fn auth_method_clone() {
        let m = AuthMethod::SIPDigest;
        let cloned = m.clone();
        assert!(matches!(cloned, AuthMethod::SIPDigest));
    }

    #[test]
    fn auth_method_serializes_to_json() {
        assert_eq!(serde_json::to_string(&AuthMethod::OAuth2).unwrap(), "\"OAuth2\"");
        assert_eq!(serde_json::to_string(&AuthMethod::JWT).unwrap(), "\"JWT\"");
        assert_eq!(serde_json::to_string(&AuthMethod::ApiKey).unwrap(), "\"ApiKey\"");
        assert_eq!(serde_json::to_string(&AuthMethod::SIPDigest).unwrap(), "\"SIPDigest\"");
    }

    #[test]
    fn auth_method_deserializes_from_json() {
        let oauth: AuthMethod = serde_json::from_str("\"OAuth2\"").unwrap();
        assert!(matches!(oauth, AuthMethod::OAuth2));
        let apikey: AuthMethod = serde_json::from_str("\"ApiKey\"").unwrap();
        assert!(matches!(apikey, AuthMethod::ApiKey));
    }

    #[test]
    fn auth_method_invalid_deserialization() {
        let result = serde_json::from_str::<AuthMethod>("\"Password\"");
        assert!(result.is_err());
    }

    #[test]
    fn auth_method_roundtrip_serialization() {
        let methods = vec![
            AuthMethod::OAuth2,
            AuthMethod::JWT,
            AuthMethod::ApiKey,
            AuthMethod::SIPDigest,
        ];
        for method in methods {
            let json = serde_json::to_string(&method).unwrap();
            let restored: AuthMethod = serde_json::from_str(&json).unwrap();
            assert_eq!(format!("{:?}", method), format!("{:?}", restored));
        }
    }
}