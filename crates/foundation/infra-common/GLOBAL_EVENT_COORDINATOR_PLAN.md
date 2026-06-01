# Global Event Coordinator — Plan (outstanding work)

The plan's core shipped: `GlobalEventCoordinator` is a true process-wide
singleton for monolithic deployments (`OnceCell` + `global_coordinator()` in
`src/global/coordinator.rs`), the legacy `monolithic()` constructor is
`#[deprecated]`, and a distributed-transport trait exists with NATS/gRPC
**stubs** that return clear "not implemented" errors.

This doc was trimmed (2026-06-01) to track **only what remains**; the original
phased plan and design rationale are in git history.

## Outstanding — distributed transport (stubbed by design)

The distributed path is intentionally non-functional today. Making it real:

- **NATS transport** — real connect + event (de)serialization (`src/global/transport/mod.rs` has the serialization TODOs; the stub round-trips nothing).
- **gRPC transport** — currently fully stubbed (`src/global/transport/grpc.rs` returns "not implemented").
- Service discovery; Redis pub/sub transport option.
- Metrics / monitoring and circuit breakers for distributed mode.

## Open questions (maintainer decisions)

1. Environment variables vs config files for distributed configuration?
2. Deprecate `monolithic()` immediately, or after a grace period?
3. Feature-flag the distributed code, or always compile it?
4. Lazy (first-access) vs eager (app-startup) singleton initialization?
