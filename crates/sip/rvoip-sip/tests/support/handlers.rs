//! Reusable `CallHandler` impls for callback-style tests.

#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use rvoip_sip::api::callback_peer::{CallHandler, CallHandlerDecision};
use rvoip_sip::api::headers::options::SipRequestOptions;
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::api::unified::UnifiedCoordinator;
use rvoip_sip::HeaderName;

/// Auto-accepts every inbound INVITE. The default §10 test handler.
pub struct AutoAccept;

#[async_trait::async_trait]
impl CallHandler for AutoAccept {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        let _ = call.accept().await;
        CallHandlerDecision::Accept
    }
}

/// B2BUA carry-through handler used by §10 #22.
///
/// On every inbound INVITE:
///
/// 1. Reads the `IncomingCall`'s typed `Arc<Request>` view via the
///    `SipHeaderView` trait it implements.
/// 2. Drives an outbound INVITE on the supplied `outbound_coord` using
///    `with_headers_from(&incoming, names)` to carry through the
///    application-controlled headers, then runs the §11.3 strip / rewrite
///    pattern (`strip_names` removed, `rewrites` re-stamped via
///    `with_raw_header`).
/// 3. Rejects the inbound with a 503 once the outbound is dispatched — the
///    test only cares about the outbound INVITE wire, not the inbound leg.
pub struct B2buaCarryThrough {
    pub outbound_coord: Arc<UnifiedCoordinator>,
    pub outbound_target: String,
    pub outbound_from: String,
    pub carry_names: Vec<HeaderName>,
    pub strip_names: Vec<HeaderName>,
    pub rewrites: Vec<(HeaderName, String)>,
}

#[async_trait::async_trait]
impl CallHandler for B2buaCarryThrough {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        // Drive the outbound leg using the IncomingCall as the
        // SipHeaderView source for `with_headers_from`. Errors are
        // surfaced via a panic so the test fails loudly.
        let builder = self.outbound_coord.invite(
            Some(self.outbound_from.clone()),
            self.outbound_target.clone(),
        );

        let (mut chain, _report) = builder
            .with_headers_from(&call, &self.carry_names)
            .expect("with_headers_from must succeed");

        for name in &self.strip_names {
            chain = chain.strip_header(name);
        }
        for (name, value) in &self.rewrites {
            chain = chain
                .with_raw_header(name.clone(), value.clone())
                .expect("with_raw_header rewrite");
        }

        // Fire and forget — the outbound call's wire will be captured on
        // bob's coord. We don't need to wait for it to complete.
        let _ = chain.send().await;

        // Settle so the outbound leg lands before we reject the inbound.
        tokio::time::sleep(Duration::from_millis(50)).await;

        CallHandlerDecision::Reject {
            status: 503,
            reason: "B2BUA test done".into(),
        }
    }
}
