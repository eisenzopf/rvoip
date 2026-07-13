# Secure Configuration Checklist

## Required

- [ ] SIP Basic and Bearer cleartext remain disabled unless a written exception
  exists.
- [ ] TLS/WSS is configured for credential-bearing SIP traffic.
- [ ] Bearer validators enforce issuer and audience/resource.
- [ ] Bearer validators enforce expiry and accepted algorithms.
- [ ] JWT/JWKS validators check `kid` behavior.
- [ ] Access-token revocation, OAuth2 introspection, or short TTLs satisfy the
  revocation requirement.
- [ ] Clustered SIP UAS deployments use shared `DigestReplayStore`.
- [ ] Every enabled `SipListenerAuthPolicy` has a validated explicit tenant;
  Bearer and trusted-CIDR/mTLS principals match it exactly.
- [ ] SIP mTLS fingerprint mappings are paired with an Optional or Required
  TLS client-certificate policy and an explicit client CA bundle.
- [ ] Rate limiter is configured for REGISTER, Basic/password, Digest, Bearer,
  API-key, and token issuance paths.
- [ ] Audit sink is configured and redaction has been verified.
- [ ] Audit failure policy is documented.
- [ ] Digest MD5 support is documented as PBX compatibility when enabled.
- [ ] SIP Digest HA1 values are stored as secrets.
- [ ] users-core production signing keys are configured and not generated
  ad hoc at startup.
- [ ] Redis, LDAP, OIDC, and database credentials come from a secret manager or
  equivalent secure configuration.

## Recommended

- [ ] Prefer RS256/ES256 JWKS over HS256 for multi-service deployments.
- [ ] Use Redis TLS and ACLs for shared replay/revocation/rate-limit state.
- [ ] Use LDAPS or StartTLS for LDAP verification.
- [ ] Keep access-token TTLs short.
- [ ] Rotate JWT signing keys and SIP Digest credentials on a fixed schedule.
- [ ] Export auth audit events to a SIEM or centralized log store.
- [ ] Run negative auth tests before release: issuer, audience, `kid`, expiry,
  revocation, Basic cleartext, Digest replay, stale nonce, and provider outage.
