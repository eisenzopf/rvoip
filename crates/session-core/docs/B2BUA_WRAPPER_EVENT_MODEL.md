# B2BUA Wrapper Event Model

This note records the event-routing decision made during the session-core
hardening pass.

## Decision

Do not add `coordinator_id` or B2BUA ownership metadata to
`session-core` app events.

The first B2BUA crate should use one `UnifiedCoordinator` per B2BUA
instance/listener. Each B2BUA call owns two `SessionId`s inside that
coordinator:

- inbound leg
- outbound leg

The B2BUA wrapper subscribes once with `UnifiedCoordinator::events()` and
routes events through its own registry:

```text
BridgeId -> inbound SessionId
BridgeId -> outbound SessionId
SessionId -> BridgeId + LegRole
```

`session-core` remains the programmable UA/session layer. B2BUA call graph
ownership, leg pairing, cause propagation, media bridge lifetime, and future
multi-coordinator event namespacing belong in the wrapper crate.

## Future Multi-Coordinator Shape

If a future B2BUA process needs several independent coordinators, the wrapper
can tag events after receiving them:

```rust,ignore
struct B2buaEvent {
    coordinator: B2buaCoordinatorId,
    bridge_id: Option<BridgeId>,
    leg: Option<LegRole>,
    session_id: Option<SessionId>,
    event: rvoip_session_core::Event,
}
```

That keeps the global bus contract stable for existing session-core users and
lets the B2BUA crate define the topology semantics where they are needed.
