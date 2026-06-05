# rvoip-oidc

Generic OIDC discovery helpers for RVoIP Bearer authentication.

Use this crate when an application uses an OIDC-compatible identity provider
that is not Keycloak-specific. It discovers provider metadata, constructs JWKS
and OAuth2 introspection validators from `rvoip-auth-core`, and reports basic
provider health.

Protocol crates should still depend on `rvoip-auth-core` traits, not on this
extension directly.
