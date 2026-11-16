# rvoip-webex-byova

Webex Contact Center BYOVA (Bring Your Own Virtual Agent) adapter for rvoip.
Webex calls a virtual agent over its WebSocket protocol (`/v1/va`) or, with
the `grpc` feature, its gRPC protocol (`ProcessCallerInput`). This crate turns
each Webex conversation into an rvoip `Connection` on `Transport::WebexByova`,
so admission, lifecycle, media and teardown go through the `Orchestrator` like
any other transport. Bridge it to an agent connection, such as
[`rvoip-vapi`](../rvoip-vapi) in ordered mode, and the agent's audio reaches
Webex through the core media graph.

Status: **experimental**. Simulator and loopback tests pass; no live Cisco
interoperability is claimed for this adapter. The default build (WebSocket
transport, adapter, authentication) needs nothing beyond Cargo. Cisco's
protobuf contract is not redistributed with the crate: see
[Building the gRPC transport](#building-the-grpc-transport).

## How a conversation flows

1. The host terminates TLS (Webex requires WSS) and hands each stream to
   [`WebexByovaAdapter::accept_websocket`], or serves
   `WebexByovaGrpcService` (`grpc` feature) behind TLS.
2. The upgrade, or every gRPC call, is authenticated before anything is read:
   the `Authorization` bearer token must pass the configured
   [`WebexAuthenticator`]. There is no unauthenticated mode.
3. The first application message must be a valid SESSION_START from the
   authenticated organization for the configured virtual agent.
4. The adapter mints a local `ConnectionId` (Webex's conversation ID is kept
   only for correlation) and publishes the connection together with its
   principal as one atomic `AuthenticatedInboundConnection` event. The
   principal's tenant is the Webex organization. With an inbound admission
   gate installed, the policy sees the ticket first; a rejection, or
   `notify_inbound_admission_outcome(false)`, ends the conversation.
5. Nothing is written to Webex until core accepts the connection
   (`InboundAction::Accept`), which places it in the host's session. The host
   receives [`WebexInput::Started`] and bridges the connection to its agent.
6. Caller audio flows into the media graph; agent audio from the graph is
   written to Webex as it arrives. Webex paces playback, so nothing is paced
   here. Digits, NO_INPUT, custom events and SESSION_END arrive on
   [`WebexByovaCall::next_input`].
7. A terminal reply (`end_session`, `transfer_to_agent`,
   `complete_with_metadata`), Webex's SESSION_END, `close`, core teardown,
   the session ceiling or a protocol failure ends the conversation; the
   adapter reports `Ended` or `Failed` to core and releases its slot.

## Ordered media and reply boundaries

Webex needs each reply's audio in order, followed by exactly one FINAL, and
no stale audio after a barge-in. The adapter's media stream asks bridges for
lossless delivery (`MediaStream::lossless_delivery_stall_timeout`), so the core
media graph applies backpressure instead of dropping frames, and a consumer
that stalls past [`WebexByovaConfig::media_stall_timeout`] fails the route.

Reply boundaries are positions in the agent audio: the number of PCMU bytes
this connection has received from its media path. Mark them in either of two
ways:

- **In band (exact).** [`WebexByovaCall::follow_reply_marks`] hands the
  session task a stream of [`ReplyMark`]s. Before writing each agent frame the
  task takes every mark already published and applies it at its exact byte,
  even when later audio is already queued. `rvoip-vapi`'s ordered mode
  publishes each `VapiOrderedEvent` (with its `audio_offset`) before the audio
  that follows it, so mapping those events to marks gives exact boundaries:

  ```text
  assistant speech-update "stopped" at offset N  ->  ReplyMark { position: N, action: Complete }
  user-interrupted at offset N                   ->  ReplyMark { position: N, action: Interrupt }
  ```

- **By command.** [`WebexByovaCall::complete_reply`] and
  [`WebexByovaCall::interrupt`] take a [`ReplyBoundary`]. A completion waits
  until audio reaches its position (bounded by
  [`WebexByovaConfig::reply_boundary_timeout`]); an interruption discards
  audio before its position that has not been written yet. A command that
  arrives after later audio was already written cannot move that audio out of
  the current reply, so prefer marks when the agent can publish them.

Positions line up only when the bridge does not transcode: use PCMU/8 kHz on
the agent side (`VapiAudioFormat::MuLaw8Khz` for Vapi). Agent frames in any
other payload type fail the call.

Vapi's speech events remain heuristics, not authoritative turn or audio
completion barriers; see the implementation specification's risk register.

## Authentication

[`WebexJwsValidator`] verifies Cisco's BYODS JWS against a key set the host
supplies and refreshes with `replace_keys` (it never fetches a URL found in a
token). It checks the signature by `kid` with the allowed algorithms (RS256
by default), the allowlisted issuer, audience, optional subject, `exp`,
`nbf`, `iat`, a maximum lifetime of 24 hours, the registration nonce in `jti`
(constant-time, reusable across calls), and the organization, data source URL
and schema ID for the transport that received the token. Keys older than
`keys_max_age` (one hour) fail closed.

Cisco does not publish the claim names for the organization, URL and schema.
The defaults (`com.cisco.user.orgid`, `com.cisco.datasource.url`,
`com.cisco.datasource.schema.uuid`) are assumptions; override
[`WebexClaimNames`] if a live token differs. A connection may outlive its
upgrade token; the protocol defines no mid-session reauthentication.

## Example

```no_run
use std::sync::Arc;
use rvoip_core::{config::Config, Orchestrator};
use rvoip_webex_byova::{
    WebexByovaAdapter, WebexByovaConfig, WebexJwsConfig, WebexJwsValidator, WebexRegistration,
    WebexVirtualAgent, WebexWebSocket,
};

async fn serve(jwks: jsonwebtoken::jwk::JwkSet) -> Result<(), Box<dyn std::error::Error>> {
    let auth = WebexJwsConfig::new(
        vec!["https://idbroker-b-us.webex.com/idb".into()],
        "registered-audience",
        "webex-org-id",
    )
    .with_websocket(WebexRegistration::websocket(
        "https://va.example.com/v1/va",
        "registration-nonce",
    ));
    let validator = Arc::new(WebexJwsValidator::new(auth, jwks)?);
    let adapter = WebexByovaAdapter::new(WebexByovaConfig::new(
        validator,
        WebexVirtualAgent::new("agent-1", "Support agent"),
    ))?;
    let core = Orchestrator::new(Config::default());
    core.register(adapter.clone())?;
    // In production each accepted stream is first wrapped in TLS.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8443").await?;
    loop {
        let (stream, _) = listener.accept().await?;
        let adapter = adapter.clone();
        tokio::spawn(async move {
            if let Ok(WebexWebSocket::Conversation(call)) = adapter.accept_websocket(stream).await {
                // Accept it through the Orchestrator when core publishes
                // `ConnectionInbound`, bridge it to the agent, then drive
                // `call` (inputs, reply marks, terminal replies).
                drop(call);
            }
        });
    }
}
```

## Configuration

| Field | Default | Purpose |
| --- | --- | --- |
| `authenticator` | required | Token verification; `WebexJwsValidator` or a host implementation |
| `virtual_agent` | required | The agent Webex selects; also returned by the list endpoint |
| `websocket_path` / `list_agents_path` | `/v1/va` / `/v1/listVirtualAgents` | Accepted upgrade paths; others get 404 |
| `audio_output` | `RawPcmu` | WebSocket output profile (`wav-compat` adds Cisco sandbox WAV profiles) |
| `grpc_reply_framing` | `OneSecondChunks` | gRPC CHUNK framing (`grpc` feature) |
| `max_connections` | 100 | Concurrent conversations |
| `queue_capacity` | 128 | Per-connection input, command, event and media queues |
| `max_frame_bytes` | 128 KiB | WebSocket message limit |
| `start_timeout` | 10 s | Transport open to SESSION_START |
| `admission_timeout` | 15 s | Publication to core acceptance |
| `write_timeout` | 5 s | One Webex write |
| `reply_boundary_timeout` | 5 s | Wait for audio to reach a reply boundary |
| `media_stall_timeout` | 5 s | Lossless media budget requested from bridges |
| `max_session_duration` | 2 h | Conversation ceiling |
| `max_pending_output_bytes` | 256 KiB | gRPC output held while no RPC is open |

## gRPC

With `features = ["grpc"]`, `WebexByovaAdapter::grpc_service` returns the
tonic service and a receiver of new conversations. Each RPC is authenticated
from its `authorization` metadata (with or without `Bearer `). The first RPC
must carry SESSION_START; later RPCs of the same conversation attach to the
same connection. Agent output goes to the oldest open RPC and its FINAL
completes that RPC normally. Meaningful input on a newer RPC supersedes older
ones; Cisco's 3-byte listening placeholder does not. Later audio frames that
omit identity and format inherit them from the same RPC's first request.
The `grpc` module keeps the protobuf types and reply framers (CHUNK, WAV
chunk and whole WAV) for hosts that need them directly.

## Building the gRPC transport

Cisco's `voicevirtualagent.proto` and `byova_common.proto` have no published
licence, so rvoip neither packages nor commits them. The `grpc` feature
compiles them at build time from the directory named by
`RVOIP_WEBEX_BYOVA_PROTO_DIR` (with a vendored `protoc`; no system `protoc`).

In the rvoip workspace (any `--all-features` build includes `grpc`), run

```sh
python3 scripts/ci/fetch_webex_byova_protos.py
```

once. It downloads both files from Cisco's repository at the pinned commit
into the ignored `target/third_party/cisco-webex-byova-proto/`, checks them
against `third_party/cisco-webex-byova-proto/SHA256SUMS`, and fails closed on
a mismatch; the build finds them there. CI runs it automatically before every
build that needs it.

A build from crates.io with `grpc` and no directory fails with a message that
says where to get the files:

```sh
mkdir -p webex-byova-proto && cd webex-byova-proto
base=https://raw.githubusercontent.com/webex/dataSourceSchemas/fca090dfa806e28695acb145e927ad0d557ce7fe/Services/VoiceVirtualAgent/5397013b-7920-4ffc-807c-e8a3e0a18f43/Proto
curl -fsSLO "$base/voicevirtualagent.proto"
curl -fsSLO "$base/byova_common.proto"
# Expected SHA-256 (the tested revision):
#   815514908ef0c4ec417359cf7085052cde9433b4c21c7aa5e3e44ff130aca57a  voicevirtualagent.proto
#   d65abfa0970047ab60af703395c7fda620b98f6ffb67d7822446c7f377f77bb3  byova_common.proto
sha256sum voicevirtualagent.proto byova_common.proto
export RVOIP_WEBEX_BYOVA_PROTO_DIR="$PWD"
```

The `rvoip` facade's `webex-byova` feature enables only the default,
proto-free surface.

## Protocol engine

`Session` (WebSocket) and the `grpc` framers remain usable without the
adapter. `Session` enforces identity, sequence and reply ordering; it never
infers endpointing from audio. See its module documentation for an example,
and the opt-in `wav-compat` profiles (`Pcm16Wav`, `Pcm16WavFastStart`,
`Pcm16WavWhole`) qualified only against Cisco's early-access sandbox.

## Not yet done

- Live Cisco interoperability of this adapter, load and soak testing.
- An authoritative provider turn contract: Vapi speech events are heuristics.
- BYODS registration renewal and JWKS fetching (host responsibilities).
- Digit collection policy, handoff tools and transcript policy (host
  application policy built on `next_input`, `report` and `transfer_to_agent`).

The design, schema discrepancies and open questions are in
`docs/WEBEX_BYOVA_IMPLEMENTATION_SPEC.md` at the workspace root.

## Tests

```sh
cargo test --locked -p rvoip-webex-byova
cargo test --locked -p rvoip-webex-byova --features wav-compat
cargo test --locked -p rvoip-webex-byova --all-features
cargo clippy --locked -p rvoip-webex-byova --all-features --all-targets -- -D warnings
```
