//! SIP_API_DESIGN_2 Phase B — the structured per-call INVITE overrides
//! (`with_from_display`, `with_subject`, `with_contact_uri`, `with_pai`)
//! reach the wire AND survive a 401-driven auth retry.
//!
//! Regression guard for the bug where these setters were silently dropped:
//! `with_from_display` / `with_subject` never reached the wire at all, and
//! `with_pai` / `with_contact_uri` reached the *initial* INVITE but vanished
//! on the 401/407 retry that actually completes the call (the retry rebuilt
//! headers from raw `extra_headers` only). The fix routes the initial INVITE
//! and the retry through one `materialize_invite_options` mapping +
//! `InviteRequestOptions`, so both wire attempts are identical.
//!
//! Harness mirrors `builder_auth_retry_preserves_headers.rs`: a raw-UDP mock
//! UAS answers INVITE #1 with `401 + WWW-Authenticate` and the credentialed
//! retry with `200 OK`, capturing both INVITEs.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout};

use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::types::Credentials;

use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderValue};

use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

const UAS_PORT: u16 = 35280;
const UAC_PORT: u16 = 35281;

const DISPLAY_NAME: &str = "Alice Smith";
const SUBJECT: &str = "Support call";
const CONTACT_OVERRIDE: &str = "sip:alice@127.0.0.1:5099;ob";
const PAI_URI: &str = "sip:+15551112222@trunk.example.net";

/// What we capture from each INVITE the UAS receives.
#[derive(Debug, Clone)]
struct CapturedInvite {
    from_raw: Option<String>,
    from_count: usize,
    subject: Option<String>,
    contact_raw: Option<String>,
    has_pai: bool,
    has_auth: bool,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn structured_invite_overrides_survive_401_driven_auth_retry() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    let uas_addr = format!("127.0.0.1:{UAS_PORT}");
    let sock = Arc::new(UdpSocket::bind(&uas_addr).await.expect("auth UAS bind"));

    let invite_count = Arc::new(AtomicU32::new(0));
    let invites_seen = Arc::new(Mutex::new(Vec::<CapturedInvite>::new()));

    let sock_task = sock.clone();
    let count_task = invite_count.clone();
    let captured_task = invites_seen.clone();
    let uas_handle = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            let (n, from) = match sock_task.recv_from(&mut buf).await {
                Ok(p) => p,
                Err(_) => return,
            };
            let msg = match parse_message(&buf[..n]) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let request = match msg {
                Message::Request(r) if r.method() == Method::Invite => r,
                _ => continue, // ACK to the 401 etc. — ignore
            };

            let count = count_task.fetch_add(1, Ordering::SeqCst);

            let captured = CapturedInvite {
                from_raw: request.raw_header_value(&HeaderName::From),
                from_count: request
                    .headers
                    .iter()
                    .filter(|h| matches!(h, TypedHeader::From(_)))
                    .count(),
                subject: request.raw_header_value(&HeaderName::Subject),
                contact_raw: request.raw_header_value(&HeaderName::Contact),
                has_pai: request.headers.iter().any(|h| {
                    matches!(h, TypedHeader::PAssertedIdentity(_))
                        || matches!(h.name(), HeaderName::Other(n) if n.eq_ignore_ascii_case("p-asserted-identity"))
                }),
                has_auth: request
                    .raw_header_value(&HeaderName::Authorization)
                    .is_some(),
            };
            captured_task.lock().await.push(captured);

            if count == 0 {
                // 401 with WWW-Authenticate to drive the credentialed retry.
                let mut resp = create_response(&request, StatusCode::Unauthorized);
                resp.headers.push(TypedHeader::Other(
                    HeaderName::WwwAuthenticate,
                    HeaderValue::Raw(
                        br#"Digest realm="testrealm", nonce="nonce-xyz", algorithm=MD5, qop="auth""#
                            .to_vec(),
                    ),
                ));
                let bytes = Message::Response(resp).to_bytes();
                let _ = sock_task.send_to(&bytes, from).await;
            } else {
                // 200 OK on the credentialed retry.
                let mut resp = create_response(&request, StatusCode::Ok);
                if let Some(contact) = request.header(&HeaderName::Contact) {
                    resp.headers.push(contact.clone());
                }
                let bytes = Message::Response(resp).to_bytes();
                let _ = sock_task.send_to(&bytes, from).await;
            }
        }
    });

    let coord = UnifiedCoordinator::new(Config::local("alice", UAC_PORT))
        .await
        .expect("UAC coordinator");
    sleep(Duration::from_millis(150)).await;

    let _call_id = coord
        .invite(
            Some("sip:alice@127.0.0.1".to_string()),
            format!("sip:bob@127.0.0.1:{UAS_PORT}"),
        )
        .with_credentials(Credentials::new("alice", "password").with_realm("testrealm"))
        .with_from_display(DISPLAY_NAME)
        .with_subject(SUBJECT)
        .with_contact_uri(CONTACT_OVERRIDE)
        .with_pai(PAI_URI)
        .send()
        .await
        .expect("invite.send()");

    // Wait for exactly two INVITEs (initial + retry).
    let observed = timeout(Duration::from_secs(8), async {
        loop {
            if invite_count.load(Ordering::SeqCst) >= 2 {
                return;
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(
        observed.is_ok(),
        "UAS never saw 2 INVITEs (count={})",
        invite_count.load(Ordering::SeqCst)
    );
    sleep(Duration::from_millis(300)).await;

    let captured = invites_seen.lock().await;
    assert_eq!(
        captured.len(),
        2,
        "expected initial INVITE + auth retry, got {}",
        captured.len()
    );

    // Both the initial INVITE and the authenticated retry must carry the
    // identical structured overrides — that is the whole point of the fix.
    for (label, inv) in [("initial", &captured[0]), ("retry", &captured[1])] {
        assert_eq!(
            inv.from_count, 1,
            "{label} INVITE must carry exactly one From header (guard against duplicate-From smuggling); got {}",
            inv.from_count
        );
        let from_raw = inv
            .from_raw
            .as_deref()
            .unwrap_or_else(|| panic!("{label} INVITE missing From header"));
        assert!(
            from_raw.contains(DISPLAY_NAME),
            "{label} INVITE From must carry display name {DISPLAY_NAME:?}; got {from_raw:?}"
        );
        assert_eq!(
            inv.subject.as_deref(),
            Some(SUBJECT),
            "{label} INVITE must carry Subject {SUBJECT:?}"
        );
        let contact_raw = inv
            .contact_raw
            .as_deref()
            .unwrap_or_else(|| panic!("{label} INVITE missing Contact header"));
        assert!(
            contact_raw.contains("127.0.0.1:5099"),
            "{label} INVITE Contact must reflect the with_contact_uri override; got {contact_raw:?}"
        );
        assert!(inv.has_pai, "{label} INVITE must carry P-Asserted-Identity");
    }

    // Auth specifics: initial has none, retry is credentialed.
    assert!(
        !captured[0].has_auth,
        "initial INVITE must NOT carry Authorization"
    );
    assert!(
        captured[1].has_auth,
        "auth retry INVITE must carry Authorization (credentialed)"
    );

    uas_handle.abort();
}

const PRECOMP_UAS_PORT: u16 = 35282;
const PRECOMP_UAC_PORT: u16 = 35283;
const PRECOMPUTED_AUTH: &str = r#"Digest username="alice", realm="testrealm", nonce="n", uri="sip:bob@127.0.0.1", response="deadbeef""#;

/// `with_precomputed_authorization` rides the **initial** INVITE so a UAS that
/// accepts pre-emptive auth never has to challenge (no 401 round-trip).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn precomputed_authorization_rides_initial_invite() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    let uas_addr = format!("127.0.0.1:{PRECOMP_UAS_PORT}");
    let sock = Arc::new(UdpSocket::bind(&uas_addr).await.expect("precomp UAS bind"));

    let invite_count = Arc::new(AtomicU32::new(0));
    let auth_seen = Arc::new(Mutex::new(Vec::<Option<String>>::new()));

    let sock_task = sock.clone();
    let count_task = invite_count.clone();
    let auth_task = auth_seen.clone();
    let uas_handle = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            let (n, from) = match sock_task.recv_from(&mut buf).await {
                Ok(p) => p,
                Err(_) => return,
            };
            let request = match parse_message(&buf[..n]) {
                Ok(Message::Request(r)) if r.method() == Method::Invite => r,
                _ => continue,
            };
            count_task.fetch_add(1, Ordering::SeqCst);
            auth_task
                .lock()
                .await
                .push(request.raw_header_value(&HeaderName::Authorization));

            // Accept pre-emptive auth immediately with 200 OK — no challenge.
            let mut resp = create_response(&request, StatusCode::Ok);
            if let Some(contact) = request.header(&HeaderName::Contact) {
                resp.headers.push(contact.clone());
            }
            let _ = sock_task
                .send_to(&Message::Response(resp).to_bytes(), from)
                .await;
        }
    });

    let coord = UnifiedCoordinator::new(Config::local("alice", PRECOMP_UAC_PORT))
        .await
        .expect("UAC coordinator");
    sleep(Duration::from_millis(150)).await;

    let _call_id = coord
        .invite(
            Some("sip:alice@127.0.0.1".to_string()),
            format!("sip:bob@127.0.0.1:{PRECOMP_UAS_PORT}"),
        )
        .with_precomputed_authorization(PRECOMPUTED_AUTH)
        .send()
        .await
        .expect("invite.send()");

    let observed = timeout(Duration::from_secs(8), async {
        loop {
            if invite_count.load(Ordering::SeqCst) >= 1 {
                return;
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(observed.is_ok(), "UAS never saw an INVITE");
    sleep(Duration::from_millis(200)).await;

    let auth = auth_seen.lock().await;
    assert!(!auth.is_empty(), "no INVITE captured");
    assert_eq!(
        auth[0].as_deref(),
        Some(PRECOMPUTED_AUTH),
        "initial INVITE must carry the pre-computed Authorization header verbatim"
    );
    // A single INVITE — pre-emptive auth means no challenge/retry.
    assert_eq!(
        invite_count.load(Ordering::SeqCst),
        1,
        "pre-computed auth should not trigger a 401 retry"
    );

    uas_handle.abort();
}
