# rvoip-sip Auth Examples

These examples show how SIP auth surfaces work with first-party and external
auth providers.

- `auth_users_core_service`: users-core issues a Bearer JWT, stores SIP Digest
  HA1 material, and backs `SipAuthService` through auth-core traits.
- `auth_endpoint_register_users_core`: real local SIP REGISTER flow where an
  `Endpoint` UAC is challenged by a `UnifiedCoordinator` UAS/registrar and
  retries with users-core-backed SIP Digest credentials.
- `auth_enterprise_hooks`: local provider example for `AuthAuditSink`,
  `AuthRateLimiter`, and `DigestReplayStore`.
- `auth_keycloak_bearer_provider`: optional Keycloak/OIDC example. It loads
  `RVOIP_KEYCLOAK_*` environment variables, or `$HOME/Developer/keycloak/keycloak-local.env`
  when present, and exits successfully with a skip message when Keycloak is not
  configured.

The examples keep SIP networking minimal so they can run locally without a PBX.
PBX interop examples remain under `examples/pbx`.

## Run

```sh
cargo run -p rvoip-sip --example auth_users_core_service
cargo run -p rvoip-sip --example auth_endpoint_register_users_core
cargo run -p rvoip-sip --example auth_enterprise_hooks
```

For Keycloak:

```sh
. ~/Developer/keycloak/keycloak-local.env
cargo run -p rvoip-sip --example auth_keycloak_bearer_provider
```
