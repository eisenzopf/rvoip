# Auth Key Management Runbook

## JWT Signing Keys

- Use RS256/ES256 or another asymmetric algorithm for production JWKS.
- Assign a stable `kid` to every signing key.
- Publish only public keys through JWKS.
- Store private keys in a production secret manager or HSM-equivalent service.
- Rotate by publishing the new public key before issuing tokens with the new
  `kid`.
- Keep old public keys until all old access tokens expire.
- Emergency rotation requires revoking active refresh tokens and enabling
  access-token revocation or introspection for the affected token population.

## HS256 Secrets

- Treat HS256 as shared-secret mode.
- Do not use development-generated secrets in production.
- Rotate by configuring validators and issuers with overlapping old/new
  secrets only if the deployment explicitly supports key IDs for HMAC keys.
- Prefer asymmetric signing where multiple services validate tokens.

## SIP Digest HA1

- HA1 is password-equivalent for `(username, realm, algorithm family)`.
- Store HA1 as secret material.
- Do not derive SIP Digest from Argon2 login password hashes.
- Rotate SIP Digest credentials independently from login passwords unless the
  application deliberately couples those workflows.

## LDAP Bind Credentials

- Use a least-privilege service account that can search only required user
  branches and attributes.
- Store bind DN/password in a secret manager.
- Prefer LDAPS or StartTLS for LDAP service binds and user binds.
- Rotate LDAP bind credentials on a fixed schedule and after any suspected
  directory compromise.

## Redis Credentials

- Use TLS and ACLs for production Redis.
- Scope credentials to the RVoIP auth key namespace where supported.
- Set persistence and backup policy according to revocation/replay durability
  requirements.
- Rotate Redis credentials and flush only test namespaces during development.

## Audit Sink Credentials

- Treat SIEM/OTLP/webhook credentials as secrets.
- Validate that audit events are redacted before export.
- Use fail-closed audit policy only when the deployment can tolerate traffic
  interruption during audit outages.
