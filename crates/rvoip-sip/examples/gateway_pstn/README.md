# PSTN gateway walkthrough

**Pattern mapping:** SIP_API_DESIGN_2 §11.4 (two-leg gateway / B2BUA),
§3.3 INVITE builder, `coord.events_for_session(..)` (per-leg filtering).

## What it demonstrates

| Step | API |
|---|---|
| Trunk-facing INVITE arrival | `coord.events()` → `Event::IncomingCall { call_id, from, .. }` |
| Per-leg event filtering | `coord.events_for_session(&inbound_id)` |
| Outbound IP-side INVITE | `coord.invite(from, target).send().await` |
| Wait for callee answer | poll `Event::CallAnswered` on the outbound leg |
| Accept inbound after B-leg lands | `coord.accept_call(&inbound_id).await` |
| Reach `Active` on both legs | `coord.get_state(&sid)` polling |
| Teardown | `coord.hangup(&sid)` on each leg |

## Run

```
cargo run --example gateway_pstn
```

The example boots four coordinators in-process — `pstn-trunk` (trunk
caller), `gateway-trunk` (trunk-facing terminator), `gateway-ip`
(IP-facing originator), `ip-callee` (CallbackPeer answering).

## Note on bridging

`coord.bridge(a, b)` requires both legs to live on the *same* coord.
Production gateways typically use one coord listening on both
interfaces. This example uses separate trunk-facing and IP-facing
coords to show the trust-boundary shape; media bridging across coords
is a follow-on (`BridgeAcrossCoordinators` API). The lifecycle —
inbound INVITE event → outbound INVITE → wait for answer → accept
inbound → teardown both — is the load-bearing pattern.

## Adding carry-through

To preserve `History-Info` / `Diversion` / `P-Asserted-Identity` from
the trunk side, the gateway's `IncomingCall` (from the
`Event::IncomingCall` callback shape; this example reads the event
stream variant for brevity) can be plumbed through to the outbound
builder with `coord.invite(..).with_headers_from(&incoming_call,
&names)?`. See `examples/sbc_topology_hiding/` for that pattern.

## Adding TLS on the IP side

Replace `Config::local("gateway-ip", GATEWAY_IP_PORT)` with a TLS
variant. Outbound INVITEs and inbound 200 OKs flow through the same
state machine; the transport switch is transparent.
