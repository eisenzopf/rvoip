---
section: Security
breaking: false
---
**Explicit rustls crypto provider.** Every rustls client and server rvoip
builds now names its provider (ring) instead of using rustls's process-level
default. Applications whose dependencies compile both rustls providers
(`ring` and `aws-lc-rs`, for example through an AWS SDK) no longer panic with
"Could not automatically determine the process-level CryptoProvider". This
covers:

- SIP TLS and WSS, client and server, including client-certificate
  verification;
- the WebRTC WHIP/WHEP HTTPS and WSS listeners, and outbound WHIP and
  WebSocket signaling;
- the users-core HTTPS listener;
- the HTTP clients in auth-core (JWKS, token introspection), OIDC, Keycloak,
  audit, IMS-AKA, STIR/SHAKEN and Vapi;
- the Vapi, Amazon Connect (Chime) and UCTP WebSocket clients, and the UCTP
  QUIC client;
- LDAP.

Three dependencies can only use rustls's process-level default:

- the `dtls` crate behind DTLS-SRTP (`rvoip-rtp-core` `dtls-webrtc`, used by
  SIP DTLS-SRTP calls);
- redis TLS (`rediss://` in `rvoip-redis`);
- sqlx Postgres TLS (`rvoip-vcon-postgres`).

If no process-level rustls CryptoProvider is installed when one of those
paths first runs, rvoip installs ring; install your own at startup to choose.
rvoip never replaces a provider that is already installed, and installs one
nowhere else.
