# rvoip — universal real-time gateway facade

[![Crates.io](https://img.shields.io/crates/v/rvoip.svg)](https://crates.io/crates/rvoip)
[![Documentation](https://docs.rs/rvoip/badge.svg)](https://docs.rs/rvoip)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](../../LICENSE)

`rvoip` is the facade for the workspace's shared conversation model and
transport adapters. It always provides the transport-independent
`Orchestrator` and the `Conversation`/`Session`/`Connection`/`Stream`/
`Message`/`Participant` types, defaults to the SIP product, and lets
applications opt into WebRTC, UCTP, Vapi voice agents, client,
application-builder, and conversation-extension surfaces.

> **Unified `0.3.3` release.** The `sip` feature is the release-gated beta
> surface. Other facade features are available today as developer previews:
> they are implemented and published, but API-unstable or outside the SIP beta
> attestation. The vCon-only targeted delta reuses the immutable 0.3.2 SIP
> evidence as unchanged-subsystem background; it is not a new beta run.
> Breaking changes remain possible before `1.0`.

## Quick start

The default feature is `sip`:

```toml
[dependencies]
rvoip = "0.3.3"
```

The shared orchestrator is available with every feature combination:

```rust
use rvoip::{Config, Orchestrator};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let orchestrator = Orchestrator::new(Config::default());

    // Construct enabled transport adapters and register them with:
    // orchestrator.register(adapter)?;

    let mut events = orchestrator.subscribe_events();
    while let Ok(event) = events.recv().await {
        // Route, bridge, or handle the conversation event.
        drop(event);
    }
    Ok(())
}
```

For the fastest path to a SIP endpoint, use
[`rvoip-sip`](../sip/rvoip-sip) directly. Its `Endpoint`, `StreamPeer`,
`CallbackPeer`, and `UnifiedCoordinator` APIs are also re-exported through
`rvoip::sip` when the default `sip` feature is enabled.

## Cargo features

This table mirrors `crates/rvoip/Cargo.toml`.

| Feature | Default | Maturity | Enables |
| --- | :---: | --- | --- |
| `sip` | ✅ | **Beta-qualified** | SIP application and interop surface under `rvoip::sip` |
| `g729` |  | Developer preview | End-to-end G.729A/G.729AB media, SDP, RTP, and transcoding support; implies `sip` |
| `webrtc` |  | Developer preview | WebRTC interop adapter under `rvoip::webrtc` |
| `uctp` |  | Developer preview | UCTP protocol plus QUIC, WebTransport, and WebSocket adapters under `rvoip::uctp` |
| `vapi` |  | Developer preview | Vapi bidirectional WebSocket agent adapter under `rvoip::vapi` |
| `sip-stir-shaken` |  | Developer preview | STIR/SHAKEN signing/verification under `rvoip::stir_shaken`; implies `sip` |
| `voip-3` |  | Developer preview | `sip` + `webrtc` + `uctp` + vCon + identity + AI harness |
| `client` |  | Developer preview | Cross-transport SDK under `rvoip::client` |
| `app` |  | Developer preview | High-level SIP/WebRTC/UCTP gateway builder under `rvoip::app` |
| `full` |  | Developer preview | `voip-3` + `vapi` + `sip-stir-shaken` + `client` + `app` |

Examples:

```toml
# Shared conversation model plus SIP, WebRTC, UCTP, vCon, identity, and AI.
rvoip = { version = "0.3.3", features = ["voip-3"] }

# High-level cross-transport application builder.
rvoip = { version = "0.3.3", features = ["app"] }

# Every facade-owned feature.
rvoip = { version = "0.3.3", features = ["full"] }
```

`full` means every **facade feature**, not every crate in the rvoip workspace.

## Module layout

| Module/path | Required feature | Source product |
| --- | --- | --- |
| `rvoip::{Orchestrator, Config}` | Always | [`rvoip-core`](../foundation/rvoip-core) |
| `rvoip::core_traits` | Always | [`rvoip-core-traits`](../foundation/rvoip-core-traits) |
| `rvoip::sip` | `sip` | [`rvoip-sip`](../sip/rvoip-sip) |
| `rvoip::stir_shaken` | `sip-stir-shaken` | [`rvoip-stir-shaken`](../extensions/rvoip-stir-shaken) |
| `rvoip::webrtc` | `webrtc` | [`rvoip-webrtc`](../webrtc/rvoip-webrtc) |
| `rvoip::uctp::{protocol, quic, webtransport, websocket}` | `uctp` | UCTP and substrate crates |
| `rvoip::vapi` | `vapi` | [`rvoip-vapi`](../extensions/rvoip-vapi) |
| `rvoip::{vcon, identity, harness}` | `voip-3` | Conversation-model extension crates |
| `rvoip::client` | `client` | [`rvoip-client`](../rvoip-client) |
| `rvoip::app` | `app` | Facade-owned application/gateway layer |

## Extension routing

Some extensions are re-exported by facade features; deployment-specific
providers stay separate so protocol applications do not pull in backends they
do not use.

| Need | How to enable | Crates |
| --- | --- | --- |
| vCon, identity surface, and AI-provider harness | `rvoip` feature `voip-3` | `rvoip-vcon`, `rvoip-identity`, `rvoip-harness` |
| Vapi-hosted voice agents over SIP or WebRTC | `rvoip` feature `vapi` | `rvoip-vapi` |
| STIR/SHAKEN | `rvoip` feature `sip-stir-shaken` | `rvoip-stir-shaken` |
| OIDC or Keycloak | Add directly | `rvoip-oidc`, `rvoip-keycloak` |
| LDAP, Redis, or IMS AKA authentication | Add directly | `rvoip-ldap`, `rvoip-redis`, `rvoip-ims-aka` |
| SAML, SCIM, or WebAuthn user lifecycle | Add directly | `rvoip-saml`, `rvoip-scim`, `rvoip-webauthn` |
| Redacted audit and SIEM exports | Add directly | `rvoip-audit` |
| Postgres vCon storage | Add directly | `rvoip-vcon-postgres` |

For example:

```toml
[dependencies]
rvoip = { version = "0.3.3", features = ["sip"] }
rvoip-keycloak = "0.3.3"
rvoip-redis = "0.3.3"
rvoip-audit = "0.3.3"
```

## Specialized workspace products

These products ship in the unified `0.3.3` train but are intentionally not
facade feature flags:

| Product | Crate | Why it stays separate |
| --- | --- | --- |
| Media over QUIC | [`rvoip-moq`](../moq/rvoip-moq) | MOQT transport/relay and broadcast deployments have their own runtime and security configuration |
| Amazon Connect | [`rvoip-amazon-connect`](../webrtc/rvoip-amazon-connect) | Optional AWS control-plane dependencies and Amazon Chime media |
| OS audio devices | [`rvoip-audio-device`](../media/rvoip-audio-device) | Optional CPAL/native device dependencies |
| Enterprise identity providers | [`crates/extensions`](../extensions) | Applications select only the identity, provisioning, and audit backends they operate |

## High-level gateway builder

Enable `app` to declare transports, roles, assignment, and callbacks through
one builder:

```toml
rvoip = { version = "0.3.3", features = ["app"] }
```

```rust,no_run
use rvoip::app::*;

# async fn run() -> rvoip::app::AppResult<()> {
let app = RvoipApp::builder()
    .webrtc(
        WebRtcConfig::ws("127.0.0.1:8081")
            .allow(Role::Customer, [Capability::Text, Capability::Voice]),
    )
    .sip(
        SipConfig::bind("127.0.0.1:5060")
            .domain("callcenter.local")
            .allow(Role::Employee, [Capability::Voice])
            .registrar_users([("alice", "password123")]),
    )
    .employees(EmployeePolicy::named(["alice"]))
    .customers(CustomerPolicy::webrtc_only())
    .assignment(AssignmentPolicy::fixed("alice"))
    .on_message(|ctx, msg| async move {
        ctx.reply("Alice", format!("Alice received: {}", msg.text))
            .await
    })
    .build()
    .await?;

app.run().await
# }
```

See the repository's
[`12-customer-escalation-sip-webrtc`](../../examples/12-customer-escalation-sip-webrtc)
example for a complete cross-transport application.

## Maturity boundaries

- SIP beta evidence does not qualify WebRTC, UCTP, MoQ, Amazon Connect, or
  extension products.
- WebRTC includes ICE and DTLS-SRTP; that does not make them SIP beta
  capabilities.
- Published developer-preview crates are available to build with now, but do
  not carry a blanket production-readiness or API-compatibility guarantee.
- Product READMEs define exact supported scope, configuration, and non-claims.

## Documentation

- [Workspace overview and complete extension catalog](../../README.md)
- [Facade API](https://docs.rs/rvoip)
- [SIP API](https://docs.rs/rvoip-sip)
- [SIP beta evidence](../sip/rvoip-sip/docs/)
- [Architecture and protocol design](../../docs/)

## License

Licensed under the [MIT License](../../LICENSE).
