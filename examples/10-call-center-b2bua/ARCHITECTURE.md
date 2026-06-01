# Call-center B2BUA — architecture

## Topology

```
                         ┌───────────────────────────────────┐
                         │      call-center B2BUA (:5070)     │
                         │                                    │
   customer  ──INVITE──▶ │  UnifiedCoordinator                │ ──INVITE──▶  agent alice (:5071)
   (:5080)               │   └─ SipB2bua::handle_inbound      │             agent bob   (:5072)
             ◀──200 OK── │        (accept ▸ originate ▸ bridge)│ ◀──200 OK──
             ◀═══════════╪══════════ RTP bridge ══════════════╪═══════════▶
                         └───────────────────────────────────┘
        customer leg                                        agent leg
   (B2BUA is UAS here)                                 (B2BUA is UAC here)
```

The B2BUA is a **full user agent on both legs**: it terminates the customer's
dialog (acting as UAS) and originates a separate dialog to the agent (acting as
UAC). The two RTP streams are bridged through `media-core`. Because the legs are
independent dialogs, the B2BUA can hide topology, rewrite headers, choose the
agent, and tear either side down independently.

## Call sequence

```
customer            call-center (B2BUA)              agent (round-robin pick)
   │  INVITE  ────────────▶│                                │
   │                       │  Event::IncomingCall           │
   │                       │  pick agent[next % N]          │
   │                       │  handle_inbound(from,id,agent) │
   │                       │  INVITE  ─────────────────────▶│
   │                       │  ◀──────────────────── 200 OK  │
   │  ◀──────────── 200 OK │  (bridge established)          │
   │  ══════════════ RTP ══╪══════════════ RTP ════════════▶│
   │  BYE     ────────────▶│  Event::CallEnded              │
   │                       │  (BridgeHandle dropped)        │
```

## Components

| Process | Binary | Role |
|---------|--------|------|
| Support line | `server` | B2BUA: listens, routes round-robin, bridges. `UnifiedCoordinator` + `SipB2bua`. |
| Agents | `agent` ×2 | Reactive `CallbackPeer`s that auto-answer the originated leg. |
| Customer | `customer` | `StreamPeer` that dials the support line. |

## Routing

The server keeps an `AtomicUsize` counter and selects `agents[n % agents.len()]`
per inbound call — the simplest possible fair distribution. Swap this for any
policy (least-busy, skills-based, presence-aware) by changing how `agent_uri` is
chosen before `handle_inbound`. For availability-aware routing, resolve agents
from a registrar via `server::contact_resolver` instead of a fixed list.

## Production notes

- **Teardown:** this demo holds the `BridgeHandle` for the call's lifetime. A
  production B2BUA watches `Event::CallEnded` / `Event::CallFailed` on **both**
  legs and drops the handle explicitly to release media promptly.
- **Header policy:** across trust boundaries, use the `SipHeaderView` /
  `SipRequestOptions` carry-through API to control which inbound headers cross to
  the agent leg (topology hiding is automatic for `Via`/`Call-ID`/`CSeq`).
- **Media:** PCMU/PCMA in beta. Same-codec legs take a fast-path bridge.
