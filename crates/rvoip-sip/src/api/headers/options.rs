//! `SipRequestOptions` — the shared outbound-builder shape.
//!
//! Every outbound and response builder in `rvoip-sip` implements this
//! trait so that "inspect, add, modify, delete SIP fields" is the same
//! shape across every method and every direction.
//!
//! Default implementations of `with_headers`, `with_raw_header`,
//! `strip_header`, `with_headers_from`, `staged_headers`, and
//! `with_strictness` live on the trait and operate on a shared
//! `BuilderHeaderState`. Concrete builders only implement `method()`,
//! `with_header()`, plus a small `header_state_mut()` accessor.

use std::fmt;

use rvoip_sip_core::types::headers::{HeaderName, HeaderValue, TypedHeader};
use rvoip_sip_core::types::Method;

use super::policy::{self, HeaderRole};
use super::view::SipHeaderView;

/// Shared per-builder state. Embedded by every concrete builder and
/// surfaced to the trait defaults through
/// [`SipRequestOptions::header_state_mut`].
#[derive(Default, Debug, Clone)]
pub struct BuilderHeaderState {
    /// Application-staged headers in wire order. Stack-managed headers
    /// (Call-ID, Via, CSeq, …) are never visible here; they are stamped
    /// by the dialog/transaction layer after this list is forwarded.
    pub headers: Vec<TypedHeader>,
    /// Validation strictness. Defaults to
    /// [`BuilderStrictness::Strict`].
    pub strictness: BuilderStrictness,
}

/// Validation strictness for builder header policy.
#[non_exhaustive]
#[derive(Copy, Clone, Debug, Eq, PartialEq, Default)]
pub enum BuilderStrictness {
    /// Default. Any `HeaderPolicyViolation` is a hard `Err`.
    /// `StackManaged` is always a hard `Err` regardless of mode.
    #[default]
    Strict,
    /// `MethodShaped` violations downgrade to `tracing::warn!` and the
    /// offending header is silently dropped. `StackManaged` remains a
    /// hard `Err`.
    Lenient,
}

/// Reason a header could not be staged via `with_header`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViolationReason {
    /// Owned by the dialog or transaction layer.
    StackManaged,
    /// Wrong method for this header (e.g. Event on INVITE).
    WrongMethod,
    /// Header has a dedicated builder setter that must be used instead.
    UseDedicatedSetter(&'static str),
}

impl fmt::Display for ViolationReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ViolationReason::StackManaged => f.write_str("owned by the dialog/transaction layer"),
            ViolationReason::WrongMethod => f.write_str("not allowed on this SIP method"),
            ViolationReason::UseDedicatedSetter(s) => {
                write!(f, "use the dedicated `{s}` setter instead")
            }
        }
    }
}

/// Returned by [`SipRequestOptions::with_header`] (and friends) when a
/// policy rule rejects the header.
#[derive(Debug, Clone)]
pub struct HeaderPolicyViolation {
    pub method: Method,
    pub header: HeaderName,
    pub reason: ViolationReason,
}

impl fmt::Display for HeaderPolicyViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "header policy violation on {}: {:?} — {}",
            self.method, self.header, self.reason
        )
    }
}

impl std::error::Error for HeaderPolicyViolation {}

/// Audit report returned by
/// [`SipRequestOptions::with_headers_from`]: which inbound headers
/// were copied through and which were filtered (and why).
#[derive(Default, Debug, Clone)]
pub struct HeaderCarryThroughReport {
    pub copied: Vec<HeaderName>,
    pub skipped: Vec<(HeaderName, ViolationReason)>,
}

/// The shared outbound-builder shape. See module docs.
pub trait SipRequestOptions: Sized + Send + Sync {
    /// The SIP method this builder will emit. Drives the
    /// `HeaderPolicy` matrix.
    fn method(&self) -> Method;

    /// Mutable access to the embedded `BuilderHeaderState`. Concrete
    /// types embed `BuilderHeaderState` as one of their fields and
    /// return `&mut self.header_state`.
    fn header_state_mut(&mut self) -> &mut BuilderHeaderState;

    /// Read-only access to the same state.
    fn header_state(&self) -> &BuilderHeaderState;

    /// Append one header.
    ///
    /// Errors when the header is stack-managed or has a dedicated
    /// setter on this builder. Under `BuilderStrictness::Lenient`,
    /// method-shaped headers downgrade to a `warn!` log and the
    /// header is silently dropped (the call still returns `Ok(self)`).
    fn with_header(mut self, header: TypedHeader) -> Result<Self, HeaderPolicyViolation> {
        let method = self.method();
        let name = header.name();
        let role = policy::classify(method.clone(), &name);
        match (&role, self.header_state().strictness) {
            (HeaderRole::StackManaged, _) => Err(HeaderPolicyViolation {
                method,
                header: name,
                reason: ViolationReason::StackManaged,
            }),
            (HeaderRole::MethodShaped { setter }, BuilderStrictness::Strict) => {
                Err(HeaderPolicyViolation {
                    method,
                    header: name,
                    reason: ViolationReason::UseDedicatedSetter(setter),
                })
            }
            (HeaderRole::MethodShaped { setter }, BuilderStrictness::Lenient) => {
                tracing::warn!(
                    method = %method,
                    header = ?name,
                    setter = setter,
                    "Builder Lenient mode: dropping method-shaped header; \
                     use the dedicated setter instead",
                );
                Ok(self)
            }
            (HeaderRole::ApplicationControlled, _) => {
                self.header_state_mut().headers.push(header);
                Ok(self)
            }
        }
    }

    /// Batch form of [`SipRequestOptions::with_header`]. Fails fast on the first violation.
    fn with_headers(self, headers: Vec<TypedHeader>) -> Result<Self, HeaderPolicyViolation> {
        let mut me = self;
        for h in headers {
            me = me.with_header(h)?;
        }
        Ok(me)
    }

    /// Stage a raw `name: value` header as `TypedHeader::Other`.
    ///
    /// The name is canonicalized so `"x-customer-id"` and
    /// `"X-CUSTOMER-ID"` produce identical wire output. Same policy
    /// check as [`SipRequestOptions::with_header`].
    fn with_raw_header(
        self,
        name: impl Into<HeaderName>,
        value: impl Into<String>,
    ) -> Result<Self, HeaderPolicyViolation> {
        let name = name.into();
        let canonical = canonicalize_header_name(name);
        let hv = HeaderValue::Raw(value.into().into_bytes());
        self.with_header(TypedHeader::Other(canonical, hv))
    }

    /// Drop any header named `name` that was added earlier in the
    /// builder chain (or via carry-through). Case-insensitive.
    fn strip_header(mut self, name: &HeaderName) -> Self {
        let state = self.header_state_mut();
        state
            .headers
            .retain(|h| !super::view::header_name_eq(&h.name(), name));
        self
    }

    /// B2BUA carry-through. Copy headers named `names` from `source`,
    /// filtering stack-managed ones automatically; the report lists
    /// every name that was filtered and why.
    fn with_headers_from<S: SipHeaderView>(
        mut self,
        source: &S,
        names: &[HeaderName],
    ) -> Result<(Self, HeaderCarryThroughReport), HeaderPolicyViolation> {
        let method = self.method();
        let mut report = HeaderCarryThroughReport::default();

        for name in names {
            if policy::forbidden_for_carry_through(name) {
                report
                    .skipped
                    .push((name.clone(), ViolationReason::StackManaged));
                continue;
            }
            for hdr in source.headers_named(name) {
                let role = policy::classify(method.clone(), &hdr.name());
                match role {
                    HeaderRole::StackManaged => {
                        report
                            .skipped
                            .push((name.clone(), ViolationReason::StackManaged));
                    }
                    HeaderRole::MethodShaped { setter } => {
                        report
                            .skipped
                            .push((name.clone(), ViolationReason::UseDedicatedSetter(setter)));
                    }
                    HeaderRole::ApplicationControlled => {
                        self.header_state_mut().headers.push(hdr.clone());
                        report.copied.push(name.clone());
                    }
                }
            }
        }
        Ok((self, report))
    }

    /// Inspect headers staged so far.
    fn staged_headers(&self) -> &[TypedHeader] {
        &self.header_state().headers
    }

    /// Set validation strictness. Builder default is `Strict`.
    fn with_strictness(mut self, mode: BuilderStrictness) -> Self {
        self.header_state_mut().strictness = mode;
        self
    }
}

/// Convert builder-staged headers into the `Vec<TypedHeader>` that
/// dialog-core's `extra_headers` channel accepts.
pub fn take_staged(state: &mut BuilderHeaderState) -> Vec<TypedHeader> {
    std::mem::take(&mut state.headers)
}

fn canonicalize_header_name(name: HeaderName) -> HeaderName {
    match name {
        HeaderName::Other(s) => HeaderName::Other(canonicalize_other(&s)),
        other => other,
    }
}

/// Title-case each token separated by '-' so `"x-customer-id"` becomes
/// `"X-Customer-Id"`. Pure ASCII; never touches multi-byte input.
fn canonicalize_other(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut start_of_word = true;
    for ch in s.chars() {
        if ch == '-' {
            out.push('-');
            start_of_word = true;
        } else if start_of_word {
            for upper in ch.to_uppercase() {
                out.push(upper);
            }
            start_of_word = false;
        } else {
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalize_simple() {
        assert_eq!(canonicalize_other("x-customer-id"), "X-Customer-Id");
        assert_eq!(canonicalize_other("X-CUSTOMER-ID"), "X-Customer-Id");
        assert_eq!(canonicalize_other("history-info"), "History-Info");
    }
}
