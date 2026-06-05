# rvoip-sip Auth Examples

These examples show how SIP auth surfaces work with first-party and external
auth providers.

- `auth_users_core_service`: users-core issues a Bearer JWT, stores SIP Digest
  HA1 material, and backs `SipAuthService` through auth-core traits.
- `auth_endpoint_register_users_core`: real local SIP REGISTER flow where an
  `Endpoint` UAC is challenged by a `UnifiedCoordinator` UAS/registrar and
  retries with users-core-backed SIP Digest credentials.
- `auth_endpoint_invite_bearer`: real local SIP INVITE flow where an
  `Endpoint` UAC is challenged by a `UnifiedCoordinator` UAS and retries with
  Bearer auth.
- `auth_endpoint_invite_digest`: real local SIP INVITE flow where an
  `Endpoint` UAC is challenged by a `UnifiedCoordinator` UAS and retries with
  users-core-backed SIP Digest auth.
- `auth_enterprise_hooks`: local provider example for `AuthAuditSink`,
  `AuthRateLimiter`, and `DigestReplayStore`.
- `auth_keycloak_bearer_provider`: optional Keycloak/OIDC example. It loads
  `RVOIP_KEYCLOAK_*` environment variables, or `$HOME/Developer/keycloak/keycloak-local.env`
  when present, and exits successfully with a skip message when Keycloak is not
  configured.
- `auth_redis_enterprise_hooks`: optional Redis-backed replay, revocation, and
  rate-limit provider example. Skips unless `RVOIP_REDIS_URL` is set.
- `auth_generic_oidc_provider`: optional generic OIDC discovery/JWKS/
  introspection example. Skips unless `RVOIP_OIDC_ISSUER` is set.
- `auth_ldap_basic_provider`: optional LDAP-backed Basic-over-TLS verifier
  example. Skips unless `RVOIP_LDAP_URL` is set.
- `auth_scim_users_core`: local SCIM provisioning into users-core with a
  fake Bearer admin validator.
- `auth_saml_users_core`: local SAML service-provider adapter shape with a
  fake already-verified signed assertion.
- `auth_webauthn_passkeys`: local WebAuthn/passkey registration ceremony start
  backed by users-core passkey storage.
- `auth_ims_aka_provider`: deterministic IMS AKA provider-shape example for
  `AkaClientProvider` / `AkaVectorProvider`.
- `auth_custom_provider`: custom external provider shape for Basic and Digest.

The examples keep SIP networking minimal so they can run locally without a PBX.
PBX interop examples remain under `examples/pbx`.

## Run

```sh
cargo run -p rvoip-sip --example auth_users_core_service
cargo run -p rvoip-sip --example auth_endpoint_register_users_core
cargo run -p rvoip-sip --example auth_endpoint_invite_bearer
cargo run -p rvoip-sip --example auth_endpoint_invite_digest
cargo run -p rvoip-sip --example auth_enterprise_hooks
cargo run -p rvoip-sip --example auth_scim_users_core
cargo run -p rvoip-sip --example auth_saml_users_core
cargo run -p rvoip-sip --example auth_webauthn_passkeys
cargo run -p rvoip-sip --example auth_ims_aka_provider
cargo run -p rvoip-sip --example auth_custom_provider
```

For Keycloak:

```sh
. ~/Developer/keycloak/keycloak-local.env
cargo run -p rvoip-sip --example auth_keycloak_bearer_provider
```

For optional provider fixtures:

```sh
RVOIP_REDIS_URL=redis://127.0.0.1:6379 \
  cargo run -p rvoip-sip --example auth_redis_enterprise_hooks

RVOIP_OIDC_ISSUER=https://idp.example.com/realms/rvoip \
  RVOIP_OIDC_AUDIENCE=rvoip-sip \
  cargo run -p rvoip-sip --example auth_generic_oidc_provider

RVOIP_LDAP_URL=ldap://127.0.0.1:1389 \
  RVOIP_LDAP_BIND_DN='cn=admin,dc=rvoip,dc=local' \
  RVOIP_LDAP_BIND_PASSWORD=adminpassword \
  RVOIP_LDAP_USER_BASE_DN='ou=users,dc=rvoip,dc=local' \
  cargo run -p rvoip-sip --example auth_ldap_basic_provider
```
