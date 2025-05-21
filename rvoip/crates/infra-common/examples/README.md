# Event Bus Examples

This directory contains examples showing how to use the event bus system in infra-common.

## Running Examples

You can run any example using cargo:

```bash
# Debug mode
cargo run --example static_event_publisher
cargo run --example zero_copy_event_bus

# Release mode (recommended for benchmarks)
cargo run --release --example static_event_publisher
cargo run --release --example zero_copy_event_bus
```

## Available Examples

### Static Event Publisher (`static_event_publisher.rs`)

This example demonstrates the high-performance `FastPublisher` for static events, which benchmarks show is the fastest event bus implementation. It shows:

- How to define custom static events
- Creating a `FastPublisher` for your event types
- Subscribing to events from multiple subscribers
- Publishing events efficiently
- Handling events in an async context

The example uses realistic event types that might be used in a VoIP system.

### Zero-Copy Event Bus (`zero_copy_event_bus.rs`)

This example demonstrates the regular EventBus with zero-copy optimization enabled. It shows:

- How to configure an EventBus for zero-copy performance
- Defining events and handlers
- Implementing the EventHandler trait
- Publishing events directly through the EventBus
- Multiple publishers and subscribers working concurrently

## Performance Comparison

We've conducted sustained performance tests with identical parameters (20 subscribers, 5 concurrent publishers, 30-second runtime) in release mode:

| Implementation | Publishing Throughput | Processing Ratio | Notes |
|----------------|----------------------:|----------------:|-------|
| Static Event FastPublisher | ~1,153,000 events/sec | 2000% | Global registry, no routing overhead, highly optimized |
| Zero-Copy EventBus | ~319,000 events/sec | 661% | Full event bus capabilities with zero-copy optimization |

**Key observations:**
- The Static Event FastPublisher is roughly 3.6x faster than the Zero-Copy EventBus
- Processing ratio shows the static implementation consistently delivers events to all subscribers more reliably
- Both implementations use identical channel capacities (10,000) and event payload structures for fair comparison
- Both show consistent performance across multiple test runs

## When to Use Each Approach

- **Static Event FastPublisher**: Use when you need maximum performance and don't need the advanced routing features of the full event bus. Best for high-frequency events where you know the event types at compile time. Ideal for media packets, telemetry, or other high-volume data flows.

- **Zero-Copy EventBus**: Use when you need the full feature set of the event bus (filtering, prioritization, timeout handling, etc.) but still want good performance. More flexible but with some performance trade-offs compared to the static approach. Better for control events, state changes, or when you need advanced routing capabilities.

## Benchmarking Methodology

For accurate performance measurements:

1. Each test runs for a full 30 seconds to measure sustained throughput
2. Tests are executed in release mode with full optimizations
3. Both implementations use multiple publishers working concurrently
4. All subscribers receive and process every message
5. Processing is kept minimal to focus on event bus overhead
6. Both implementations use the same:
   - Channel capacity (10,000)
   - Event payload structure (MediaPacketEvent)
   - Number of publishers (5) and subscribers (20)
   - Test duration (30 seconds)

This provides a realistic and fair comparison of how each implementation performs under production-like conditions. 