//! SIP_API_DESIGN_2 §10 verification #8 — `with_header` policy guards.
//!
//! Asserts the builder layer rejects stack-managed names with
//! `Err(StackManaged)`, redirects method-shaped names to their dedicated
//! setters via `Err(UseDedicatedSetter)`, and that `with_headers_from`
//! audits skipped names in `HeaderCarryThroughReport.skipped`.
//!
//! Pure builder test — no wire I/O.

use std::time::Duration;

use rvoip_sip::api::headers::options::{ViolationReason, SipRequestOptions};
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::TypedHeader;
use rvoip_sip_core::types::{CSeq, Method, Request, Uri};

async fn boot(name: &str, port: u16) -> std::sync::Arc<UnifiedCoordinator> {
    let coord = UnifiedCoordinator::new(Config::local(name, port))
        .await
        .expect("coordinator");
    tokio::time::sleep(Duration::from_millis(50)).await;
    coord
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_header_rejects_stack_managed_cseq_on_invite() {
    let coord = boot("guard-invite", 17050).await;

    let cseq = TypedHeader::CSeq(CSeq::new(1, Method::Invite));
    let result = coord
        .invite(None, "sip:bob@127.0.0.1:1")
        .with_header(cseq);
    let err = match result {
        Ok(_) => panic!("CSeq must be rejected on INVITE"),
        Err(e) => e,
    };

    assert_eq!(
        err.reason,
        ViolationReason::StackManaged,
        "CSeq must be StackManaged on INVITE; got {:?}",
        err.reason
    );
    assert_eq!(err.method, Method::Invite);
    assert_eq!(err.header, HeaderName::CSeq);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_header_rejects_authorization_with_dedicated_setter_hint_on_register() {
    let coord = boot("guard-register", 17051).await;

    // Authorization on REGISTER is MethodShaped — the policy must
    // surface the dedicated setter name in the error so callers can
    // discover it without reading the policy source.
    let auth = TypedHeader::Other(
        HeaderName::Authorization,
        rvoip_sip_core::types::headers::HeaderValue::Raw(
            b"Digest username=\"alice\", realm=\"test\"".to_vec(),
        ),
    );
    let result = coord
        .register("sip:registrar.example.com", "alice", "secret")
        .with_header(auth);
    let err = match result {
        Ok(_) => panic!("Authorization must be rejected on REGISTER under Strict"),
        Err(e) => e,
    };

    match err.reason {
        ViolationReason::UseDedicatedSetter(setter) => {
            assert!(
                setter.contains("credentials"),
                "expected setter hint to mention 'credentials'; got `{setter}`"
            );
        }
        other => panic!(
            "expected UseDedicatedSetter for Authorization on REGISTER; got {other:?}"
        ),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_headers_from_skips_stack_managed_names_in_audit_report() {
    let coord = boot("guard-carry", 17052).await;

    // Wrap a minimal Request as the SipHeaderView source. We populate
    // it with both stack-managed names (Via, CSeq, Call-ID, Max-Forwards)
    // and one application-controlled name (Subject) to confirm the
    // audit reports skipped names correctly and copies the rest.
    let req = build_request_with_topology_headers();

    let view = RequestView(&req);
    let names = vec![
        HeaderName::Via,
        HeaderName::CSeq,
        HeaderName::CallId,
        HeaderName::MaxForwards,
        HeaderName::Subject,
    ];

    let (_builder, report) = coord
        .invite(None, "sip:bob@127.0.0.1:1")
        .with_headers_from(&view, &names)
        .expect("carry-through must succeed");

    let skipped_names: Vec<HeaderName> =
        report.skipped.iter().map(|(n, _)| n.clone()).collect();
    for must_skip in [
        HeaderName::Via,
        HeaderName::CSeq,
        HeaderName::CallId,
        HeaderName::MaxForwards,
    ] {
        assert!(
            skipped_names.contains(&must_skip),
            "{must_skip:?} must be skipped; report.skipped = {:?}",
            report.skipped
        );
    }
    assert!(
        report.copied.contains(&HeaderName::Subject),
        "Subject must be copied; report.copied = {:?}",
        report.copied
    );
}

/// Minimal `SipHeaderView` wrapper around a `Request`. Implements only
/// what `with_headers_from` needs.
struct RequestView<'a>(&'a Request);

impl<'a> rvoip_sip::api::headers::view::SipHeaderView for RequestView<'a> {
    fn header(&self, name: &HeaderName) -> Option<&TypedHeader> {
        self.0.header(name)
    }
    fn headers_named<'b>(
        &'b self,
        name: &HeaderName,
    ) -> Box<dyn Iterator<Item = &'b TypedHeader> + 'b> {
        let n = name.clone();
        Box::new(self.0.headers.iter().filter(move |h| h.name() == n))
    }
    fn headers<'b>(&'b self) -> Box<dyn Iterator<Item = &'b TypedHeader> + 'b> {
        Box::new(self.0.headers.iter())
    }
    fn header_names(&self) -> Vec<HeaderName> {
        let mut seen = Vec::new();
        for h in &self.0.headers {
            let n = h.name();
            if !seen.contains(&n) {
                seen.push(n);
            }
        }
        seen
    }
}

fn build_request_with_topology_headers() -> Request {
    use rvoip_sip_core::types::call_id::CallId as CallIdHdr;
    use rvoip_sip_core::types::max_forwards::MaxForwards;

    let uri: Uri = "sip:bob@127.0.0.1".parse().expect("uri");
    let mut req = Request::new(Method::Invite, uri);
    req.headers
        .push(TypedHeader::CallId(CallIdHdr::new("topology-call-id")));
    req.headers
        .push(TypedHeader::CSeq(CSeq::new(42, Method::Invite)));
    req.headers.push(TypedHeader::MaxForwards(MaxForwards::new(70)));
    req.headers.push(TypedHeader::Other(
        HeaderName::Subject,
        rvoip_sip_core::types::headers::HeaderValue::Raw(b"hello".to_vec()),
    ));
    req
}
