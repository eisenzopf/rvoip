//! Request Lifecycle Management
//!
//! Symmetric counterpart to [`ResponseLifecycle`](crate::manager::ResponseLifecycle).
//! Defines hooks called at critical points while building and emitting
//! outbound SIP requests, so signing / header-stamping / instrumentation
//! can attach without each emission site reinventing the seam.
//!
//! ## STIR/SHAKEN role (RFC 8224)
//!
//! `pre_send_request` is the hook point at which the installed
//! [`PASSporTSigner`](crate::manager::PASSporTSigner) attaches an
//! `Identity:` header to dialog-creating requests. It runs **after**
//! Via / Max-Forwards / Route stamping by the transaction layer but
//! **before** the message hits the wire — the PASSporT claims are
//! computed from the final `From` / `To` / `Date` headers, so the
//! signature covers the canonical form the peer's verifier will see
//! (RFC 8224 §5.1).
//!
//! ## Architecture
//!
//! ```text
//! UAC outbound (INVITE example):
//!   build Request (dialog quick fns)
//!   ↓
//!   transaction layer stamps Via / Max-Forwards / Route
//!   ↓
//!   RequestLifecycle::pre_send_request(&mut req, dest):
//!       — extract claim seed from req.From / req.To / Date / etc.
//!       — signer.sign(claims) → IdentityHeaderValue
//!       — req.headers.push(TypedHeader::Identity(...))
//!   ↓
//!   serialize → Transport::send_message
//!   ↓
//!   post_send_request hook (no-op default; metrics / tracing later)
//! ```

use rvoip_sip_core::Request;
use std::net::SocketAddr;

use crate::diagnostics::safe_log::method_class;
use crate::errors::DialogResult;

#[cfg(test)]
pub(crate) mod test_hooks {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, LazyLock, Mutex};

    pub(crate) struct PostSendGate {
        pub(crate) entered: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }

    static POST_SEND_GATES: LazyLock<Mutex<HashMap<String, Arc<PostSendGate>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    pub(crate) fn install_post_send_gate(call_id: &str) -> Arc<PostSendGate> {
        let gate = Arc::new(PostSendGate {
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        });
        POST_SEND_GATES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(call_id.to_string(), gate.clone());
        gate
    }

    pub(crate) fn remove_post_send_gate(call_id: &str) {
        POST_SEND_GATES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(call_id);
    }

    pub(crate) async fn wait_if_installed(request: &Request) {
        let gate = request.call_id().and_then(|call_id| {
            POST_SEND_GATES
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(&call_id.value())
                .cloned()
        });
        if let Some(gate) = gate {
            gate.entered.notify_one();
            gate.release.notified().await;
        }
    }
}

/// Request lifecycle hooks for dialog-creating outbound requests.
///
/// Implementors receive the fully-built request (with all stack-
/// managed headers already stamped) and may mutate it before the
/// transaction layer serializes and ships it. The current set of
/// hooks targets STIR/SHAKEN signing; the same shape extends to
/// future per-request instrumentation needs.
pub trait RequestLifecycle {
    /// Called BEFORE the outbound request is serialised and shipped.
    ///
    /// Mutations to `request` go on the wire. Returning `Err(_)`
    /// aborts the outbound emission and surfaces the error to the
    /// caller — use sparingly; the typical case is to mutate and
    /// return `Ok(())`.
    ///
    /// # Arguments
    /// * `request` — the fully-stamped outbound request (mutable).
    /// * `destination` — the resolved next-hop address; available so
    ///   the hook can make per-destination decisions (e.g. skip
    ///   signing on internal trunks).
    fn pre_send_request(
        &self,
        request: &mut Request,
        destination: SocketAddr,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;

    /// Called AFTER the outbound request has been handed to the
    /// transport. Defaults to a no-op; provided for symmetry with
    /// [`ResponseLifecycle::post_send_response`](crate::manager::ResponseLifecycle::post_send_response).
    fn post_send_request(
        &self,
        _request: &Request,
        _destination: SocketAddr,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send {
        async { Ok(()) }
    }
}

/// Default implementation for [`DialogManager`](crate::manager::DialogManager):
///
/// - `pre_send_request` consults the installed
///   [`PASSporTSigner`](crate::manager::PASSporTSigner). If present, it
///   builds a [`PassportClaimSummary`](crate::manager::PassportClaimSummary)
///   from the request's typed headers, calls `signer.sign(...)`, and
///   appends the returned `Identity:` header to the request.
/// - When no signer is installed, the hook is a no-op so existing
///   callers see zero behaviour change.
impl RequestLifecycle for crate::manager::DialogManager {
    async fn pre_send_request(
        &self,
        request: &mut Request,
        _destination: SocketAddr,
    ) -> DialogResult<()> {
        // Gate: signer must be installed.
        let signer = match self.identity_signer() {
            Some(s) => s,
            None => return Ok(()),
        };

        // Build a coarse PassportClaimSummary from the request. The
        // signer's job is to assemble the full RFC 8225 PASSporT —
        // we hand it the SIP-shaped raw facts and let it decide
        // which (TN vs URI) shape to emit.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let (orig_uri, orig_tn) = extract_uri_and_tn(request, Direction::From);
        let (dest_uri, dest_tn) = extract_uri_and_tn(request, Direction::To);

        let claims = crate::manager::PassportClaimSummary {
            orig_tn,
            orig_uri,
            dest_tn,
            dest_uri,
            iat: now,
            origid: Some(uuid::Uuid::new_v4()),
            // The signer's deployment configuration picks the
            // attestation level; we pass `None` so the signer's
            // policy (typically "A" for full attestation in SHAKEN
            // deployments) wins.
            attest: None,
            // Same for ppt — signer chooses the profile.
            ppt: None,
        };

        match signer.sign(claims).await {
            Ok(value) => {
                let identity = rvoip_sip_core::types::identity::Identity::with_params(
                    value.jwt,
                    Some(value.info),
                    Some(value.alg),
                    value.ppt,
                );
                request
                    .headers
                    .push(rvoip_sip_core::types::TypedHeader::Identity(identity));
                Ok(())
            }
            Err(kind) => {
                tracing::warn!(
                    "PASSporTSigner failed ({:?}) — outbound {} request emitted unsigned",
                    kind,
                    method_class(&request.method)
                );
                // Degrade open: send unsigned rather than fail the
                // request. SHAKEN-strict deployments override by
                // wrapping the trait impl with a fail-closed adapter.
                Ok(())
            }
        }
    }

    async fn post_send_request(
        &self,
        _request: &Request,
        _destination: SocketAddr,
    ) -> DialogResult<()> {
        #[cfg(test)]
        test_hooks::wait_if_installed(_request).await;
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Direction {
    From,
    To,
}

/// Extract `(uri, tn)` from the From or To header of a Request.
/// `tn` is populated when the URI scheme is `tel:` or the user-part
/// looks like an E.164 number (starts with `+` followed by digits).
fn extract_uri_and_tn(request: &Request, dir: Direction) -> (Option<String>, Option<String>) {
    use rvoip_sip_core::types::TypedHeader;

    let addr_opt = request.headers.iter().find_map(|h| match (h, dir) {
        (TypedHeader::From(from), Direction::From) => Some(&from.0),
        (TypedHeader::To(to), Direction::To) => Some(&to.0),
        _ => None,
    });

    let addr = match addr_opt {
        Some(a) => a,
        None => return (None, None),
    };

    let uri_str = addr.uri.to_string();
    let tn = tn_from_uri(&addr.uri);

    (Some(uri_str), tn)
}

/// Extract an E.164 TN from a URI when the scheme is `tel:` or the
/// SIP user-part is itself an E.164 number.
///
/// For `tel:+15551234567`, the URI parser stores the number on the
/// `host` field as a `Domain`, not on `user`. For `sip:+15551234567@gw`,
/// the number is on `user`. Handle both shapes.
fn tn_from_uri(uri: &rvoip_sip_core::types::uri::Uri) -> Option<String> {
    use rvoip_sip_core::types::uri::{Host, Scheme};
    match uri.scheme {
        Scheme::Tel => match &uri.host {
            Host::Domain(d) => Some(d.clone()),
            _ => None,
        },
        Scheme::Sip | Scheme::Sips => {
            let user = uri.user.as_deref()?;
            if user.starts_with('+') && user[1..].chars().all(|c| c.is_ascii_digit()) {
                Some(user.to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tn_from_tel_uri() {
        let uri: rvoip_sip_core::types::uri::Uri = "tel:+15551234567".parse().unwrap();
        assert_eq!(tn_from_uri(&uri).as_deref(), Some("+15551234567"));
    }

    #[test]
    fn tn_from_sip_uri_with_e164_user() {
        let uri: rvoip_sip_core::types::uri::Uri =
            "sip:+15551234567@gw.example.com".parse().unwrap();
        assert_eq!(tn_from_uri(&uri).as_deref(), Some("+15551234567"));
    }

    #[test]
    fn tn_none_for_named_sip_user() {
        let uri: rvoip_sip_core::types::uri::Uri = "sip:alice@example.com".parse().unwrap();
        assert!(tn_from_uri(&uri).is_none());
    }
}
