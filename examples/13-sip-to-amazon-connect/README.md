# Example 13 — SIP → Amazon Connect (batteries-included screen pop)

Boots `rvoip_amazon_connect::ConnectScreenPopServer` from a **single config
object**. It stands up a SIP UAS; every inbound INVITE (as a Vapi blind transfer
would send) has its custom headers translated into Amazon Connect **contact
attributes** (the screen-pop channel), a WebRTC contact is placed into Connect
via `StartWebRTCContact`, and the audio is bridged (G.711 ⟷ Opus). No
orchestrator wiring required.

## About your hosted widget + JWT

The Amazon Connect **communications widget** script and its **JWT** are
**browser-only** and are **not used by this server**. The widget runs the Chime
SDK in a web page and authenticates to Amazon's hosted backend via the
KMS-encrypted `snippetId`; the JWT (HS256, signed with the 44-char widget
secret) just carries `attributes` for that browser session. There is no headless
endpoint that takes `snippetId`+JWT and returns Chime connection data.

This server uses the **IAM `StartWebRTCContact`** path instead. The widget is
still useful for two things it tells us:
- the instance has in-app/web calling + a routing flow enabled, and
- the attribute-key convention the flow reads: `$.Attributes.HostedWidget-<name>`.
  We therefore emit `HostedWidget-`-prefixed attribute keys so SIP-bridged calls
  drive the **same** screen pop. The JWT's `attributes` claim is simply the
  browser equivalent of what `AttributeMapping` produces here.

## Run it

Offline (default) — SIP UAS on `udp/5060` with a mock control plane. It answers
SIP and prints the attributes that *would* be sent to `StartWebRTCContact`; the
Connect media leg then fails fast against a fake signaling URL (expected without
AWS):

```bash
cargo run
```

Live — bridge a real SIP call into Amazon Connect (needs IAM creds with
`connect:StartWebRTCContact`):

```bash
export AWS_REGION=us-west-2
export AWS_ACCESS_KEY_ID=...          # or use an instance/ECS role
export AWS_SECRET_ACCESS_KEY=...
export AMAZON_CONNECT_INSTANCE_ID=<uuid>
export AMAZON_CONNECT_FLOW_ID=<uuid>  # the WebRTC/agent-routing flow the widget uses

# Reachability for an external caller (Vapi):
export SIP_BIND_IP=0.0.0.0
export SIP_ADVERTISED_ADDR=<public-ip>:5060   # behind NAT, so Via/Contact are routable

cargo run --features aws-live
```

## Drive a call

With the server running, send it an INVITE carrying custom headers — e.g. with
`sipp`, another rvoip endpoint, or your Vapi blind-transfer target — including
the headers the `AttributeMapping` renames (edit `src/main.rs` to match the exact
header names Vapi sends and the exact attribute keys your flow checks):

```
X-Vapi-Customer-Id: cust-42
X-Account-Tier: platinum
```

Watch the logs: header translation → `StartWebRTCContact` (real `contact_id`) →
Chime JOIN/SUBSCRIBE → `wait_connected` → "bridged SIP ⟷ Amazon Connect". The
agent's CCP rings and the screen pop fires from the `HostedWidget-`-prefixed
attributes.

## Notes / follow-ups

- **Verify attribute keys** against your flow's *Check contact attributes* block;
  adjust the `.rename(...)` calls in `src/main.rs` accordingly.
- **Teardown on SIP BYE is wired.** The server runs a teardown watcher: when the
  SIP leg ends (`CallEnded`/`CallFailed`/`CallCancelled`) it ends the Connect
  contact and aborts the bridge — no leaked Connect contacts.
- This is the first real exercise of the Chime signaling URL / JOIN-frame wiring;
  if media fails to connect, see `signaling/chime.rs` and the crate's
  `docs/IMPLEMENTATION_PLAN.md` live-validation checklist.
