//! SIP_API_DESIGN_2 §10 verification #12 — `BuilderStrictness` policy
//! mode integration.
//!
//! - `Strict` mode rejects `with_header(Authorization)` on
//!   `RegisterBuilder` with `Err(UseDedicatedSetter)`.
//! - `Lenient` mode silently drops the same header (with a `warn!` log)
//!   and the call returns `Ok(self)`.
//! - Both modes reject `with_header(CallId)` as `Err(StackManaged)` —
//!   stack-managed names are never down-graded.

use std::time::Duration;

use rvoip_sip::api::headers::options::{
    BuilderStrictness, SipRequestOptions, ViolationReason,
};
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip_core::types::call_id::CallId as CallIdHdr;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::{HeaderValue, TypedHeader};

async fn boot(name: &str, port: u16) -> std::sync::Arc<UnifiedCoordinator> {
    let coord = UnifiedCoordinator::new(Config::local(name, port))
        .await
        .expect("coordinator");
    tokio::time::sleep(Duration::from_millis(50)).await;
    coord
}

fn make_authorization() -> TypedHeader {
    TypedHeader::Other(
        HeaderName::Authorization,
        HeaderValue::Raw(b"Digest username=\"alice\"".to_vec()),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn strict_mode_rejects_method_shaped_authorization_on_register() {
    let coord = boot("strict-method", 17060).await;

    let result = coord
        .register("sip:registrar.example.com", "alice", "secret")
        .with_strictness(BuilderStrictness::Strict)
        .with_header(make_authorization());

    let err = match result {
        Ok(_) => panic!("Strict mode must reject Authorization on RegisterBuilder"),
        Err(e) => e,
    };
    assert!(
        matches!(err.reason, ViolationReason::UseDedicatedSetter(_)),
        "expected UseDedicatedSetter under Strict; got {:?}",
        err.reason
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lenient_mode_drops_method_shaped_authorization_on_register() {
    let coord = boot("lenient-method", 17061).await;

    let builder = coord
        .register("sip:registrar.example.com", "alice", "secret")
        .with_strictness(BuilderStrictness::Lenient)
        .with_header(make_authorization())
        .expect("Lenient mode must downgrade method-shaped header to a drop");

    // The dropped header must not appear in the staged set.
    let staged = builder.staged_headers();
    assert!(
        staged
            .iter()
            .all(|h| h.name() != HeaderName::Authorization),
        "Lenient drop must not stage the header; staged = {:?}",
        staged.iter().map(|h| h.name()).collect::<Vec<_>>()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn both_modes_reject_stack_managed_call_id() {
    let coord = boot("stack-both", 17062).await;

    for mode in [BuilderStrictness::Strict, BuilderStrictness::Lenient] {
        let call_id = TypedHeader::CallId(CallIdHdr::new("test-call-id"));
        let result = coord
            .invite(None, "sip:bob@127.0.0.1:1")
            .with_strictness(mode)
            .with_header(call_id);
        let err = match result {
            Ok(_) => panic!("mode {mode:?} must reject Call-ID"),
            Err(e) => e,
        };
        assert_eq!(
            err.reason,
            ViolationReason::StackManaged,
            "mode {mode:?} must reject Call-ID as StackManaged"
        );
    }
}
