//! SIP_API_DESIGN_2 §10 verification #26 — topology hiding guarantee.
//!
//! `with_headers_from(&inbound, names)` must filter every stack-managed
//! topology name (Via, Record-Route, Call-ID, CSeq, Max-Forwards,
//! Content-Length) into `HeaderCarryThroughReport.skipped`. The filter
//! is the load-bearing guarantee that a naïve B2BUA cannot leak
//! upstream topology to a downstream peer through carry-through.
//!
//! Pure builder/policy test — no wire I/O.

use std::time::Duration;

use rvoip_sip::api::headers::options::{SipRequestOptions, ViolationReason};
use rvoip_sip::api::headers::view::SipHeaderView;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip_core::types::call_id::CallId as CallIdHdr;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::{HeaderValue, TypedHeader};
use rvoip_sip_core::types::max_forwards::MaxForwards;
use rvoip_sip_core::types::{CSeq, Method, Request, Uri};

async fn boot() -> std::sync::Arc<UnifiedCoordinator> {
    let coord = UnifiedCoordinator::new(Config::local("topology", 17080))
        .await
        .expect("coordinator");
    tokio::time::sleep(Duration::from_millis(50)).await;
    coord
}

/// Minimal `SipHeaderView` over a `Request` so the test can simulate
/// an inbound message with topology headers populated.
struct RequestView<'a>(&'a Request);

impl<'a> SipHeaderView for RequestView<'a> {
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

fn build_inbound_with_topology() -> Request {
    let uri: Uri = "sip:bob@upstream.example".parse().expect("uri");
    let mut req = Request::new(Method::Invite, uri);
    req.headers.push(TypedHeader::CallId(CallIdHdr::new(
        "upstream-call-id@upstream.example",
    )));
    req.headers
        .push(TypedHeader::CSeq(CSeq::new(101, Method::Invite)));
    req.headers
        .push(TypedHeader::MaxForwards(MaxForwards::new(70)));
    // Via and Record-Route as Other (their parser construction is more
    // involved; the policy classifies them by name, not value).
    req.headers.push(TypedHeader::Other(
        HeaderName::Via,
        HeaderValue::Raw(b"SIP/2.0/UDP upstream.example:5060;branch=z9hG4bK-upstream".to_vec()),
    ));
    req.headers.push(TypedHeader::Other(
        HeaderName::RecordRoute,
        HeaderValue::Raw(b"<sip:upstream-proxy.example;lr>".to_vec()),
    ));
    req.headers.push(TypedHeader::Other(
        HeaderName::Other("Subject".to_string()),
        HeaderValue::Raw(b"legitimate-app-header".to_vec()),
    ));
    req
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn topology_headers_are_filtered_into_skipped_report() {
    let coord = boot().await;

    let inbound = build_inbound_with_topology();
    let view = RequestView(&inbound);

    // Request every topology name explicitly to confirm none of them
    // are copied through — they must all surface in `skipped`.
    let names = vec![
        HeaderName::Via,
        HeaderName::RecordRoute,
        HeaderName::CallId,
        HeaderName::CSeq,
        HeaderName::MaxForwards,
        HeaderName::ContentLength,
        // A legitimate application header that must come through.
        HeaderName::Other("Subject".to_string()),
    ];

    let (builder, report) = coord
        .invite(None, "sip:downstream@127.0.0.1:1")
        .with_headers_from(&view, &names)
        .expect("with_headers_from must succeed");

    // Every topology name must appear in the skipped audit, regardless
    // of whether the source request actually carried it.
    for must_skip in [
        HeaderName::Via,
        HeaderName::RecordRoute,
        HeaderName::CallId,
        HeaderName::CSeq,
        HeaderName::MaxForwards,
        HeaderName::ContentLength,
    ] {
        let found = report.skipped.iter().any(|(n, _r)| n == &must_skip);
        assert!(
            found,
            "{must_skip:?} must appear in report.skipped; report = {:?}",
            report.skipped
        );
    }

    // Every skipped topology name must be StackManaged — never a
    // method-shape mismatch or anything subtler.
    for (name, reason) in &report.skipped {
        if matches!(
            name,
            HeaderName::Via
                | HeaderName::RecordRoute
                | HeaderName::CallId
                | HeaderName::CSeq
                | HeaderName::MaxForwards
                | HeaderName::ContentLength
        ) {
            assert_eq!(
                reason,
                &ViolationReason::StackManaged,
                "{name:?} must be skipped as StackManaged; got {reason:?}"
            );
        }
    }

    // The non-topology header must come through.
    assert!(
        report
            .copied
            .contains(&HeaderName::Other("Subject".to_string())),
        "Subject must be copied through; report.copied = {:?}",
        report.copied
    );

    // Staged headers must contain only the legitimate carry-through —
    // no topology bleed.
    let staged_names: Vec<HeaderName> = builder.staged_headers().iter().map(|h| h.name()).collect();
    for must_not_stage in [
        HeaderName::Via,
        HeaderName::RecordRoute,
        HeaderName::CallId,
        HeaderName::CSeq,
        HeaderName::MaxForwards,
    ] {
        assert!(
            !staged_names.contains(&must_not_stage),
            "{must_not_stage:?} must NOT be staged on outbound; staged = {staged_names:?}"
        );
    }
}
