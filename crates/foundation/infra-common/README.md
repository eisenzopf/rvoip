# rvoip-infra-common

Shared infrastructure layer for the [rvoip](../../../README.md) stack. Every
other rvoip crate can lean on it for the pieces that should behave the same
everywhere:

- **Event system** (`events`) — a publish/subscribe bus with two
  implementations selected through `EventSystemBuilder`
  (`ImplementationType::ZeroCopy` and `ImplementationType::StaticFastPath`),
  plus the cross-crate event definitions in `events::cross_crate` that carry
  signaling, dialog, and media events between crates (including redacted
  SIP trace events).
- **Configuration** (`config`) — `ConfigProvider`, `ConfigLoader` for
  file/environment loading, `DynamicConfig` for hot-reloadable values, and
  schema validation helpers.
- **Lifecycle** (`lifecycle`) — the `Component` trait plus dependency,
  health, and manager helpers.
- **Logging and metrics** (`logging`) — `setup_logging` / `LoggingConfig`
  on top of `tracing`, `LogContext` spans, and a small `MetricsCollector`.
- **Planes** (`planes`) — the `FederatedPlane` abstractions for transport,
  media, and signaling planes in monolithic or distributed deployments.
- **Errors** (`errors`) — the shared `Error` type.

The crate sets `mimalloc` as the global allocator unless the
`no-global-allocator` feature is enabled. Bus payloads carry raw inbound
bytes (`Arc<bytes::Bytes>`), not SIP types, so this crate stays
protocol-agnostic.

## Install

```toml
[dependencies]
rvoip-infra-common = "0.3.12"
```

## Cargo features

All features are off by default.

- `no-global-allocator` — skip the `#[global_allocator]` declaration. Needed
  by profiling consumers (for example `rvoip-sip`'s `dhat` feature) that
  install their own allocator.
- `memory-diagnostics` — enable the `memory_diagnostics` module (object and
  allocation counters, `spawn_tracked`) and mimalloc's extended API.
- `otel` — wire an OpenTelemetry OTLP exporter layer into `setup_logging`.
  Spans are exported only when `LoggingConfig.otel_endpoint` is `Some(_)`;
  with the feature off the endpoint is ignored.

## Usage

The snippet below mirrors the doctest in `src/events/mod.rs`.

```rust,no_run
use rvoip_infra_common::events::api::EventSystem as _;
use rvoip_infra_common::events::builder::{EventSystemBuilder, ImplementationType};
use rvoip_infra_common::events::types::{Event, EventPriority};
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::time::Duration;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MyEvent {
    id: u32,
    message: String,
}

impl Event for MyEvent {
    fn event_type() -> &'static str {
        "my_event"
    }

    fn priority() -> EventPriority {
        EventPriority::Normal
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

async fn example() -> Result<(), Box<dyn std::error::Error>> {
    let system = EventSystemBuilder::new()
        .implementation(ImplementationType::ZeroCopy)
        .channel_capacity(1000)
        .build();
    system.start().await?;

    let mut subscriber = system.subscribe::<MyEvent>().await?;
    let publisher = system.create_publisher::<MyEvent>();
    publisher
        .publish(MyEvent { id: 1, message: "Hello".to_string() })
        .await?;

    let event = subscriber.receive_timeout(Duration::from_secs(1)).await?;
    println!("received id={} message={}", event.id, event.message);

    system.shutdown().await?;
    Ok(())
}
```

## Examples

See [`examples/README.md`](examples/README.md) for details and benchmark
numbers.

- `api_simple_fastpath` / `api_simple_zerocopy` — the two implementations
  through the public `EventSystem` API.
- `core_fastpath` / `core_zerocopy` — direct use of the underlying
  implementations.
- `api_bench_both` / `core_bench_both` — sustained-throughput benchmarks
  (run with `--release`).

```bash
cargo run -p rvoip-infra-common --example api_simple_zerocopy
cargo run -p rvoip-infra-common --release --example api_bench_both
```

## License

Licensed under the MIT license. See the repository [LICENSE](../../../LICENSE).
