# rvoip-rtp-core 0.3.5 security migration

Version 0.3.5 deliberately fails closed for security mechanisms that were
present in the public API but were not complete. Most 0.3.4 call sites remain
source compatible. The exceptions below require an explicit migration because
preserving their old behavior would continue to advertise, select, or execute
an incomplete security path. Security configuration should now be validated at
construction boundaries rather than inferred from a preset name.

## DTLS

`dtls::create_connection(DtlsConfig) -> Result<DtlsConnection>` retains its
0.3.4 signature. It now validates the requested SRTP profiles and returns
`Error::UnsupportedFeature` instead of constructing or panicking inside the
incomplete DTLS implementation.

The bypass APIs `DtlsConnection::new` and
`dtls::handshake::HandshakeState` are no longer public. There is no supported
DTLS migration in 0.3.5; select explicitly provisioned direct SRTP or SDES instead and
handle the typed unsupported error if DTLS is requested.

The public client/server DTLS transport helpers also return
`UnsupportedFeature` before opening a DTLS transport, spawning a handshake
task, or consuming a datagram. A server-managed security context reports DTLS
secure only when it has a live connection, a completed handshake, and an
installed SRTP context; constructing its retained public fields cannot make an
uninitialized context appear secure.

The `SecurityMode`, `SecurityProfile`, `SecurityConfig`,
`ClientSecurityConfig`, and `ServerSecurityConfig` defaults are now explicitly
unsecured. Callers that require security must select SDES or provide exactly
one direct-SRTP suite plus key material; defaults no longer imply that the
unavailable DTLS stack will protect traffic.

`SecurityConfig`, `ClientSecurityConfig`, and `ServerSecurityConfig` now expose
`validate()`. `ClientConfigBuilder::try_build()` is the checked alternative to
the source-compatible infallible client `build()`. Server
`ServerConfigBuilder::build()` now performs security validation and returns an
error for the unprovisioned `sip()` preset, unavailable `webrtc()` preset, or
any incoherent security fields. Supply an explicit implemented suite and key
for direct SRTP, or use an explicitly unsecured configuration.

These DTLS configuration helpers now return `Result<_, SecurityError>` so an
unsupported profile cannot be converted into an implemented one:

- `api::client::security::dtls::connection::create_dtls_config`
- `api::server::security::util::connection_config_to_dtls_config`
- `api::server::security::client::DefaultClientSecurityContext::new`

Use `?` (or match `SecurityError::UnsupportedFeature`) at each call site.

## SRTP profiles and conversion helpers

AES-GCM retains its public profile constants and now has distinct
`SrtpEncryptionAlgorithm::{AeadAes128Gcm, AeadAes256Gcm}` identities. Both are
unavailable in 0.3.5: validation, conversion, advertisement, construction, and
negotiation return `UnsupportedFeature`.

The following profile helpers now return `Result`:

- client `profile_to_suite`;
- server `convert_profile`, `convert_profiles`, `profile_id_to_suite`, and
  `profile_to_string`;
- server utility `convert_srtp_profiles`, `srtp_profile_to_string`,
  `get_crypto_suite_strings`, `create_security_info`, and
  `string_to_security_mode`.

Replace an infallible conversion such as:

```rust
let suite = profile_to_suite(profile);
```

with explicit propagation or handling:

```rust
let suite = profile_to_suite(profile)?;
```

`SecurityError` consequently has a new `UnsupportedFeature(String)` variant.
Downstream exhaustive matches must add that case.

Direct pre-shared-key SRTP now requires exactly one implemented profile. A
multi-profile PSK configuration is rejected rather than silently selecting a
different suite. `AES_CM_128_HMAC_SHA1_80` and
`AES_CM_128_HMAC_SHA1_32` are the implemented choices. AES-128 direct SRTP
requires exactly 30 bytes of material (16-byte key plus 14-byte salt); extra
bytes are no longer silently ignored.

At the low-level API, only the four exact reviewed AES-CM/HMAC-SHA1 built-ins
are constructible: 128-bit and 256-bit keys, each with an 80-bit or 32-bit
authentication tag. `SRTP_NULL_NULL` and `SRTP_NULL_SHA1_80` remain public
identity constants but `SrtpCryptoSuite::validate`, `SrtpCrypto::new`, and
`SrtpContext::new` return `UnsupportedFeature`. NULL/SHA1 is integrity-only and
does not provide confidentiality; it is deliberately unavailable rather than
being reported as a secure transport. AES-CM with NULL authentication and
other hand-built combinations are also rejected.

Every constructible suite requires an exact 14-byte master salt. Both shorter
and longer salts now fail construction instead of truncating or ignoring
bytes. `SrtpCryptoKey::from_base64` accepts only exact combined key-material
sizes: 30 bytes for AES-128 or 46 bytes for AES-256.

## SDES constructors

These constructors now return `Result<Self, SecurityError>`:

- `SdesClient::{new, from_security_config}`;
- `SdesServer::{new, from_security_config}`;
- `SdesServerSession::{new, from_security_config}`.

Add `?` or handle the validation error. This prevents unsupported profiles from
being retained for later SDP advertisement.

## MIKEY, ZRTP, key management, and SRTCP

All MIKEY modes and ZRTP public configuration/types are retained for source
compatibility, but validation, checked construction, factory selection,
advertisement, negotiation, initialization, and message processing return a
typed unsupported error before state or key mutation. MIKEY-PSK is included:
the previous path authenticated a message that carried TEK/salt in cleartext,
so it did not provide the key protection its name implied. `Mikey::new` and
`Zrtp::new` remain type-level compatibility constructors only.

The source-compatible MIKEY-PKE helpers `sign_certificate_with_ca` and
`validate_certificate_chain` now always return `Error::UnsupportedFeature`.
The former placeholder returned a self-signed subject certificate while
claiming CA signing, and the latter accepted a chain without checking issuer or
signature. Real key generation, self-signed certificate generation, and
certificate metadata extraction remain available, but they do not establish a
trusted MIKEY-PKE chain.

Built-in security policies and default, enterprise, peer-to-peer, and
development recovery configurations now name only SDES. They do not advertise
DTLS-SRTP, MIKEY, ZRTP, or an unprovisioned PSK fallback. Explicit custom
recovery configurations can still name those public enum values, but context
creation returns a typed unsupported or missing-key error and never fabricates
an all-zero key.

The advanced multi-stream key derivation/rotation API also fails closed in
0.3.5. Its former XOR-based placeholder was not a reviewed standard KDF.
`derive_stream_key`, stream setup, rotation, and `KeyManager::initialize`
return `UnsupportedFeature` before installing keys, incrementing generations,
inserting auto-configured sessions, or starting a background task.

When SRTP is installed or required, RTP transports no longer send or accept
plaintext RTCP. `SrtpContext::{protect_rtcp, unprotect_rtcp}` return
`Error::UnsupportedFeature` for enabled contexts until the per-SSRC SRTCP state
implementation lands. Applications must not fall back to plaintext RTCP.

`SrtpContext::set_key_rotation` retains its 0.3.4 signature. A non-`None`
schedule now causes the first required rotation to return
`Error::UnsupportedFeature` before encryption or packet-index mutation instead
of silently continuing with the expired key. Leave rotation set to `None` until
the stateful key-rotation implementation is available.

Built-in policy templates that require rotation, packet-lifetime enforcement,
or perfect-forward-secrecy enforcement now return `UnsupportedFeature` during
policy validation. They no longer claim enforcement that the media path does
not perform.

## Transport and manager behavior

`SecurityRtpTransport::set_srtp_context` and
`UdpRtpTransport::set_srtp_contexts` now return `Result`. Add `?` or explicitly
handle a rejected disabled/unapproved context. The UDP setter irreversibly
latches secure-media policy before validation, so ignoring an error cannot
re-enable plaintext RTP/RTCP; readiness remains false until valid contexts are
installed in both directions.

Once secure media is required, public raw RTP sends are rejected, public UDP
receives authenticate and decrypt before returning data, and plaintext or
wrong-key packets do not produce media events. A secure
`SecurityRtpTransport` owns socket receive processing; its direct
`receive_packet` method returns `UnsupportedFeature` to avoid racing the
authenticated event path. Subscribe to transport events for decrypted media.
Raw socket handles (`UdpRtpTransport::get_socket`, wrapper `inner_transport`,
and `RtpSession::get_socket_handle`) remain low-level interoperability escapes
that bypass transport security and must never carry protected media directly.

Standalone direct-PSK client/server security-context objects configure key
material but do not themselves own or install media crypto, so `is_secure()`
now remains false. High-level clients report secure only while connected and
an approved SRTP context is installed. `SecurityContextManager` exposes
`protect_rtp`/`unprotect_rtp` as the real media-crypto handoff; its legacy PSK
setup helpers fail closed because they do not install transport crypto.

Capability reporting is exact: pre-provisioned PSK is a supported media method
but has no signaling offer/answer, while the current unified SDES context can
create offers but is not an answerer. Unknown or non-UTF-8 signaling no longer
defaults silently to SDES.

## Compatibility audit

The 0.3.5 public API was compared with the crates.io 0.3.4 API. The two
accidental break classes found during the audit were removed: the public
`KeyExchangeConfig` variant shapes are unchanged, and
`DefaultClientSecurityContext` remains constructible with its 0.3.4 public
fields. The semver-check failures are the intentional fail-closed changes
described above: new exhaustive-enum variants and removal of the incomplete
low-level DTLS constructors/state machine. Newly fallible security setters and
helpers are source migrations even though this semver-check run did not flag
them. Exact tool output is recorded in `SEMVER_EXCEPTIONS_0.3.5.md`.
