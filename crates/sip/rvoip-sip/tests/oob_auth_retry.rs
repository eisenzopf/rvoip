//! Out-of-dialog MESSAGE / OPTIONS / SUBSCRIBE auth retry.

mod support;

use std::time::Duration;

use rvoip_sip::types::Credentials;
use rvoip_sip::{Config, Result, SessionError, SipClientAuth, UnifiedCoordinator};
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::HeaderAccess;
use rvoip_sip_core::types::Message;

use support::{boot_auth_uas, CapturedAuthRequest, ChallengeReply};

fn challenge_once(realm: &'static str, nonce: &'static str) -> impl Fn(u32) -> ChallengeReply {
    move |idx| {
        if idx == 0 {
            ChallengeReply::Challenge401 {
                realm: realm.to_string(),
                nonce: nonce.to_string(),
            }
        } else {
            ChallengeReply::Ok
        }
    }
}

fn assert_digest_retry(captured: &[CapturedAuthRequest], method: &str, nonce: &str) {
    assert_eq!(
        captured.len(),
        2,
        "expected initial request plus auth retry"
    );
    assert_eq!(captured[0].method, method);
    assert_eq!(captured[1].method, method);
    assert!(
        captured[1].cseq > captured[0].cseq,
        "RFC 3261 §22.2 retry must increment CSeq for {method}: initial={}, retry={}",
        captured[0].cseq,
        captured[1].cseq
    );
    assert_eq!(
        captured[1].call_id, captured[0].call_id,
        "RFC 3261 §8.1.3.5 retry should preserve Call-ID for {method}"
    );
    assert_eq!(
        captured[1].from_tag, captured[0].from_tag,
        "RFC 3261 §8.1.3.5 retry should preserve From tag for {method}"
    );
    assert_eq!(
        captured[1].to_header, captured[0].to_header,
        "RFC 3261 §8.1.3.5 retry should preserve To for {method}"
    );
    assert_ne!(
        captured[1].via_header, captured[0].via_header,
        "RFC 3261 §22.2 auth retry should use a fresh client transaction Via for {method}"
    );
    assert!(
        !captured[0].raw.contains("Authorization:"),
        "initial {method} must not carry Authorization: {}",
        captured[0].raw
    );
    let retry = &captured[1].raw;
    assert!(
        retry.contains("Authorization: Digest "),
        "retry {method} must carry digest Authorization: {retry}"
    );
    assert!(
        retry.contains(r#"username="alice""#),
        "retry {method} must include username: {retry}"
    );
    assert!(
        retry.contains(r#"response=""#),
        "retry {method} must include computed response: {retry}"
    );
    assert!(
        retry.contains(&format!(r#"nonce="{nonce}""#)),
        "retry {method} must echo nonce: {retry}"
    );
}

fn challenge_once_proxy(
    realm: &'static str,
    nonce: &'static str,
) -> impl Fn(u32) -> ChallengeReply {
    move |idx| {
        if idx == 0 {
            ChallengeReply::Challenge407 {
                realm: realm.to_string(),
                nonce: nonce.to_string(),
            }
        } else {
            ChallengeReply::Ok
        }
    }
}

fn request_header(raw: &str, header_name: HeaderName) -> Option<String> {
    let message = parse_message(raw.as_bytes()).expect("captured SIP request parses");
    let Message::Request(request) = message else {
        panic!("captured message was not a request");
    };
    request.raw_header_value(&header_name)
}

fn assert_proxy_digest_retry(captured: &[CapturedAuthRequest], method: &str, nonce: &str) {
    assert_eq!(
        captured.len(),
        2,
        "expected initial request plus proxy-auth retry"
    );
    assert_eq!(captured[0].method, method);
    assert_eq!(captured[1].method, method);
    assert!(
        captured[1].cseq > captured[0].cseq,
        "RFC 3261 §22.2 retry must increment CSeq for {method}: initial={}, retry={}",
        captured[0].cseq,
        captured[1].cseq
    );
    assert_eq!(
        captured[1].call_id, captured[0].call_id,
        "RFC 3261 §8.1.3.5 retry should preserve Call-ID for {method}"
    );
    assert_eq!(
        captured[1].from_tag, captured[0].from_tag,
        "RFC 3261 §8.1.3.5 retry should preserve From tag for {method}"
    );
    assert_eq!(
        captured[1].to_header, captured[0].to_header,
        "RFC 3261 §8.1.3.5 retry should preserve To for {method}"
    );
    assert_ne!(
        captured[1].via_header, captured[0].via_header,
        "RFC 3261 §22.2 auth retry should use a fresh client transaction Via for {method}"
    );
    assert!(
        !captured[0].raw.contains("Authorization:"),
        "initial {method} must not carry auth headers: {}",
        captured[0].raw
    );
    let retry = &captured[1].raw;
    assert!(
        retry.contains("Proxy-Authorization: Digest "),
        "407 retry {method} must carry Proxy-Authorization: {retry}"
    );
    assert!(
        !retry.contains("\nAuthorization: Digest "),
        "407 retry {method} must not carry WWW Authorization: {retry}"
    );
    assert!(
        retry.contains(&format!(r#"nonce="{nonce}""#)),
        "retry {method} must echo nonce: {retry}"
    );
}

#[tokio::test]
async fn message_with_credentials_retries_with_full_digest() -> Result<()> {
    let uas = boot_auth_uas(16360, challenge_once("oob-message", "msg-nonce")).await;
    let coord = UnifiedCoordinator::new(Config::local("alice", 16361)).await?;

    coord
        .message(format!("sip:bob@{}", uas.addr))
        .with_body("hello")
        .with_credentials(Credentials::new("alice", "password"))
        .send()
        .await?;

    let captured = uas.wait_for_n(2, Duration::from_secs(8)).await;
    assert_digest_retry(&captured, "MESSAGE", "msg-nonce");
    coord.shutdown_gracefully(None).await?;
    uas.shutdown();
    Ok(())
}

#[tokio::test]
async fn options_with_credentials_retries_with_full_digest() -> Result<()> {
    let uas = boot_auth_uas(16362, challenge_once("oob-options", "opt-nonce")).await;
    let coord = UnifiedCoordinator::new(Config::local("alice", 16363)).await?;

    let response = coord
        .options(format!("sip:bob@{}", uas.addr))
        .with_credentials(Credentials::new("alice", "password"))
        .send()
        .await?;

    assert_eq!(response.status_code, 200);
    let captured = uas.wait_for_n(2, Duration::from_secs(8)).await;
    assert_digest_retry(&captured, "OPTIONS", "opt-nonce");
    coord.shutdown_gracefully(None).await?;
    uas.shutdown();
    Ok(())
}

#[tokio::test]
async fn subscribe_with_credentials_retries_with_full_digest() -> Result<()> {
    let uas = boot_auth_uas(16364, challenge_once("oob-subscribe", "sub-nonce")).await;
    let coord = UnifiedCoordinator::new(Config::local("alice", 16365)).await?;

    let _handle = coord
        .subscribe(format!("sip:bob@{}", uas.addr), "presence")
        .with_credentials(Credentials::new("alice", "password"))
        .send()
        .await?;

    let captured = uas.wait_for_n(2, Duration::from_secs(8)).await;
    assert_digest_retry(&captured, "SUBSCRIBE", "sub-nonce");
    coord.shutdown_gracefully(None).await?;
    uas.shutdown();
    Ok(())
}

#[tokio::test]
async fn message_with_bearer_token_retries_with_bearer_authorization() -> Result<()> {
    let uas = boot_auth_uas(16380, |idx| {
        if idx == 0 {
            ChallengeReply::Challenge401Raw(r#"Bearer realm="api", scope="calls""#.to_string())
        } else {
            ChallengeReply::Ok
        }
    })
    .await;
    let coord = UnifiedCoordinator::new(Config::local("alice", 16381)).await?;

    coord
        .message(format!("sip:bob@{}", uas.addr))
        .with_body("hello")
        .with_auth(SipClientAuth::bearer_token("token-123").allow_bearer_over_cleartext(true))
        .send()
        .await?;

    let captured = uas.wait_for_n(2, Duration::from_secs(8)).await;
    assert_eq!(captured.len(), 2);
    assert!(
        !captured[0].raw.contains("Authorization:"),
        "initial MESSAGE must not carry Authorization: {}",
        captured[0].raw
    );
    let authorization = request_header(&captured[1].raw, HeaderName::Authorization)
        .expect("retry carries Authorization");
    assert_eq!(authorization, "Bearer token-123");
    coord.shutdown_gracefully(None).await?;
    uas.shutdown();
    Ok(())
}

#[tokio::test]
async fn message_with_basic_auth_requires_explicit_cleartext_opt_in() -> Result<()> {
    let uas = boot_auth_uas(16382, |idx| {
        if idx == 0 {
            ChallengeReply::Challenge401Raw(r#"Basic realm="legacy""#.to_string())
        } else {
            ChallengeReply::Ok
        }
    })
    .await;
    let coord = UnifiedCoordinator::new(Config::local("alice", 16383)).await?;

    let err = coord
        .message(format!("sip:bob@{}", uas.addr))
        .with_body("hello")
        .with_basic_credentials("alice", "secret")
        .send()
        .await
        .expect_err("Basic over cleartext should be rejected by default");

    assert!(
        matches!(err, SessionError::RequestAuthConstructionFailed),
        "cleartext Basic rejection must retain the typed outbound-auth class: {err}"
    );
    let captured = uas.wait_for_n(1, Duration::from_secs(2)).await;
    assert_eq!(
        captured.len(),
        1,
        "cleartext Basic rejection must not send an authenticated retry"
    );
    coord.shutdown_gracefully(None).await?;
    uas.shutdown();
    Ok(())
}

#[tokio::test]
async fn message_with_basic_auth_retries_when_cleartext_opted_in() -> Result<()> {
    let uas = boot_auth_uas(16384, |idx| {
        if idx == 0 {
            ChallengeReply::Challenge401Raw(r#"Basic realm="legacy""#.to_string())
        } else {
            ChallengeReply::Ok
        }
    })
    .await;
    let coord = UnifiedCoordinator::new(Config::local("alice", 16385)).await?;

    coord
        .message(format!("sip:bob@{}", uas.addr))
        .with_body("hello")
        .with_auth(SipClientAuth::basic("alice", "secret").allow_basic_over_cleartext(true))
        .send()
        .await?;

    let captured = uas.wait_for_n(2, Duration::from_secs(8)).await;
    let authorization = request_header(&captured[1].raw, HeaderName::Authorization)
        .expect("retry carries Authorization");
    assert_eq!(authorization, "Basic YWxpY2U6c2VjcmV0");
    coord.shutdown_gracefully(None).await?;
    uas.shutdown();
    Ok(())
}

#[tokio::test]
async fn message_composite_auth_picks_bearer_over_digest() -> Result<()> {
    let uas = boot_auth_uas(16386, |idx| {
        if idx == 0 {
            ChallengeReply::Challenge401Raw(
                r#"Digest realm="pbx", nonce="digest-nonce", algorithm=MD5, Bearer realm="api""#
                    .to_string(),
            )
        } else {
            ChallengeReply::Ok
        }
    })
    .await;
    let coord = UnifiedCoordinator::new(Config::local("alice", 16387)).await?;

    coord
        .message(format!("sip:bob@{}", uas.addr))
        .with_body("hello")
        .with_auth(SipClientAuth::any([
            SipClientAuth::digest("alice", "password"),
            SipClientAuth::bearer_token("stronger-token").allow_bearer_over_cleartext(true),
        ]))
        .send()
        .await?;

    let captured = uas.wait_for_n(2, Duration::from_secs(8)).await;
    let authorization = request_header(&captured[1].raw, HeaderName::Authorization)
        .expect("retry carries Authorization");
    assert_eq!(authorization, "Bearer stronger-token");
    coord.shutdown_gracefully(None).await?;
    uas.shutdown();
    Ok(())
}

#[tokio::test]
async fn message_with_credentials_retries_407_with_proxy_authorization() -> Result<()> {
    let uas = boot_auth_uas(16366, challenge_once_proxy("oob-message", "msg-proxy")).await;
    let coord = UnifiedCoordinator::new(Config::local("alice", 16367)).await?;

    coord
        .message(format!("sip:bob@{}", uas.addr))
        .with_body("hello")
        .with_credentials(Credentials::new("alice", "password"))
        .send()
        .await?;

    let captured = uas.wait_for_n(2, Duration::from_secs(8)).await;
    assert_proxy_digest_retry(&captured, "MESSAGE", "msg-proxy");
    coord.shutdown_gracefully(None).await?;
    uas.shutdown();
    Ok(())
}

#[tokio::test]
async fn options_with_credentials_retries_407_with_proxy_authorization() -> Result<()> {
    let uas = boot_auth_uas(16368, challenge_once_proxy("oob-options", "opt-proxy")).await;
    let coord = UnifiedCoordinator::new(Config::local("alice", 16369)).await?;

    coord
        .options(format!("sip:bob@{}", uas.addr))
        .with_credentials(Credentials::new("alice", "password"))
        .send()
        .await?;

    let captured = uas.wait_for_n(2, Duration::from_secs(8)).await;
    assert_proxy_digest_retry(&captured, "OPTIONS", "opt-proxy");
    coord.shutdown_gracefully(None).await?;
    uas.shutdown();
    Ok(())
}

#[tokio::test]
async fn subscribe_with_credentials_retries_407_with_proxy_authorization() -> Result<()> {
    let uas = boot_auth_uas(16370, challenge_once_proxy("oob-subscribe", "sub-proxy")).await;
    let coord = UnifiedCoordinator::new(Config::local("alice", 16371)).await?;

    let _handle = coord
        .subscribe(format!("sip:bob@{}", uas.addr), "presence")
        .with_credentials(Credentials::new("alice", "password"))
        .send()
        .await?;

    let captured = uas.wait_for_n(2, Duration::from_secs(8)).await;
    assert_proxy_digest_retry(&captured, "SUBSCRIBE", "sub-proxy");
    coord.shutdown_gracefully(None).await?;
    uas.shutdown();
    Ok(())
}

#[tokio::test]
async fn message_with_credentials_recovers_once_from_stale_nonce() -> Result<()> {
    let uas = boot_auth_uas(16372, |idx| match idx {
        0 => ChallengeReply::Challenge401 {
            realm: "oob-message".to_string(),
            nonce: "old-nonce".to_string(),
        },
        1 => ChallengeReply::Challenge401Full {
            realm: "oob-message".to_string(),
            nonce: "fresh-nonce".to_string(),
            algorithm: "MD5".to_string(),
            qop: Some("auth".to_string()),
            stale: true,
        },
        _ => ChallengeReply::Ok,
    })
    .await;
    let coord = UnifiedCoordinator::new(Config::local("alice", 16373)).await?;

    coord
        .message(format!("sip:bob@{}", uas.addr))
        .with_body("hello")
        .with_credentials(Credentials::new("alice", "password"))
        .send()
        .await?;

    let captured = uas.wait_for_n(3, Duration::from_secs(8)).await;
    assert_eq!(captured.len(), 3);
    assert!(
        captured[1].cseq > captured[0].cseq && captured[2].cseq > captured[1].cseq,
        "RFC 3261 §22.2 stale retry must increment CSeq each time: initial={}, retry={}, stale_retry={}",
        captured[0].cseq,
        captured[1].cseq,
        captured[2].cseq
    );
    assert!(
        captured
            .iter()
            .all(|capture| capture.call_id == captured[0].call_id),
        "RFC 3261 §8.1.3.5 stale retry should preserve Call-ID across all attempts"
    );
    assert!(
        captured
            .iter()
            .all(|capture| capture.from_tag == captured[0].from_tag),
        "RFC 3261 §8.1.3.5 stale retry should preserve From tag across all attempts"
    );
    assert!(
        captured
            .iter()
            .all(|capture| capture.to_header == captured[0].to_header),
        "RFC 3261 §8.1.3.5 stale retry should preserve To across all attempts"
    );
    assert!(
        captured
            .windows(2)
            .all(|pair| pair[1].via_header != pair[0].via_header),
        "RFC 3261 §22.2 stale retry should use a fresh client transaction Via on each retry"
    );
    assert!(captured[1].raw.contains(r#"nonce="old-nonce""#));
    assert!(captured[2].raw.contains(r#"nonce="fresh-nonce""#));
    assert!(captured[2].raw.contains("nc=00000001"));
    coord.shutdown_gracefully(None).await?;
    uas.shutdown();
    Ok(())
}

#[tokio::test]
async fn message_with_credentials_rejects_unsupported_digest_algorithm() -> Result<()> {
    let uas = boot_auth_uas(16374, |idx| {
        if idx == 0 {
            ChallengeReply::Challenge401Full {
                realm: "oob-message".to_string(),
                nonce: "bad-algorithm".to_string(),
                algorithm: "SHA-999".to_string(),
                qop: Some("auth".to_string()),
                stale: false,
            }
        } else {
            ChallengeReply::Ok
        }
    })
    .await;
    let coord = UnifiedCoordinator::new(Config::local("alice", 16375)).await?;

    let result = coord
        .message(format!("sip:bob@{}", uas.addr))
        .with_body("hello")
        .with_credentials(Credentials::new("alice", "password"))
        .send()
        .await;

    assert!(result.is_err(), "unsupported algorithm must fail");
    let captured = uas.wait_for_n(1, Duration::from_secs(8)).await;
    assert_eq!(captured.len(), 1, "must not retry unsupported algorithm");
    coord.shutdown_gracefully(None).await?;
    uas.shutdown();
    Ok(())
}

#[tokio::test]
async fn message_with_credentials_uses_auth_int_when_offered_with_body() -> Result<()> {
    let uas = boot_auth_uas(16376, |idx| {
        if idx == 0 {
            ChallengeReply::Challenge401Full {
                realm: "oob-message".to_string(),
                nonce: "auth-int-nonce".to_string(),
                algorithm: "MD5".to_string(),
                qop: Some("auth-int".to_string()),
                stale: false,
            }
        } else {
            ChallengeReply::Ok
        }
    })
    .await;
    let coord = UnifiedCoordinator::new(Config::local("alice", 16377)).await?;

    coord
        .message(format!("sip:bob@{}", uas.addr))
        .with_body("hello")
        .with_credentials(Credentials::new("alice", "password"))
        .send()
        .await?;

    let captured = uas.wait_for_n(2, Duration::from_secs(8)).await;
    let authorization = request_header(&captured[1].raw, HeaderName::Authorization)
        .expect("retry Authorization header");
    let parsed = rvoip_sip::auth::DigestAuthenticator::parse_authorization(&authorization)
        .expect("parse Authorization");
    assert_eq!(parsed.qop.as_deref(), Some("auth-int"));
    assert!(
        rvoip_sip::auth::DigestAuthenticator::new("oob-message")
            .validate_response_with_body(&parsed, "MESSAGE", "password", Some(b"hello"))
            .expect("auth-int validation"),
        "auth-int response must validate against body"
    );
    coord.shutdown_gracefully(None).await?;
    uas.shutdown();
    Ok(())
}
