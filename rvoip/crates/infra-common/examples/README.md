# Event Bus Examples

This directory contains examples showing how to use the event bus system in infra-common.

## Running Examples

You can run any example using cargo:

```bash
# Debug mode
cargo run --example api_simple_fastpath
cargo run --example api_simple_zerocopy

# Release mode (recommended for benchmarks)
cargo run --release --example api_bench_both
cargo run --release --example core_bench_both
```

## Available Examples

### Simple API Examples

These examples demonstrate basic usage patterns using the public API:

#### API Simple Fast Path (`api_simple_fastpath.rs`)

A simple example showing how to use the StaticFastPath implementation via the public API:
- Creating an event system with the Static Fast Path implementation
- Defining and publishing static events
- Subscribing to events using the unified API
- Basic event handling and processing

#### API Simple Zero Copy (`api_simple_zerocopy.rs`)

A simple example showing how to use the ZeroCopy implementation via the public API:
- Creating an event system with the Zero Copy implementation
- Configuring advanced features (priorities, timeouts, etc.)
- Publishing events through the unified API
- Subscribing and processing events with different priority levels

### Direct Implementation Examples

These examples show direct usage of the underlying implementations:

#### Core Fast Path (`core_fastpath.rs`)

An example demonstrating direct usage of the Static Fast Path implementation:
- Using the `FastPublisher` directly without the API layer
- Creating and managing static event types
- High-performance publishing and subscribing
- Detailed usage patterns for maximum performance

#### Core Zero Copy (`core_zerocopy.rs`)

An example demonstrating direct usage of the Zero Copy event bus:
- Configuring and using the EventBus with zero-copy optimizations
- Working with broadcast channels directly
- Advanced configuration options
- Handling events with specific priorities and timeouts

### Benchmarking Examples

These examples provide standardized benchmarks to compare performance:

#### API Benchmark Both (`api_bench_both.rs`)

A comprehensive benchmark for comparing both implementations using the public API:
- Tests both Static Fast Path and Zero Copy implementations
- Uses identical parameters for fair comparison
- Measures sustained throughput over 30 seconds
- Uses 5 subscribers for realistic workload testing

#### Core Benchmark Both (`core_bench_both.rs`)

A benchmark identical to api_bench_both.rs but using direct implementation access:
- Tests both implementations without the API abstraction layer
- Uses the same parameters as the API version for direct comparison
- Helps quantify any overhead from the API abstraction
- Useful for validating consistency between abstraction layers

## Performance Comparison

We've conducted sustained performance tests with identical parameters (5 subscribers, 30-second runtime) in debug mode:

| Implementation | Processing Rate | Notes |
|----------------|----------------:|-------|
| Zero Copy | ~2,230,000 events/sec | Surprisingly faster in our specific test workload |
| Static Fast Path | ~900,000 events/sec | More consistent but slower in our test scenario |

**Key observations:**
- The Zero Copy implementation outperformed Static Fast Path by approximately 2.5x in our specific test scenario
- Both implementations used identical channel capacities (10,000), event payload structures, and test parameters
- Performance characteristics may vary based on workload and configuration

## API vs. Direct Implementation Benchmarks

We've benchmarked the performance difference between using the public API abstraction layer and directly using the underlying implementation with our updated test methodology:

### Latest Benchmark Results

| Implementation | API Rate (events/sec) | Direct Rate (events/sec) | Overhead (%) |
|----------------|----------------------:|--------------------------|--------------|
| Static Fast Path | 899,377 | 897,009 | -0.26% |
| Zero Copy | 2,230,940 | 2,242,736 | 0.53% |

### Key Findings

1. **Negligible API Overhead**: Our latest tests show virtually no performance penalty when using the public API abstraction layer versus direct implementation access, with differences less than 1%.

2. **Zero Copy Advantage**: In our specific test workload, the Zero Copy implementation significantly outperforms the Static Fast Path implementation, contradicting earlier benchmarks. This may be due to improvements in the Zero Copy implementation or specific characteristics of our test scenario.

3. **Consistency**: Results are consistent between API and direct implementations, confirming that the benchmarks are reliable and the abstraction layer adds minimal overhead.

4. **Recommendation**: Use the public API abstraction layer with confidence, as it provides a clean interface without measurable performance penalties.

## When to Use Each Approach

- **Static Event FastPublisher**: Use when you need a simpler implementation with consistent performance characteristics. Best when you know the event types at compile time. Ideal for media packets, telemetry, or other high-volume data flows that don't require complex routing.

- **Zero-Copy EventBus**: Use when you need the full feature set of the event bus (filtering, prioritization, timeout handling, etc.) and want the best possible performance. Surprisingly, this implementation may outperform the Static Fast Path in many scenarios.

## Benchmarking Methodology

For accurate performance measurements in our updated benchmark:

1. Each test runs for a full 30 seconds to measure sustained throughput
2. Tests are executed with 5 subscribers receiving and processing every message
3. Both implementations use the same:
   - Channel capacity (10,000)
   - Event payload structure (MediaPacketEvent)
   - Test duration (30 seconds)
   - Subscriber count (5)
   - Worker thread count (4)

The updated benchmarks provide a fair comparison of both implementations under identical conditions with minimal differences between the API abstraction and direct implementation access. 