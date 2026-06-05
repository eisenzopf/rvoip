//! Per-destination outbound INVITE routing on the `rvoip-sip` UAC surface.
//!
//! This is the "I want full say over the INVITE" example. Instead of the
//! one-shot `endpoint.call_and_wait(target)`, every call is built through the
//! [`OutboundCallBuilder`](rvoip_sip::api::send::OutboundCallBuilder) returned
//! by `coordinator().invite(from, to)`, so you vary — *per destination* — the
//! `From` AoR, auth scheme, outbound proxy `Route`, `P-Asserted-Identity`,
//! `Contact`, the SDP offer, RFC 3262 100rel, and arbitrary application
//! headers.
//!
//! Run it end-to-end against the embedded auto-answer UAS (no external
//! infrastructure needed) — it prints the INVITE the UAS actually received:
//!
//!   cargo run -p rvoip-sip --example outbound_routing
//!
//! The `route()` policy shows three realistic destination classes — internal
//! extension, PSTN trunk, federated TLS peer. The runnable path dials the
//! embedded UAS through the `internal` class; the trunk/TLS plans are built
//! the same way and are what you would send at a real Asterisk / carrier SBC.

use std::time::Duration;

use rvoip_sip::api::headers::convenience;
use rvoip_sip::api::headers::options::SipRequestOptions; // brings `.with_header(..)` into scope
use rvoip_sip::auth::SipClientAuth;
use rvoip_sip::{Config, Result, SessionError, StreamPeer};
use rvoip_sip_core::types::headers::{HeaderName, HeaderValue, TypedHeader};

/// Everything that varies by *where* you are calling. Build this from your own
/// routing table / dialplan; each field maps onto one builder call below.
struct CallPlan {
    /// `From:` AoR — your identity or trunk for this destination.
    from_uri: String,
    /// Request-URI / `To:`. A `sips:` target negotiates a TLS leg.
    target_uri: String,
    /// UAC auth used for a 401/407 retry. `None` for an IP-authenticated trunk.
    auth: Option<SipClientAuth>,
    /// Per-call outbound proxy `Route:` (e.g. a specific SBC for this carrier).
    outbound_proxy: Option<String>,
    /// `P-Asserted-Identity` (RFC 3325) for this call only.
    pai: Option<String>,
    /// `Contact:` URI advertised on this INVITE (`None` auto-derives it).
    contact_uri: Option<String>,
    /// `From:` display name.
    from_display: Option<String>,
    /// `Subject:` header.
    subject: Option<String>,
    /// Application headers (Privacy, Diversion, X-* trunk tags, …).
    extra_headers: Vec<TypedHeader>,
    /// Hand-rolled SDP offer. `None` lets the stack generate one.
    sdp: Option<String>,
}

/// Pick the call parameters for a destination. Replace the body with your
/// dialplan; the shape — classify the target, return a `CallPlan` — is the point.
fn route(target: &str) -> CallPlan {
    // Any header sip-core does not model natively rides through
    // `TypedHeader::Other` (policy-classified ApplicationControlled, so
    // `with_header` accepts it). `convenience::*` gives typed constructors for
    // the common RFC headers; this closure is the generic escape hatch.
    let trunk_tag = |id: &str| {
        TypedHeader::Other(
            HeaderName::Other("X-Trunk-Id".to_string()),
            HeaderValue::Raw(id.as_bytes().to_vec()),
        )
    };

    if let Some(number) = target.strip_prefix("pstn:") {
        // PSTN via a carrier trunk: digest auth, a dedicated SBC, an asserted
        // caller-ID, and a Privacy header — all scoped to this one call.
        CallPlan {
            from_uri: "sip:+15551234567@trunk.example.net".into(),
            target_uri: format!("sip:{number}@trunk.example.net"),
            auth: Some(SipClientAuth::digest("trunk-user", "trunk-secret")),
            outbound_proxy: Some("sip:sbc1.example.net:5060;lr".into()),
            pai: Some("sip:+15551234567@trunk.example.net".into()),
            contact_uri: None,
            from_display: Some("ACME Sales".into()),
            subject: Some("Outbound PSTN call".into()),
            extra_headers: vec![convenience::privacy("id"), trunk_tag("carrier-a")],
            sdp: None,
        }
    } else if let Some(user) = target.strip_prefix("secure:") {
        // Federated peer over TLS, authenticated with a bearer token. The
        // `sips:` request-URI is what drives the TLS leg.
        CallPlan {
            from_uri: "sip:alice@secure.example.org".into(),
            target_uri: format!("sips:{user}@secure.example.org"),
            auth: Some(SipClientAuth::bearer_token("eyJhbGciOiJSUzI1Ni...")),
            outbound_proxy: None,
            pai: None,
            contact_uri: None,
            from_display: Some("Alice".into()),
            subject: Some("Federated call".into()),
            extra_headers: vec![],
            sdp: None,
        }
    } else {
        // Internal extension on the local PBX. The `outbound_proxy` here points
        // at the UAS only so this example completes in-process — a real
        // internal call usually has no `Route`.
        CallPlan {
            from_uri: "sip:alice@127.0.0.1:5100".into(),
            target_uri: target.to_string(),
            auth: None,
            outbound_proxy: Some("sip:127.0.0.1:5101;lr".into()),
            pai: Some("sip:alice@127.0.0.1:5100".into()),
            contact_uri: Some("sip:alice@127.0.0.1:5100;ob".into()),
            from_display: Some("Alice (desk)".into()),
            subject: Some("Internal call".into()),
            extra_headers: vec![convenience::privacy("none"), trunk_tag("lan")],
            sdp: None,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // ── Embedded auto-answer UAS that prints the INVITE it receives ──────
    let uas = tokio::spawn(async {
        let mut bob = StreamPeer::with_config(Config::local("bob", 5101)).await?;
        let incoming = bob.wait_for_incoming().await?;
        if let Some(req) = incoming.raw_request() {
            println!("[uas] received INVITE {}", req.uri());
            for header in &req.headers {
                println!("[uas]   {header}");
            }
        }
        let call = incoming.accept().await?;
        call.wait_for_end(None).await?;
        bob.shutdown().await
    });
    tokio::time::sleep(Duration::from_millis(300)).await;

    // ── UAC ─────────────────────────────────────────────────────────────
    let mut alice = StreamPeer::with_config(Config::local("alice", 5100)).await?;
    // The coordinator is the lowest-level surface; `invite(from, to)` is where
    // the per-call `From` override lives (the StreamPeer shortcut uses config From).
    let coord = alice.coordinator().clone();

    // Classify the destination, then resolve its per-call plan.
    let target = "sip:bob@127.0.0.1:5101";
    let plan = route(target);
    println!("[uac] dialing {} as {}", plan.target_uri, plan.from_uri);

    // ── Build the INVITE with full per-destination control ──────────────
    let mut invite = coord.invite(Some(plan.from_uri.clone()), plan.target_uri.clone());
    if let Some(auth) = plan.auth {
        invite = invite.with_auth(auth); // negotiates digest/bearer/basic on 401/407
    }
    if let Some(proxy) = plan.outbound_proxy {
        invite = invite.with_outbound_proxy(proxy); // emits Route:
    }
    if let Some(pai) = plan.pai {
        invite = invite.with_pai(pai); // emits P-Asserted-Identity:
    }
    if let Some(contact) = plan.contact_uri {
        invite = invite.with_contact_uri(contact); // overrides the auto Contact:
    }
    if let Some(display) = plan.from_display {
        invite = invite.with_from_display(display); // sets the From display name
    }
    if let Some(subject) = plan.subject {
        invite = invite.with_subject(subject); // emits Subject:
    }
    if let Some(sdp) = plan.sdp {
        invite = invite.with_sdp(sdp); // your own offer; otherwise the stack builds one
    }
    invite = invite.with_supported_100rel(true); // emits Supported: 100rel

    // `with_header` is policy-guarded: it returns Err for stack-managed headers
    // (Via, CSeq, From/To, Call-ID, Contact, …) so you cannot corrupt the dialog.
    for header in plan.extra_headers {
        invite = invite
            .with_header(header)
            .map_err(|violation| SessionError::Other(format!("header policy: {violation:?}")))?;
    }

    // ── Fire it and drive the call lifecycle ────────────────────────────
    let call_id = invite.send().await?;
    let call = coord.session(&call_id); // public handle for this call leg
    alice.wait_for_answered(call.id()).await?;
    println!("[uac] answered: {}", call.id());

    // The returned handle drives mid-call control too (hold/resume/DTMF/transfer).
    tokio::time::sleep(Duration::from_millis(300)).await;
    call.hangup_and_wait(Some(Duration::from_secs(5))).await?;
    alice.shutdown().await?;

    uas.await
        .map_err(|err| SessionError::Other(err.to_string()))??;
    println!("[uac] done");
    Ok(())
}
