# Auth Incident Response Runbook

## JWT Signing Key Compromise

1. Stop issuing tokens with the compromised key.
2. Publish a new key and update issuers.
3. Remove the compromised key from JWKS after validators are updated.
4. Revoke refresh tokens for affected users or clients.
5. Enable access-token revocation or introspection until the longest affected
   access-token TTL has elapsed.
6. Review audit events for token validation failures and unusual scope use.

## Bearer Token Leak

1. Revoke the token JTI or opaque token identifier.
2. Revoke related refresh tokens if the leak source is unknown.
3. Lower token TTL temporarily if broad exposure is suspected.
4. Search audit logs by subject, `jti`, peer, issuer, and client id.

## Password Or Basic Credential Compromise

1. Disable or reset the affected user credential.
2. Revoke refresh tokens and API keys for the user.
3. Rotate SIP Digest HA1 credentials if SIP and login identities are linked.
4. Review rate-limit and failed-auth telemetry.

## SIP Digest Replay Or Nonce Abuse

1. Confirm shared `DigestReplayStore` is configured for all UAS nodes.
2. Check Redis/shared store health and clock skew.
3. Rotate affected Digest credentials if replay indicates credential exposure.
4. Increase monitoring on nonce-count rejection and stale nonce spikes.

## LDAP Or IdP Outage

1. Confirm provider health from `rvoip-oidc`, `rvoip-keycloak`, or LDAP probes.
2. Expect credential validation to fail closed.
3. Do not bypass validator failures by enabling cleartext Basic or static
   fallback secrets without an approved emergency change.
4. Communicate impact and restore provider connectivity.

## Audit Sink Outage

1. Determine whether deployment is fail-open or fail-closed for audit.
2. Restore sink connectivity or disk capacity.
3. If fail-open, reconcile local logs and provider logs for the outage window.
4. If fail-closed, restore service only after audit delivery is healthy or an
   approved exception is recorded.
