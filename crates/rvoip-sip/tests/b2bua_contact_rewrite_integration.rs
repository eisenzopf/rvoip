//! SIP_API_DESIGN_2 §10 verification #16 —
//! `OutboundCallBuilder::with_contact_uri(...)` rewrites the Contact
//! header on the outbound INVITE.
//!
//! **Skeleton-only.** The builder accepts `with_contact_uri(...)` and
//! stages the value into `OutboundCallOptionsSnapshot.contact_uri`,
//! but `Action::SendINVITEWithOptions` does not currently thread it
//! through to dialog-core: the dialog-core entry
//! `make_call_with_extra_headers_for_session` has no `contact_uri`
//! parameter, and dialog-core stamps Contact from the local socket
//! address. Wiring this end-to-end requires extending the dialog-core
//! INVITE API (additive, per the spec's §7.2 carve-out for
//! contact_uri overrides). Out of scope for the SIP_API_DESIGN_2
//! audit closeout; tracked separately as a follow-up.
//!
//! Wire-side coverage of in-dialog Contact rewriting today is
//! exercised on the REGISTER builder (see
//! `third_party_register_integration::third_party_register_rewrites_from_contact_and_pai_on_wire`).

#[test]
#[ignore = "needs dialog-core API extension to thread INVITE Contact override (see file header)"]
fn outbound_call_builder_rewrites_contact_uri() {}
