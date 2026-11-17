---
section: Breaking changes
breaking: true
---
**reqwest 0.12 → 0.13.** `rvoip_oidc::OidcProvider::from_metadata` takes a
reqwest 0.13 `Client`. rvoip builds reqwest with `rustls-no-provider` on
the workspace's ring rustls provider: every rvoip client constructor
installs ring as the process-default provider when none is installed yet,
and reqwest 0.13 verifies server certificates with the platform verifier
(the OS trust store) instead of the bundled webpki roots. Migration: build
the client you pass to `from_metadata` with reqwest 0.13; an application
that wants another rustls provider installs it before creating rvoip
clients.
