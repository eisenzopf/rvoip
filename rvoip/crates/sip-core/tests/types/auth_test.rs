// Tests for Authentication related types

use crate::common::{assert_parses_ok, assert_parse_fails, assert_display_parses_back, uri};
use rvoip_sip_core::types::auth::{Scheme, Algorithm, Qop, WwwAuthenticate, ProxyAuthenticate, Authorization, AuthenticationInfo, ProxyAuthorization};
use rvoip_sip_core::uri::Uri;
use std::str::FromStr;

#[test]
fn test_auth_enums_display() {
    assert_eq!(Scheme::Digest.to_string(), "Digest");
    assert_eq!(Scheme::Other("Custom".to_string()).to_string(), "Custom");

    assert_eq!(Algorithm::Md5.to_string(), "MD5");
    assert_eq!(Algorithm::Sha256.to_string(), "SHA-256");
    assert_eq!(Algorithm::Sha512Sess.to_string(), "SHA-512-256-sess");

    assert_eq!(Qop::Auth.to_string(), "auth");
    assert_eq!(Qop::AuthInt.to_string(), "auth-int");
}

#[test]
fn test_www_authenticate_display_parse_roundtrip() {
    let auth1 = WwwAuthenticate {
        scheme: Scheme::Digest,
        realm: "example.com".to_string(),
        domain: None,
        nonce: "nonce123".to_string(),
        opaque: Some("opaque456".to_string()),
        stale: Some(false),
        algorithm: Some(Algorithm::Md5),
        qop: vec![Qop::Auth],
    };
    assert_display_parses_back(&auth1);

    let auth2 = WwwAuthenticate {
        scheme: Scheme::Digest,
        realm: "secure.com".to_string(),
        domain: Some("/sip/users".to_string()),
        nonce: "nonceABC".to_string(),
        opaque: None,
        stale: None,
        algorithm: Some(Algorithm::Sha256),
        qop: vec![Qop::Auth, Qop::AuthInt],
    };
    assert_display_parses_back(&auth2);
     
    // Test ProxyAuthenticate Display delegates
    let proxy_auth = ProxyAuthenticate(auth1.clone());
    assert_eq!(proxy_auth.to_string(), auth1.to_string());
    // Test ProxyAuthenticate FromStr
    assert_parses_ok(
        "Digest realm=\"proxy.com\", nonce=\"pnonce\"", 
        ProxyAuthenticate(WwwAuthenticate {
            scheme: Scheme::Digest,
            realm: "proxy.com".to_string(),
            nonce: "pnonce".to_string(),
            domain: None, opaque: None, stale: None, algorithm: None, qop: vec![]
        })
    );
    assert_parse_fails::<ProxyAuthenticate>("Digest nonce=\"pnonce\"");
}

#[test]
fn test_authorization_display_parse_roundtrip() {
    let auth1 = Authorization {
        scheme: Scheme::Digest,
        username: "bob".to_string(),
        realm: "biloxi.example.com".to_string(),
        nonce: "dcd98b7102dd2f0e8b11d0f600bfb0c093".to_string(),
        uri: uri("sip:bob@biloxi.example.com"),
        response: "245f2341c95403d85a1aeae87d33a3e4".to_string(),
        algorithm: Some(Algorithm::Md5),
        cnonce: Some("0a4f113b".to_string()),
        opaque: Some("5ccc069c403ebaf9f0171e9517f40e41".to_string()),
        message_qop: Some(Qop::Auth),
        nonce_count: Some(1),
    };
    assert_display_parses_back(&auth1);
    
    // Test minimum fields
    let auth2 = Authorization {
        scheme: Scheme::Digest,
        username: "alice".to_string(),
        realm: "atlanta.com".to_string(),
        nonce: "nonce1".to_string(),
        uri: uri("sip:target@atlanta.com"),
        response: "resp1".to_string(),
        algorithm: None,
        cnonce: None,
        opaque: None,
        message_qop: None,
        nonce_count: None,
    };
    assert_display_parses_back(&auth2);
    
    // Test ProxyAuthorization delegation
    let proxy_authz = ProxyAuthorization(auth1.clone());
    assert_eq!(proxy_authz.to_string(), auth1.to_string());
    // Test ProxyAuthorization FromStr
     assert_parses_ok(
        "Digest username=\"pu\", realm=\"pr\", nonce=\"pn\", uri=\"sip:a@b\", response=\"pr\"", 
        ProxyAuthorization(Authorization {
            scheme: Scheme::Digest, username: "pu".to_string(), realm: "pr".to_string(), 
            nonce: "pn".to_string(), uri: uri("sip:a@b"), response: "pr".to_string(),
            algorithm: None, cnonce: None, opaque: None, message_qop: None, nonce_count: None
        })
    );
     assert_parse_fails::<ProxyAuthorization>("Digest username=\"pu\"");
}

#[test]
fn test_authentication_info_display_parse_roundtrip() {
    let info1 = AuthenticationInfo {
        nextnonce: Some("nonce123".to_string()),
        qop: Some(Qop::Auth),
        rspauth: Some("rsp456".to_string()),
        cnonce: Some("cnonce789".to_string()),
        nc: Some(1), // Decimal 1
    };
    assert_display_parses_back(&info1);

    let info2 = AuthenticationInfo {
        nextnonce: None,
        qop: Some(Qop::AuthInt),
        rspauth: Some("abc".to_string()),
        cnonce: None,
        nc: Some(8), // Decimal 8
    };
    assert_display_parses_back(&info2);

    let info3 = AuthenticationInfo {
        nextnonce: Some("next1".to_string()),
        qop: None,
        rspauth: None,
        cnonce: None,
        nc: None,
    };
    assert_display_parses_back(&info3);
    
    // Test FromStr failures
     assert_parse_fails::<AuthenticationInfo>("nc=bad");
}

#[test]
fn test_auth_builder_methods() {
    // Test WwwAuthenticate builders
    let www_auth = WwwAuthenticate::new(Scheme::Digest, "realm1", "nonce1")
        .with_algorithm(Algorithm::Sha256)
        .with_qop(Qop::Auth)
        .with_qop(Qop::AuthInt) // Add multiple
        .with_opaque("opaque1")
        .with_stale(true)
        .with_domain("domain1");
        
    assert_eq!(www_auth.algorithm, Some(Algorithm::Sha256));
    assert_eq!(www_auth.qop, vec![Qop::Auth, Qop::AuthInt]);
    assert_eq!(www_auth.opaque.as_deref(), Some("opaque1"));
    assert_eq!(www_auth.stale, Some(true));
    assert_eq!(www_auth.domain.as_deref(), Some("domain1"));

    // Test Authorization builders
    let authz = Authorization::new(Scheme::Digest, "user", "realm", "nonce", uri("sip:a@b"), "resp")
        .with_algorithm(Algorithm::Md5Sess)
        .with_qop(Qop::AuthInt)
        .with_cnonce("cnonce1")
        .with_opaque("opaque2")
        .with_nonce_count(5);
        
    assert_eq!(authz.algorithm, Some(Algorithm::Md5Sess));
    assert_eq!(authz.message_qop, Some(Qop::AuthInt));
    assert_eq!(authz.cnonce.as_deref(), Some("cnonce1"));
    assert_eq!(authz.opaque.as_deref(), Some("opaque2"));
    assert_eq!(authz.nonce_count, Some(5));

    // Test AuthenticationInfo builders
    let auth_info = AuthenticationInfo::new()
        .with_nextnonce("nextnonce1")
        .with_qop(Qop::Auth)
        .with_rspauth("rspauth1")
        .with_cnonce("cnonce2")
        .with_nonce_count(8); // Decimal 8
        
    assert_eq!(auth_info.nextnonce.as_deref(), Some("nextnonce1"));
    assert_eq!(auth_info.qop, Some(Qop::Auth));
    assert_eq!(auth_info.rspauth.as_deref(), Some("rspauth1"));
    assert_eq!(auth_info.cnonce.as_deref(), Some("cnonce2"));
    assert_eq!(auth_info.nc, Some(8));
}

// Removed old separate display/from_str tests 