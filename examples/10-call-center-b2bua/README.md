# 10 · Mini call-center (B2BUA bridge)

> **Beta status: Supported.** B2BUA bridging on `UnifiedCoordinator` +
> `server::b2bua` is supported. See
> [`COMPATIBILITY_MATRIX.md`](../../crates/sip/rvoip-sip/docs/COMPATIBILITY_MATRIX.md).

## Overview

The flagship scenario: a **back-to-back user agent (B2BUA)** that fronts a
support line, routes each inbound customer call to an available agent, and
bridges the two call legs through media-core. The server is a full UA on both
legs, so it controls routing, topology, and media — the foundation of a call
center, SBC, or carrier gateway.

Built on [`UnifiedCoordinator`] (raw call control) plus
[`server::b2bua::SipB2bua`], whose `handle_inbound` wires the canonical
**inbound → originate → bridge** flow in one call. Agents are chosen round-robin
to demonstrate routing. See [ARCHITECTURE.md](ARCHITECTURE.md) for the topology.

## Demo flow

1. Two **agents** (`:5071`, `:5072`) come up and wait for calls.
2. The **call-center** B2BUA (`:5070`) listens on the support line.
3. A **customer** (`:5080`) dials `sip:support@…:5070`.
4. The B2BUA accepts the customer leg, originates a leg to the next agent
   (round-robin), and bridges them. To the customer it's one ordinary call.

## Architecture (summary)

```
   customer ── INVITE ─▶ call-center B2BUA ── INVITE ─▶ agent (alice | bob)
   (:5080)               (:5070)                         (:5071 / :5072)
                          └────── RTP bridge ──────┘
```

## Quick start

```sh
./run_demo.sh
```

Or manually, in four terminals:

```sh
cargo run --bin agent -- --port 5071 --name alice
cargo run --bin agent -- --port 5072 --name bob
cargo run --bin server -- --bind 127.0.0.1:5070
cargo run --bin customer -- --port 5080
```

## Expected output

```text
  [call-center] support line on 127.0.0.1:5070 — 2 agents in the pool
  [call-center] customer … → …: routing to sip:agent@127.0.0.1:5071 (session=…)
  [call-center] ✅ bridged … ↔ sip:agent@127.0.0.1:5071
  [customer] ✅ connected to an agent via the call center
  [alice] ✅ connected to customer (…)
  [call-center] session … ended: BYE received

✅ DEMO SUCCESSFUL — customer bridged to an agent via the B2BUA
```

(Run a second customer and the next call routes to `bob` — round-robin.)

## Command-line options

| Binary | Flag | Default |
|--------|------|---------|
| `server` | `--bind` / `--from` | `127.0.0.1:5070` / `sip:b2bua@127.0.0.1:5070` |
| `agent` | `--port` / `--name` | `5071` / `agent` |
| `customer` | `--port` / `--support` / `--talk-secs` | `5080` / `sip:support@127.0.0.1:5070` / `2` |

## Beta scope notes

- B2BUA bridging is beta-supported; media stays **PCMU/PCMA**.
- The agent pool here is a fixed list. A production deployment resolves agents
  from a registrar (presence/availability) — see `server::contact_resolver`.
- Real B2BUAs watch both legs for `CallEnded` and tear down explicitly; this demo
  holds the `BridgeHandle` for the call's duration for clarity.

## Troubleshooting

- **`bridge failed`** — the chosen agent isn't reachable; ensure both agents are
  up before the customer calls (`run_demo.sh` waits on their ports).
- **Port conflicts** — change `--bind` / agent `--port` / customer `--port`.

## Next steps

- [05-blind-transfer](../05-blind-transfer/) / [06-attended-transfer](../06-attended-transfer/) — agent-initiated transfers.
- In-crate references: `sip_b2bua`, `unified/04_b2bua_bridge`, `call_center_agent`.

[`UnifiedCoordinator`]: https://docs.rs/rvoip-sip/latest/rvoip_sip/struct.UnifiedCoordinator.html
