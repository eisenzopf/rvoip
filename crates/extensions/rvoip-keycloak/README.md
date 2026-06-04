# rvoip-keycloak

Optional Keycloak/OIDC helpers for RVoIP authentication.

Core protocol crates depend on `rvoip-auth-core` traits, not on Keycloak. Use
this crate when a deployment wants a concrete Keycloak integration or a
repeatable local fixture.

## What It Provides

- Keycloak realm URL construction.
- OIDC discovery from `.well-known/openid-configuration`.
- JWKS-backed Bearer validators that enforce issuer and configured audience.
- OAuth2 introspection Bearer validators built from discovered
  `introspection_endpoint` metadata.
- Configurable JWKS cache TTL through `KeycloakConfig::with_jwks_cache_ttl`.
- Keycloak-compatible claim mapping through auth-core validators:
  `scope`/`scopes` claims become scopes, realm roles become
  `realm:<role>`, and client roles become `<client_id>:<role>`.
- Provider health checks for discovered issuer, JWKS reachability, optional
  introspection/revocation endpoints, and configured audience.
- A password-grant client for local tests and demos.

The password-grant client is not a production login recommendation. Production
applications should use their normal OAuth/OIDC login flow and pass resulting
tokens to SIP UAC or UAS auth surfaces.

## Local Fixture

The local Keycloak fixture lives outside this repo at:

`/Users/jonathan/Developer/keycloak`

Start it with:

```sh
/Users/jonathan/Developer/keycloak/scripts/up.sh
. /Users/jonathan/Developer/keycloak/keycloak-local.env
cargo test -p rvoip-keycloak --test keycloak_live
```

The same environment also enables the auth-core JWKS integration test:

```sh
cargo test -p rvoip-auth-core --test keycloak_jwks
```
