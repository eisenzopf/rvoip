# SRTP transport status in 0.3.8

The direct pre-shared-key media path installs SRTP before reporting a connected
client as secure. An SRTP-enabled transport never falls back to plaintext when
its context is missing, disabled, or fails authentication.

## Supported boundary

- `UdpRtpTransport` protects normal RTP sends after directional contexts are
  installed. Its public raw-byte send is rejected in secure mode.
- `UdpRtpTransport::receive_packet` authenticates and decrypts RTP in secure
  mode. Plaintext and bad-authentication datagrams return an error.
- `SecurityRtpTransport` owns a separate authenticated interceptor. To avoid a
  second reader racing the same socket or double-consuming SRTP state, direct
  `receive_packet` is unavailable in secure mode; consumers use its event
  subscription.
- A wrapper created for plaintext cannot later accept an SRTP context.
- Standalone PSK “security context” types validate configuration only and do
  not claim that a media transport is secure.

## Deliberate restrictions

SRTCP state is not implemented in this release. Once SRTP is configured, RTCP
send and receive paths reject unauthenticated RTCP. Raw socket handles are
documented low-level escapes that bypass transport security and are never proof
of a protected media path.

Stateful per-SSRC SRTP/SRTCP rollover, replay, and transport ownership changes
belong to the dedicated SRTP state repair. Until then, no compatibility path may
silently return or emit unauthenticated media.
