//! `HeaderPolicy` — layer-boundary enforcement.
//!
//! Classifies every header into one of three roles:
//!
//! - **StackManaged** — owned by the dialog or transaction layer (Call-ID,
//!   CSeq, Via, Max-Forwards, Content-Length, Route, Record-Route).
//!   Applications cannot stage these; doing so would desync the dialog.
//! - **MethodShaped** — exists in sip-core, but the builder offers a
//!   dedicated typed setter (Authorization → `with_credentials` / `with_auth`,
//!   Contact → `with_contact_uri`, Expires → `with_expires`, …).
//! - **ApplicationControlled** — applications may stage freely
//!   (`Diversion`, `History-Info`, `Subject`, `X-*`, etc.).
//!
//! The matrix is **method-aware** — `Contact` is shaped on initial
//! INVITE/REGISTER but stack-managed in-dialog; `Refer-To` is only
//! shaped on REFER; `Event` only on SUBSCRIBE/NOTIFY.

#![allow(missing_docs)] // policy module is internal infrastructure

use rvoip_sip_core::types::headers::{HeaderName, TypedHeader};
use rvoip_sip_core::types::Method;

use super::ViolationReason;

/// Role assigned to a header for a given SIP method.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeaderRole {
    /// Owned by the dialog/transaction layer. Hard `Err` even under
    /// `BuilderStrictness::Lenient` because staging would desync the
    /// dialog.
    StackManaged,
    /// Has a dedicated typed setter on this builder. The name of that
    /// setter is reported back to the application.
    MethodShaped { setter: &'static str },
    /// Applications may stage freely.
    ApplicationControlled,
}

/// Validate-outbound result: the requested header is required for this
/// method but missing from the staged set.
#[derive(Debug, Clone)]
pub struct MissingRequiredHeader {
    pub method: Method,
    pub name: HeaderName,
    pub reason: &'static str,
}

/// Classify a header for a given method. See module-level docs.
pub fn classify(method: Method, name: &HeaderName) -> HeaderRole {
    if is_always_stack_managed(name) {
        return HeaderRole::StackManaged;
    }
    if matches!(name, HeaderName::Route) {
        // Route is stack-managed by the dialog route-set logic on every
        // outbound request. Applications use `with_outbound_proxy` /
        // `without_outbound_proxy` instead.
        return HeaderRole::StackManaged;
    }

    // Method-shaped overrides.
    if let Some(setter) = method_shaped_setter(method, name) {
        return HeaderRole::MethodShaped { setter };
    }

    HeaderRole::ApplicationControlled
}

/// Whether `name` should be silently filtered when copying from an
/// inbound message via `with_headers_from`. The stack-managed slice is
/// always filtered; method-shaped names go through the normal policy
/// check on the destination builder.
pub fn forbidden_for_carry_through(name: &HeaderName) -> bool {
    is_always_stack_managed(name) || matches!(name, HeaderName::Route)
}

/// SIP_API_DESIGN_2 §5.4 — the load-bearing safety check at the
/// dialog-adapter boundary. Catches stack-managed names that slipped
/// past the builder's `with_header` strictness gate (e.g. via direct
/// manipulation of `BuilderHeaderState::headers`). Each forbidden hit
/// is reported as a `MissingRequiredHeader` whose `reason` documents
/// the policy violation; callers convert that into the typed
/// `SessionError::HeaderPolicy` / `SessionError::MissingRequiredHeader`.
///
/// Today the positive (required-on-method) side is enforced at the
/// per-builder setter level — `InfoBuilder` requires a non-empty
/// content-type via the typed field, `SubscribeBuilder` requires an
/// event package, etc. — so this hook only runs the negative pass over
/// the extra-headers slice.
pub fn validate_outbound(
    method: Method,
    headers: &[TypedHeader],
) -> Result<(), Vec<MissingRequiredHeader>> {
    let mut violations = Vec::new();
    for h in headers {
        let name = h.name();
        if matches!(classify(method.clone(), &name), HeaderRole::StackManaged) {
            violations.push(MissingRequiredHeader {
                method: method.clone(),
                name,
                reason: "stack-managed header staged in application extras — \
                         this name is owned by the dialog/transaction layer",
            });
        }
    }
    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

fn is_always_stack_managed(name: &HeaderName) -> bool {
    matches!(
        name,
        HeaderName::CallId
            | HeaderName::CSeq
            | HeaderName::Via
            | HeaderName::MaxForwards
            | HeaderName::ContentLength
            | HeaderName::RecordRoute
    )
}

fn method_shaped_setter(method: Method, name: &HeaderName) -> Option<&'static str> {
    use HeaderName as H;
    use Method as M;

    match (method, name) {
        // Contact: shaped on initial INVITE / REGISTER; SUBSCRIBE init.
        // (In-dialog re-INVITE / BYE / NOTIFY get Contact stack-managed,
        // but the builder-side classification only sees fresh requests.)
        (M::Invite | M::Register | M::Subscribe, H::Contact) => Some("with_contact_uri"),

        // Authorization: shaped on every UAC request that accepts
        // `with_credentials` or `with_auth`. The Bye/Cancel path doesn't expose creds
        // (stack-managed on in-dialog) but is harmless to flag here.
        (
            M::Invite | M::Register | M::Subscribe | M::Message | M::Options | M::Refer,
            H::Authorization,
        ) => Some("with_credentials"),

        // Expires: shaped on REGISTER and SUBSCRIBE only.
        (M::Register | M::Subscribe, H::Expires) => Some("with_expires"),

        // Refer-To: shaped on REFER only.
        (M::Refer, H::ReferTo) => Some("refer(.., refer_to)"),

        // Event / Subscription-State: shaped on NOTIFY and SUBSCRIBE only.
        (M::Notify | M::Subscribe, H::Event) => Some("notify(.., event_package)"),
        (M::Notify, H::SubscriptionState) => Some("with_subscription_state"),

        _ => None,
    }
}

/// Map a method+header pair to a `ViolationReason`. The caller has
/// already determined the role; this is the one-stop converter for
/// builder setters that need the user-facing reason.
pub fn role_to_violation(role: &HeaderRole) -> Option<ViolationReason> {
    match role {
        HeaderRole::StackManaged => Some(ViolationReason::StackManaged),
        HeaderRole::MethodShaped { setter } => Some(ViolationReason::UseDedicatedSetter(setter)),
        HeaderRole::ApplicationControlled => None,
    }
}
