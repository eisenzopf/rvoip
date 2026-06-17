# rvoip-amazon-connect

Amazon Connect **interop adapter** for `rvoip-core`. Delivers a call arriving
over any rvoip transport (SIP, WebRTC, UCTP/QUIC) to a live **Amazon Connect**
agent, carrying SIP-header-derived data as Connect **contact attributes** — the
channel that drives an agent **screen pop**.

The motivating flow: a PSTN call enters a **Vapi** application, which does a
**blind transfer** (SIP `INVITE`) to an rvoip server with custom headers
(`X-Vapi-Customer-Id`, `X-Account-Tier`, …). rvoip reads those headers,
translates them to Connect contact attributes, places a WebRTC contact into
Connect, and bridges the inbound audio to the agent.

```text
 Vapi (PSTN) ──blind-transfer INVITE + X-headers──▶ rvoip-sip (IncomingCall)
        │  read custom headers (SipHeaderView)  ─▶  AttributeMapping.translate(..)
        ▼
 AmazonConnectAdapter::originate_contact(attributes, display_name, ..)
        │ 1. StartWebRTCContact (aws-sdk-connect)  ─▶ ConnectionData (Chime meeting+attendee)
        │ 2. Chime signaling: JOIN→JOIN_ACK(TURN)→SUBSCRIBE(offer)→SUBSCRIBE_ACK(answer)
        │ 3. webrtc-rs peer connection (Opus) reaches Connected
        ▼
 Orchestrator::bridge_connections(sip_conn, connect_conn)  ── audio bridged both ways
        ▼
 Connect contact flow runs with our Attributes ─▶ agent answers in CCP ─▶ screen pop
```

## Two planes

| Plane | Module | What it does |
|---|---|---|
| Control | [`control`](src/control.rs) | `StartWebRTCContact` via `aws-sdk-connect`. Attributes (≤32 KB) become contact attributes — the screen-pop channel. Returns the Chime meeting + attendee `ConnectionData`. |
| Media | [`signaling`](src/signaling/chime.rs) | Joins the Chime meeting over the proprietary **protobuf-over-secure-WebSocket** protocol (vendored [`SignalingProtocol.proto`](proto/SignalingProtocol.proto)) and drives a `webrtc-rs` peer connection, reusing `rvoip-webrtc`'s peer/media plane. |

The control plane is abstracted behind the `ConnectContactStarter` trait, so the
crate and its unit tests build with **zero AWS dependencies**. The real
`aws-sdk-connect` implementation (`AwsConnectStarter`) is behind the
`aws-control` feature — this also isolates the AWS `aws-lc-rs` crypto provider so
it never clashes with the workspace's `ring` rustls provider unless opted in.

## Features

- `aws-control` — pull in `aws-config`/`aws-sdk-connect` and `AwsConnectStarter`.
- `aws-live` — enable the live end-to-end test (needs a real Connect instance).
- `server` — the **batteries-included** `ConnectScreenPopServer` (SIP UAS +
  media bridge). Pulls the SIP stack + media-core transcoder; off by default for
  users who only want the adapter.

## Batteries-included server (recommended)

`ConnectScreenPopServer` is the turnkey path: give it **one config object** and
it stands up a SIP UAS, reads custom headers off each inbound INVITE, translates
them to Connect attributes, places the WebRTC contact, and bridges the audio
(G.711 ⟷ Opus, transcoding) — no orchestrator wiring required.

```rust,no_run
use std::sync::Arc;
use rvoip_amazon_connect::{
    AttributeMapping, ConnectConfig, ConnectScreenPopServer, ScreenPopServerConfig, SipConfig,
};
# #[cfg(feature = "aws-control")]
use rvoip_amazon_connect::AwsConnectStarter;

# async fn run() -> rvoip_amazon_connect::Result<()> {
# #[cfg(feature = "aws-control")] {
let connect = ConnectConfig::new("INSTANCE_ID", "CONTACT_FLOW_ID")
    .with_region("us-west-2")
    .with_attribute_mapping(
        AttributeMapping::default().rename("X-Vapi-Customer-Id", "customerId"),
    );

let sip = SipConfig::local("connect-bridge", 5060);
let starter = Arc::new(AwsConnectStarter::from_env(Some("us-west-2".into())).await);

let server = ConnectScreenPopServer::build(
    ScreenPopServerConfig::new(sip, connect, starter),
).await?;

server.serve().await?; // listens for INVITEs; bridges each to Amazon Connect
# }
# Ok(())
# }
```

Enable it with `features = ["server"]` (add `"aws-control"` for the real AWS
control plane). See [`examples/13-sip-to-amazon-connect`](../../../examples/13-sip-to-amazon-connect)
for a runnable demo (offline mock by default, `--features aws-live` for AWS).

## Lower-level usage (adapter only)

```rust,no_run
use std::sync::Arc;
use rvoip_amazon_connect::{AmazonConnectAdapter, AttributeMapping, ConnectConfig};
# #[cfg(feature = "aws-control")]
use rvoip_amazon_connect::control::AwsConnectStarter;

# async fn run() -> rvoip_amazon_connect::Result<()> {
# #[cfg(feature = "aws-control")] {
let config = ConnectConfig::new("INSTANCE_ID", "CONTACT_FLOW_ID")
    .with_region("us-west-2")
    .with_attribute_mapping(
        AttributeMapping::default().rename("X-Vapi-Customer-Id", "customerId"),
    );

let starter = Arc::new(AwsConnectStarter::from_env(config.region.clone()).await);
let adapter = AmazonConnectAdapter::new(config, starter);

// `attributes` is produced from inbound SIP headers (see below).
let connect_conn = adapter
    .originate_contact(attributes, Some("Jane Caller".into()), None)
    .await?;
// orchestrator.bridge_connections(sip_conn, connect_conn).await?;
# }
# Ok(())
# }
```

### Reading SIP custom headers on an inbound call

`originate_contact` takes a plain attribute map; produce it from the inbound
INVITE using rvoip-sip's `SipHeaderView`:

```rust,ignore
use rvoip_sip::{CallHandler, CallHandlerDecision, IncomingCall};
use rvoip_sip::api::headers::SipHeaderView;
use rvoip_sip_core::types::headers::HeaderName;

#[async_trait::async_trait]
impl CallHandler for ScreenPopApp {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        // Collect every custom header off the INVITE (the `headers` map is the
        // simplest source; `call.header_str(&HeaderName::Other(..))` reads a
        // specific one with full typing).
        let headers: Vec<(String, String)> = call.headers.iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let mapped = self.mapping.translate(headers);

        let connect_conn = self.adapter
            .originate_contact(mapped.attributes, Some(call.from.clone()), None)
            .await
            .expect("start connect contact");

        // Accept the SIP leg and bridge it to the Connect leg.
        self.orchestrator.bridge_connections(self.sip_conn(&call), connect_conn).await.ok();
        CallHandlerDecision::Accept
    }
}
```

For `REFER`-driven blind transfers, the `Event::ReferReceived { request, .. }`
event carries the full `IncomingRequest` (also `SipHeaderView`), so the same
translation applies to headers on the REFER itself.

## Screen pop

The `attributes` you pass become standard Connect contact attributes. Reference
them in the contact flow (e.g. a *Set contact attributes* / *Check contact
attributes* block) and read them in the agent desktop via the Amazon Connect
Streams API `contact.getAttributes()` to drive the pop. No additional rvoip code
is involved on the AWS side — configuring the flow + CCP is an AWS console task.

## Live-validation checklist

The wire format (one protobuf `SdkSignalFrame` per binary WebSocket message) is
stable, but two pieces are reconstructed from the public JS SDK and should be
confirmed against a real instance (run with `--features aws-live`):

1. The signaling-URL query string in `signaling::chime::build_signaling_url`
   (join token carried as `sessionToken`).
2. The optional `SdkJoinFrame` fields sent in the JOIN.

Both are localized to `src/signaling/chime.rs`.

## Limitations (v1)

- Audio only (Opus / G.711). Video & screen-share are out of scope.
- One agent per contact; no WHEP-style multi-party fan-out.
- Inbound only (deliver *into* Connect); Connect→other-transport origination is
  future work.
