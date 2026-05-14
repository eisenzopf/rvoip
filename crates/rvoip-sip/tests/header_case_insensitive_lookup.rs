//! SIP_API_DESIGN_2 §10 verification #22 — case-insensitive header
//! name canonicalization.
//!
//! `with_raw_header("x-customer-id", ...)` must stage a header whose
//! name canonicalizes to `X-Customer-Id`, so subsequent lookups with
//! any case (lower, upper, mixed) resolve to the same staged entry.

use std::time::Duration;

use rvoip_sip::api::headers::options::SipRequestOptions;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip_core::types::header::HeaderName;

async fn boot() -> std::sync::Arc<UnifiedCoordinator> {
    let coord = UnifiedCoordinator::new(Config::local("case-test", 17070))
        .await
        .expect("coordinator");
    tokio::time::sleep(Duration::from_millis(50)).await;
    coord
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn raw_header_name_is_canonicalized_for_lookup() {
    let coord = boot().await;

    // Stage with lowercase; assert canonical form (Title-Case-Per-Token).
    let builder = coord
        .invite(None, "sip:bob@127.0.0.1:1")
        .with_raw_header(
            HeaderName::Other("x-customer-id".to_string()),
            "cust-7",
        )
        .expect("with_raw_header must accept application-controlled name");

    let staged = builder.staged_headers();
    let names: Vec<HeaderName> = staged.iter().map(|h| h.name()).collect();

    let canonical = HeaderName::Other("X-Customer-Id".to_string());
    assert!(
        names.contains(&canonical),
        "expected canonical `X-Customer-Id` in staged headers; got {names:?}"
    );

    // Equivalent uppercase input must produce the same canonical entry,
    // not a duplicate with different casing.
    let builder = builder
        .with_raw_header(
            HeaderName::Other("X-CUSTOMER-ID".to_string()),
            "cust-8",
        )
        .expect("identical name uppercased must still stage");

    let staged = builder.staged_headers();
    let canonical_count = staged
        .iter()
        .filter(|h| h.name() == canonical)
        .count();
    assert_eq!(
        canonical_count, 2,
        "both stagings must land under the same canonical name; got {} matches",
        canonical_count
    );

    // No alt-case form should appear under a different key.
    for alt in [
        HeaderName::Other("x-customer-id".to_string()),
        HeaderName::Other("X-CUSTOMER-ID".to_string()),
        HeaderName::Other("X-customer-ID".to_string()),
    ] {
        assert!(
            !staged.iter().any(|h| h.name() == alt),
            "non-canonical key `{alt:?}` must not appear in staged headers"
        );
    }
}
