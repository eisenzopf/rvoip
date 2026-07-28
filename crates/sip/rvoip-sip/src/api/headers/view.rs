//! `SipHeaderView` — uniform inbound-header inspection.
//!
//! Implemented by every wrapper around a received SIP message
//! (`IncomingCall`, `IncomingRequest`, `IncomingResponse`,
//! `IncomingRegister`). The shape is identical across them so that
//! gateway / B2BUA / SBC code can write generic carry-through logic
//! over `S: SipHeaderView`.
//!
//! Raw `&Arc<Request>` / `&Arc<Response>` access lives on the
//! concrete inherent impls — not on the trait — because the trait
//! must remain object-safe.

use rvoip_sip_core::types::headers::{HeaderName, TypedHeader};

/// Common header-inspection surface for inbound SIP messages.
///
/// All matching is case-insensitive per RFC 3261 §7.3.1; concrete
/// implementations canonicalize header names before comparison.
pub trait SipHeaderView {
    /// First header matching `name`, typed when sip-core has a variant
    /// for it. Returns `None` when absent.
    fn header(&self, name: &HeaderName) -> Option<&TypedHeader>;

    /// Every header matching `name`, in wire order. Returns an empty
    /// iterator when none. Boxed for object safety; concrete types
    /// additionally expose zero-alloc `headers_named_iter()` inherent
    /// accessors.
    fn headers_named<'a>(
        &'a self,
        name: &HeaderName,
    ) -> Box<dyn Iterator<Item = &'a TypedHeader> + 'a>;

    /// All headers in wire order.
    fn headers<'a>(&'a self) -> Box<dyn Iterator<Item = &'a TypedHeader> + 'a>;

    /// Header value as a string via `TypedHeader::Display`. For
    /// `TypedHeader::Other`, this reproduces the inbound wire value.
    fn header_str(&self, name: &HeaderName) -> Option<String> {
        self.header(name).map(|h| h.to_string())
    }

    /// All header names present, deduped, in first-seen order.
    fn header_names(&self) -> Vec<HeaderName>;
}

/// Internal helper: compare two HeaderNames, treating `Other(s)` as
/// case-insensitive on `s` per RFC 3261 §7.3.1. The other variants
/// are unit-like so equality is already correct.
pub(crate) fn header_name_eq(a: &HeaderName, b: &HeaderName) -> bool {
    a.wire_eq(b)
}

/// Internal helper that implements the default trait body over a
/// `&[TypedHeader]` slice. Concrete wrappers delegate to this so the
/// behaviour is identical across implementors.
pub(crate) fn header_slice<'a>(
    slice: &'a [TypedHeader],
    name: &HeaderName,
) -> Option<&'a TypedHeader> {
    slice.iter().find(|h| header_name_eq(&h.name(), name))
}

pub(crate) fn headers_named_slice<'a>(
    slice: &'a [TypedHeader],
    name: &'a HeaderName,
) -> impl Iterator<Item = &'a TypedHeader> + 'a {
    slice
        .iter()
        .filter(move |h| header_name_eq(&h.name(), name))
}

pub(crate) fn header_names_slice(slice: &[TypedHeader]) -> Vec<HeaderName> {
    let mut out: Vec<HeaderName> = Vec::with_capacity(slice.len());
    for h in slice {
        let n = h.name();
        if !out.iter().any(|existing| header_name_eq(existing, &n)) {
            out.push(n);
        }
    }
    out
}
