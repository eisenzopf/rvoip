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

### MediaGraph-backed virtual publishers

Use `Orchestrator::register_virtual_publisher` to expose an existing
Connection's audio under a canonical `(SessionId, StreamId)` without taking
the source receiver away from bridges, recorders, or other broadcasts. Pass a
`VirtualPublisherDescriptor` containing the Session, logical Stream, and
publisher Participant name. The returned `ManagedVirtualPublisher` owns a
bounded ten-frame sink, the fanout task, and an exact-generation publisher
registry row.

Retain that handle for the publication lifetime. `close().await` converges the
task and graph route; Drop performs immediate best-effort cancellation. Either
path unregisters only the row created by that handle, so a stale handle cannot
delete a newer registration that reused the same Session/Stream identity.
Duplicate live identities are rejected. Frames delivered to subscribers carry
the descriptor's canonical Stream ID even when the underlying SIP/WebRTC media
stream uses a different local ID.

## WebRTC dependency pin

rvoip currently patches `rtc` to exact fork revision
`1e5b7d4be6d94850694f2519f4c235d16c871d53`. It fixes DataChannels created
after the SCTP handshake and preserves received DCEP partial-reliability
metadata. A current-version port is kept on the `eisenzopf/rtc` fork for owner
review. It must not be submitted upstream without explicit approval.

## Signaling authentication and lifecycle

WebRTC signaling now authenticates WS/WSS before the HTTP 101 response and
uses the same issuer + tenant + subject ownership check for WHIP, WHEP, WS,
and WSS mutations. Outbound routes that will be exposed through authenticated
signaling must call `WebRtcAdapter::bind_authenticated_principal` first.
Route cleanup removes ownership and background ICE/keepalive tasks atomically.

SIP applications can construct `SipListenerAuthPolicy` with Digest/Bearer,
explicit trusted-CIDR principals, and/or verified mTLS fingerprint mappings,
then use `UnifiedCoordinator::new_with_listener_auth` or
`SipAdapter::from_config_with_listener_auth`. TLS/WSS listeners configure
`TlsServerClientAuthConfig::optional` or `required` with an explicit client CA.
The default remains disabled for compatibility. Enabled policies run at the
transaction boundary before dialog or application dispatch; ACK, CANCEL, and
retransmissions must match the accepted INVITE's transport binding.

UCTP production defaults now require command scopes, retain full principals,
bound replies and Connection resources to their authenticated owner, enforce
bounded replay/capacity state, and couple signaling/media tasks to one peer
supervisor. `UctpCoordinatorCaps::legacy_permissive` is only for trusted
development compatibility. QUIC, WebTransport, and WebSocket share the same
caps and configurable authentication deadline.

## UCTP 0.2 media and resource bindings

UCTP 0.2 removes the alpha invite-time synthetic audio Stream. A media-capable
peer must complete `connection.offer` and `connection.ready`; the adapter binds
each accepted wire `strm_id` to a concrete media Stream before the server emits
`stream.opened`. Applications must use the announced `stream_local_id` rather
than assuming the first Stream is ID `1`.

The media datagram is exactly an eight-byte UCTP header followed by one complete
RTP packet. New code must use `RtpDatagram`, `pack_rtp_datagram`, and
`unpack_rtp_datagram`. The hidden raw helpers exist only to compile alpha
callers and do not validate their opaque body; payload-only datagrams are not a
valid 0.2 media path.

Because the datagram header contains no Session or Connection field,
`stream_local_id` is allocated once across the entire physical QUIC or
WebTransport peer. IDs are nonzero, monotonically issued, and never reused
during that peer lifetime. Delayed datagrams therefore cannot land on a later
Stream. QUIC and WebTransport now share the rvoip `PeerMediaRouter` contract,
including authenticated owner checks, exact Session/Connection/Stream indexes,
bounded ingress, diagnostics, and cancellation-coupled cleanup.

Wire Session and Connection IDs are untrusted peer-local values. Deployments
that intentionally connect multiple peers to one call must configure a
`SessionBindingResolver` that validates the authenticated principal and maps
the wire Session to a canonical rvoip Session. Without one, the safe default
namespaces every peer independently. `PeerResourceBindings` then maps wire
Connections to real adapter Connections, rechecks expiry, prevents cross-
Session aliasing, and removes exact sibling resources during teardown.

`stream.subscribe` and `stream.unsubscribe` now operate through those canonical
bindings. Tests and applications should send the real wire commands rather
than calling `Orchestrator::add_subscription` as a protocol substitute.
Publisher cleanup is Connection-conditioned so a delayed close cannot remove
a same-named replacement Stream.

For an encrypted QUIC capture, explicitly opt in with
`enable_server_key_log_from_env` and `enable_client_key_log_from_env` after
setting `SSLKEYLOGFILE`, then supply the resulting key log to Wireshark or
tshark. rvoip never enables traffic-secret logging implicitly.
