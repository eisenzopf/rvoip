# RVoIP Auth Security Review Packet

This packet collects the auth/security evidence a reviewer needs before
approving an enterprise SIP deployment.

Documents:

- `ARCHITECTURE.md`: crate boundaries, trust boundaries, and provider model.
- `DATA_FLOWS.md`: UAC/UAS Digest, Bearer, Basic, LDAP, OIDC, Redis, and
  users-core flows.
- `CONTROLS.md`: control mapping for least privilege, transport protection,
  replay protection, revocation, audit, and rate limiting.
- `KEY_MANAGEMENT_RUNBOOK.md`: signing keys, JWKS, HMAC secrets, Digest HA1,
  LDAP bind secrets, Redis credentials, and rotation.
- `INCIDENT_RESPONSE_RUNBOOK.md`: response steps for key compromise, token
  compromise, password compromise, replay attacks, audit outage, and IdP
  outage.
- `SECURE_CONFIGURATION_CHECKLIST.md`: deployment checklist.
- `KNOWN_LIMITATIONS.md`: remaining gaps and required compensating controls.

Primary implementation tracker:

- `../COMPLETE_AUTH_USER_SERVICE_PLAN.md`
