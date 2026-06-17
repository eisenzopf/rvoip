# rvoip-identity

> ⚠️ **Alpha** (`0.1.x`) — early and API-unstable; expect breaking changes before `1.0`.

Minimal identity-provider crate for `rvoip-core::IdentityProvider`.

Current shipped backend:

- `BearerProvider`: an in-memory bearer-token table for dev/test and simple deployments.

OAuth 2.1, DPoP, OIDC, SIP Digest, Passkey/WebAuthn, SCIM/SAML/LDAP, AAuth, and related production auth pieces live in `rvoip-auth-core`, `rvoip-users-core`, and the dedicated extension crates. They are not implemented by this crate today.

Part of the [**rvoip**](https://github.com/eisenzopf/rvoip) workspace (the "rvoip 3"
unified real-time-communications stack). Published so the
[`rvoip`](https://crates.io/crates/rvoip) facade can expose it behind the `voip-3`
feature — see the [workspace README](https://github.com/eisenzopf/rvoip) and
`docs/INTERFACE_DESIGN.md` for how it fits into the architecture.

## License

Licensed under the MIT License — see [LICENSE](https://github.com/eisenzopf/rvoip/blob/main/LICENSE).
