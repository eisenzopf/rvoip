# Security compatibility notes

## Composite principal subjects

`AuthenticatedPrincipal::from_assurance` now derives `TaskScoped` and
`UserAuthorized` subjects from a domain-separated SHA-256 digest of
length-prefixed components. This intentionally changes the generated subject
and therefore the `PrincipalOwnershipKey` for those compatibility-mapped
assurance values. It closes delimiter collisions between distinct identities.

This is a wire/runtime identity compatibility break: processes on opposite
sides of an ownership check must run the same derivation revision. Drain active
sessions before upgrading a clustered deployment. Validators that already
construct an explicit issuer/tenant/subject principal are unaffected.

## Step-up providers

`IdentityProvider::authenticate_principal` is an additive method with a
fail-closed default. `Orchestrator::complete_step_up` no longer accepts the
legacy identity-plus-assurance projection because it cannot prove issuer,
tenant, subject, or credential expiry. Providers that support step-up must
return a complete principal whose ownership key exactly matches the existing
connection and whose scopes match its assurance.

## users-core API keys and security storage

`AuthenticationService::authenticate_api_key` implements the versioned
`users-core.api-key-token-exchange.disabled.v1` contract and returns
`Error::ApiKeyTokenExchangeDisabled`. Direct verification through
`verify_api_key_only` remains supported and preserves the exact key
permissions. JWT exchange can be reintroduced only with key-bound claims and
durable key-specific refresh/access revocation lineage.

Credential issuance, refresh, validation-time access-token revocation checks,
logout/revocation, and password changes now return
`Error::SecurityStoreUnavailable` when `AuthenticationService::new` was not
paired with an `AuthSecurityStore`. The standard `users_core::init` path already
installs that store. The two error variants are additive public enum variants;
downstream exhaustive matches must add fallback arms.

The public `api::AuthContext` shape remains compatible with its original five
fields. Access-token JTI and expiry data used by logout are retained only in a
private request extractor.
