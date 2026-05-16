# Endpoint softphone walkthrough

**Pattern mapping:** SIP_API_DESIGN_2 §11.1 (canonical UAC), §3.3
builder lifecycle.

## What it demonstrates

| API call | Purpose |
|---|---|
| `coord.register(uri, user, pass).with_expires(s).send().await?` | Register with the configured registrar |
| `coord.invite(from, target).send().await?` | Place an outbound call |
| `coord.hold(&session).await?` | RFC 3264 hold (re-INVITE with `a=sendonly`) |
| `coord.resume(&session).await?` | Resume from hold |
| `coord.send_dtmf(&session, '5').await?` | Send DTMF — RFC 2833 in-band by default |
| `coord.hangup(&session).await?` | Tear down with BYE |

## Run

```
cargo run --example endpoint_softphone
```

The example boots a mock registrar (raw UDP, embedded), a callee (`bob`
as a `CallbackPeer<AutoAccept>`), and a softphone (`alice` as a
`UnifiedCoordinator`). Alice registers, places a call to bob, holds /
resumes the dialog, sends a DTMF digit, then hangs up.

## Why the mock registrar is inline

The full registrar lives in `rvoip-sip-registrar`, but for the
self-contained walkthrough we only need REGISTER-200-OK, so a 30-line
raw-UDP responder is embedded. Production code should use
`coord.start_registration_server(realm, users)` instead.
