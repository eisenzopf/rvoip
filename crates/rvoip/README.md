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

> **Unified `0.3.8` release train.** The `sip` feature is the release-gated beta
> surface. Other facade features are available today as developer previews:
> they are implemented and published, but API-unstable or outside the SIP beta
> attestation. Publication requires fresh strict full-beta evidence bound to
> the exact clean `0.3.8` release source; historical exception and
> carry-forward reports do not qualify this train.
> Breaking changes remain possible before `1.0`.

## Quick start

The default feature is `sip`:

```toml
[dependencies]
rvoip = "0.3.8"
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
| `amr-nb` |  | Developer preview | End-to-end AMR narrowband media with RFC 4867 framing, DTX, and CMR; implies `sip` |
| `amr-wb` |  | Developer preview | The same for AMR wideband (G.722.2) at 16 kHz; implies `sip` |
| `amr` |  | Developer preview | Both AMR variants |
| `dtls-srtp` |  | Developer preview | SIP DTLS-SRTP keying with authenticated SDP fingerprints; implies `sip` |
| `opus` |  | Developer preview | End-to-end Opus media; requires libopus on the build host; implies `sip` |
| `all-codecs` |  | Developer preview | `g729` + `amr` + `opus` |
| `webrtc` |  | Developer preview | WebRTC interop adapter under `rvoip::webrtc` |
| `uctp` |  | Developer preview | UCTP protocol plus QUIC, WebTransport, and WebSocket adapters under `rvoip::uctp` |
| `vapi` |  | Developer preview | Vapi bidirectional WebSocket agent adapter under `rvoip::vapi` |
| `sip-stir-shaken` |  | Developer preview | STIR/SHAKEN signing/verification under `rvoip::stir_shaken`; implies `sip` |
| `voip-3` |  | Developer preview | `sip` + `webrtc` + `uctp` + vCon + identity + AI harness |
| `client` |  | Developer preview | Cross-transport SDK under `rvoip::client` |
| `app` |  | Developer preview | High-level SIP/WebRTC/UCTP gateway builder under `rvoip::app` |
| `full` |  | Developer preview | `voip-3` + `vapi` + `sip-stir-shaken` + `client` + `app` + `dtls-srtp` + `g729` + `amr`. Excludes `opus`, which needs libopus on the build host — use `all-codecs` for that |

### Deployment bundles

The additive `bundle-*` features group those leaf features into recognizable
deployment shapes: SIP endpoint, carrier SIP, browser gateway, AI conversation
gateway, full pure-Rust, and full with native codecs. Every bundle is tested
independently with default features disabled, and CI verifies that the
pure-Rust bundle does not resolve the native `opus` crate.

See the machine-checked [feature bundle matrix](../../docs/FEATURE_BUNDLES.md)
for exact membership, codecs, system dependencies, maturity, and Cargo
examples. Advanced users can continue composing the leaf features above.

Examples:

```toml
# Shared conversation model plus SIP, WebRTC, UCTP, vCon, identity, and AI.
rvoip = { version = "0.3.8", features = ["voip-3"] }

# High-level cross-transport application builder.
rvoip = { version = "0.3.8", features = ["app"] }

# Every pure-Rust facade feature and codec.
rvoip = { version = "0.3.8", default-features = false, features = ["bundle-full-pure-rust"] }
```

The high-level SIP listener exposes the same fail-closed signalling, media,
and codec posture as the lower SIP runtime. Certificates and keys stay in
operator-managed files; configuration and debug output never contain their
contents:

```rust
use rvoip::app::{SipConfig, SipMediaSecurity};

let sip = SipConfig::bind("0.0.0.0:5060")
    .advertised_addr("203.0.113.10:5060".parse()?)
    .tls_listener(
        "0.0.0.0:5061".parse()?,
        "/run/secrets/sip-cert.pem",
        "/run/secrets/sip-key.pem",
    )
    .tls_advertised_addr("203.0.113.10:5061".parse()?)
    .tls_extra_ca("/run/secrets/carrier-ca.pem")
    .media_security(SipMediaSecurity::Required)
    .offered_codecs([0, 8, 101]);
# Ok::<(), Box<dyn std::error::Error>>(())
```

SIP-TLS listener startup refuses an incomplete or unreadable identity, strict
SRTP cannot be enabled without offering SRTP, and codec offers reject empty,
duplicate, or unavailable payload types. UDP, TCP, and WebSocket listener
enablement remains controlled by the lower `rvoip-sip::Config` surface.

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
rvoip = { version = "0.3.8", features = ["sip"] }
rvoip-keycloak = "0.3.8"
rvoip-redis = "0.3.8"
rvoip-audit = "0.3.8"
```

## Specialized workspace products

These products ship in the unified `0.3.8` train but are intentionally not
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
rvoip = { version = "0.3.8", features = ["app"] }
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
- SIP DTLS-SRTP is a separate feature-gated path from WebRTC's DTLS transport;
  neither broadens the maturity claim of the other surface.
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
