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

/// Diagnostic-only method view that preserves standard-method names while
/// reducing peer/application-controlled extension spellings to a bounded
/// class and length.
pub(crate) struct MethodDiagnostic<'a>(pub(crate) &'a Method);

impl fmt::Display for MethodDiagnostic<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Method::Extension(value) => write!(formatter, "extension(len={})", value.len()),
            method => fmt::Display::fmt(method, formatter),
        }
    }
}

impl fmt::Debug for MethodDiagnostic<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Method::Extension(value) => formatter
                .debug_struct("Extension")
                .field("value_len", &value.len())
                .finish(),
            method => fmt::Debug::fmt(method, formatter),
        }
    }
}

/// Diagnostic-only header-name view. Standard names are a fixed allowlist;
/// custom spellings are reported only as a class and byte length.
pub(crate) struct HeaderNameDiagnostic<'a>(pub(crate) &'a HeaderName);

impl fmt::Display for HeaderNameDiagnostic<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            HeaderName::Other(value) => write!(formatter, "custom(len={})", value.len()),
            name => formatter.write_str(name.as_str()),
        }
    }
}

impl fmt::Debug for HeaderNameDiagnostic<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            HeaderName::Other(value) => formatter
                .debug_struct("Other")
                .field("name_len", &value.len())
                .finish(),
            name => fmt::Debug::fmt(name, formatter),
        }
    }
}

/// Debug-list adapter used by errors that retain application-controlled
/// header names as live fields.
pub(crate) struct HeaderNamesDiagnostic<'a>(pub(crate) &'a [HeaderName]);

impl fmt::Debug for HeaderNamesDiagnostic<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut list = formatter.debug_list();
        for name in self.0 {
            list.entry(&HeaderNameDiagnostic(name));
        }
        list.finish()
    }
}

/// Shared per-builder state. Embedded by every concrete builder and
/// surfaced to the trait defaults through
/// [`SipRequestOptions::header_state_mut`].
///
/// Its `Debug` output reports only the header count and strictness so staged
/// application header names and values cannot enter diagnostics.
#[derive(Default, Clone)]
pub struct BuilderHeaderState {
    /// Application-staged headers in wire order. Stack-managed headers
    /// (Call-ID, Via, CSeq, …) are never visible here; they are stamped
    /// by the dialog/transaction layer after this list is forwarded.
    pub headers: Vec<TypedHeader>,
    /// Validation strictness. Defaults to
    /// [`BuilderStrictness::Strict`].
    pub strictness: BuilderStrictness,
}

impl fmt::Debug for BuilderHeaderState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BuilderHeaderState")
            .field("header_count", &self.headers.len())
            .field("strictness", &self.strictness)
            .finish()
    }
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
#[derive(Clone)]
pub struct HeaderPolicyViolation {
    /// The SIP method the offending builder targets.
    pub method: Method,
    /// The header that was rejected.
    pub header: HeaderName,
    /// Why the header was rejected.
    pub reason: ViolationReason,
}

impl fmt::Display for HeaderPolicyViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "header policy violation on {}: {} — {}",
            MethodDiagnostic(&self.method),
            HeaderNameDiagnostic(&self.header),
            self.reason
        )
    }
}

impl fmt::Debug for HeaderPolicyViolation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HeaderPolicyViolation")
            .field("method", &MethodDiagnostic(&self.method))
            .field("header", &HeaderNameDiagnostic(&self.header))
            .field("reason", &self.reason)
            .finish()
    }
}

impl std::error::Error for HeaderPolicyViolation {}

/// Audit report returned by
/// [`SipRequestOptions::with_headers_from`]: which inbound headers
/// were copied through and which were filtered (and why).
#[derive(Default, Clone)]
pub struct HeaderCarryThroughReport {
    /// Headers that were copied through from the source.
    pub copied: Vec<HeaderName>,
    /// Headers that were filtered out, each paired with the reason.
    pub skipped: Vec<(HeaderName, ViolationReason)>,
}

impl fmt::Debug for HeaderCarryThroughReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let stack_managed_count = self
            .skipped
            .iter()
            .filter(|(_, reason)| matches!(reason, ViolationReason::StackManaged))
            .count();
        let wrong_method_count = self
            .skipped
            .iter()
            .filter(|(_, reason)| matches!(reason, ViolationReason::WrongMethod))
            .count();
        let dedicated_setter_count = self
            .skipped
            .iter()
            .filter(|(_, reason)| matches!(reason, ViolationReason::UseDedicatedSetter(_)))
            .count();

        formatter
            .debug_struct("HeaderCarryThroughReport")
            .field("copied_count", &self.copied.len())
            .field("skipped_count", &self.skipped.len())
            .field("stack_managed_count", &stack_managed_count)
            .field("wrong_method_count", &wrong_method_count)
            .field("dedicated_setter_count", &dedicated_setter_count)
            .finish()
    }
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
        let header = canonicalize_typed_header_name(header);
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
                    method = %MethodDiagnostic(&method),
                    header = %HeaderNameDiagnostic(&name),
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
/// rvoip-sip-dialog's `extra_headers` channel accepts.
pub fn take_staged(state: &mut BuilderHeaderState) -> Vec<TypedHeader> {
    std::mem::take(&mut state.headers)
}

fn canonicalize_header_name(name: HeaderName) -> HeaderName {
    match name {
        HeaderName::Other(s) => match s.parse::<HeaderName>() {
            Ok(HeaderName::Other(_)) | Err(_) => HeaderName::Other(canonicalize_other(&s)),
            Ok(known) => known,
        },
        other => other,
    }
}

fn canonicalize_typed_header_name(header: TypedHeader) -> TypedHeader {
    match header {
        TypedHeader::Other(name, value) => {
            TypedHeader::Other(canonicalize_header_name(name), value)
        }
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

    #[test]
    fn canonicalize_recognized_other_names_to_typed_identity() {
        for (alias, expected) in [
            ("call-ID", HeaderName::CallId),
            ("i", HeaderName::CallId),
            ("V", HeaderName::Via),
            ("l", HeaderName::ContentLength),
            ("route", HeaderName::Route),
            ("AUTHORIZATION", HeaderName::Authorization),
            ("Proxy-AUTHORIZATION", HeaderName::ProxyAuthorization),
        ] {
            assert_eq!(
                canonicalize_header_name(HeaderName::Other(alias.into())),
                expected
            );
        }
        assert_eq!(
            canonicalize_header_name(HeaderName::Other("x-CUSTOM".into())),
            HeaderName::Other("X-Custom".into())
        );

        let header = canonicalize_typed_header_name(TypedHeader::Other(
            HeaderName::Other("sUbJeCt".into()),
            HeaderValue::Raw(b"hello".to_vec()),
        ));
        assert_eq!(header.name(), HeaderName::Subject);
    }

    #[test]
    fn builder_header_state_debug_reports_shape_without_header_values() {
        let state = BuilderHeaderState {
            headers: vec![TypedHeader::Other(
                HeaderName::Other("X-Secret-Canary".into()),
                HeaderValue::Raw(b"builder-header-secret-canary".to_vec()),
            )],
            strictness: BuilderStrictness::Lenient,
        };

        let debug = format!("{state:?}");
        assert!(debug.contains("header_count: 1"));
        assert!(debug.contains("strictness: Lenient"));
        assert!(!debug.contains("X-Secret-Canary"));
        assert!(!debug.contains("builder-header-secret-canary"));
    }

    #[test]
    fn header_policy_diagnostics_redact_extension_method_and_custom_header_spelling() {
        const METHOD_CANARY: &str = "CUSTOM\r\nX-Method-Canary: exposed";
        const HEADER_CANARY: &str = "X-Header-Canary\r\nInjected";
        let violation = HeaderPolicyViolation {
            method: Method::Extension(METHOD_CANARY.to_string()),
            header: HeaderName::Other(HEADER_CANARY.to_string()),
            reason: ViolationReason::StackManaged,
        };

        let display = violation.to_string();
        let debug = format!("{violation:?}");
        for rendered in [&display, &debug] {
            assert!(
                !rendered.contains(METHOD_CANARY),
                "method leaked: {rendered}"
            );
            assert!(
                !rendered.contains(HEADER_CANARY),
                "header leaked: {rendered}"
            );
        }
        assert!(display.contains(&format!("extension(len={})", METHOD_CANARY.len())));
        assert!(display.contains(&format!("custom(len={})", HEADER_CANARY.len())));
        assert!(debug.starts_with("HeaderPolicyViolation"));
        assert!(debug.contains(&format!("value_len: {}", METHOD_CANARY.len())));
        assert!(debug.contains(&format!("name_len: {}", HEADER_CANARY.len())));

        // The application-facing fields remain intact for policy handling.
        assert_eq!(
            violation.method,
            Method::Extension(METHOD_CANARY.to_string())
        );
        assert_eq!(
            violation.header,
            HeaderName::Other(HEADER_CANARY.to_string())
        );
    }

    #[test]
    fn standard_header_policy_debug_shape_remains_actionable() {
        let violation = HeaderPolicyViolation {
            method: Method::Invite,
            header: HeaderName::CallId,
            reason: ViolationReason::StackManaged,
        };

        assert_eq!(
            violation.to_string(),
            "header policy violation on INVITE: Call-ID — owned by the dialog/transaction layer"
        );
        assert_eq!(
            format!("{violation:?}"),
            "HeaderPolicyViolation { method: Invite, header: CallId, reason: StackManaged }"
        );
    }

    #[test]
    fn carry_through_report_debug_exposes_counts_not_custom_names() {
        const COPIED_CANARY: &str = "X-Copied-Canary";
        const SKIPPED_CANARY: &str = "X-Skipped-Canary";
        let report = HeaderCarryThroughReport {
            copied: vec![HeaderName::Other(COPIED_CANARY.to_string())],
            skipped: vec![
                (
                    HeaderName::Other(SKIPPED_CANARY.to_string()),
                    ViolationReason::StackManaged,
                ),
                (
                    HeaderName::Authorization,
                    ViolationReason::UseDedicatedSetter("with_credentials"),
                ),
            ],
        };

        let debug = format!("{report:?}");
        assert!(debug.starts_with("HeaderCarryThroughReport"));
        assert!(debug.contains("copied_count: 1"));
        assert!(debug.contains("skipped_count: 2"));
        assert!(debug.contains("stack_managed_count: 1"));
        assert!(debug.contains("dedicated_setter_count: 1"));
        assert!(!debug.contains(COPIED_CANARY));
        assert!(!debug.contains(SKIPPED_CANARY));

        // The report still carries exact names for application decisions.
        assert_eq!(
            report.copied[0],
            HeaderName::Other(COPIED_CANARY.to_string())
        );
    }
}
