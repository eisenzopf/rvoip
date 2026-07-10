# Bridgefu foundation API migration

This note covers the reusable rvoip foundation changes developed for
Bridgefu 1.0. They are pre-release APIs until the coordinated rvoip release
candidate is cut.

## Authentication principals

`AuthenticatedPrincipal`, `AuthenticationMethod`, and
`PrincipalOwnershipKey` now live in `rvoip-core-traits`. Existing imports from
`rvoip_auth_core` and `rvoip_core::identity` remain re-exported.

New authorization code should retain the complete principal and compare
`principal.ownership_key()`. Comparing only `subject` is unsafe because
different issuers or tenants may reuse the same subject. Public event matches
must include a fallback arm because authentication and transport event enums
are designed to grow additively.

This foundation release is an intentional pre-1.0 semver transition: the
public adapter, orchestrator, UCTP-session, client-inbound, and cross-crate
event enums are now `non_exhaustive`. Existing variants remain, but exhaustive
downstream matches must add a fallback arm. `WebRtcMetrics` and the operational
WebRTC `Route` view are also non-exhaustive; construct metrics with `Default`
and mutate named public fields instead of using a struct literal.

The rich principal event is emitted in addition to the legacy assurance event.
Rich identity remains on the typed in-process event and connection route. The
global cross-crate bus is not tenant-authorized, so it receives only a redacted
authentication lifecycle marker; subject, participant, tenant, scopes, issuer,
expiry, credentials, tokens, proofs, JWKs, and fingerprint values are not
exported there.

## Data messages

Use `DataMessage` and `DataReliability` for application data instead of
encoding metadata into a transport-specific text message. Validate messages
before storage or transport. The shared contract limits labels to 128 bytes,
content types to 255 bytes, IDs to 128 bytes, and bodies to 64 KiB.

WebRTC uses the public `rvoip.data.v1` framing contract and supports arbitrary
labels, text or binary bodies, message IDs, and RFC 8832 reliability settings.
Its current dependency limits the complete encoded frame to 16 KiB. UCTP maps
to `message.send`; this path currently supports reliable ordered delivery and
returns an explicit capability error for other policies.

The public `Route` value is an operational read view, not a construction API.
Its DataChannel cache keys now combine exact label and reliability policy;
callers must not parse those private cache keys and should use adapter
DataMessage methods instead.

Legacy WebRTC text messages remain accepted. Only the reserved
`rvoip-chat` and `rvoip-messages` labels project into the legacy rvoip message
store; arbitrary DataMessages are delivered exactly once on the typed event
surface. The global cross-crate form omits body bytes, message ID, label, and
content type, retaining only aggregate-safe size/reliability diagnostics and
the server-generated opaque connection ID.

## Media graph

`MediaGraph` is now the owner of each built-in stream's single-consumer inbound
receiver. Call peers, recorders, listeners, and broadcast publishers attach as
bounded sinks instead of independently calling `frames_in()`.

`bridge_connections` remains the compatibility entry point. New lifecycle-
sensitive code should retain `ManagedMediaRoute`, await `wait_active()`, and
use acknowledged `remove()` or `remove_sink_and_wait()` during teardown. A
cloned `MediaGraphRouteStatus` observes state but does not keep the route alive.

The legacy `MediaStream::frames_in()` and graph route-ID methods remain, but
new built-in integrations should use `try_frames_in()` and managed routes so
duplicate acquisition and teardown failure are explicit.

## WebRTC dependency pin

rvoip currently patches `rtc` to exact fork revision
`1e5b7d4be6d94850694f2519f4c235d16c871d53`. It fixes DataChannels created
after the SCTP handshake and preserves received DCEP partial-reliability
metadata. A current-version port is kept on the `eisenzopf/rtc` fork for owner
review. It must not be submitted upstream without explicit approval.
