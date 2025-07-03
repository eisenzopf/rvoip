# infra-common: Cross-Cutting Infrastructure Layer

This crate provides common infrastructure that supports all other components in the RVOIP stack. Instead of each component implementing its own events, configuration, lifecycle, and logging systems, infra-common establishes standardized patterns that all components can leverage.

## Purpose

The infra-common crate exists to:

1. Reduce duplication across components
2. Establish consistent patterns for common needs
3. Provide a uniform foundation for all RVOIP components
4. Enable better component integration through standardized interfaces

## Components

### Directory Structure and Files

```
infra-common/
├── Cargo.toml
├── src/
│   ├── lib.rs                   # Main library exports and documentation
│   ├── events/                  # Event system
│   │   ├── mod.rs               # Export public event API
│   │   ├── bus.rs               # Event bus implementation
│   │   ├── subscriber.rs        # Event subscription interfaces
│   │   ├── publisher.rs         # Event publication interfaces
│   │   └── types.rs             # Common event types and traits
│   │
│   ├── config/                  # Configuration system
│   │   ├── mod.rs               # Export public configuration API
│   │   ├── loader.rs            # Configuration loading from files/env
│   │   ├── provider.rs          # Configuration provider interfaces
│   │   ├── schema.rs            # Configuration schema validation
│   │   └── dynamic.rs           # Hot-reload of configuration
│   │
│   ├── lifecycle/               # Component lifecycle management
│   │   ├── mod.rs               # Export public lifecycle API
│   │   ├── component.rs         # Component trait definitions
│   │   ├── manager.rs           # Lifecycle orchestration
│   │   ├── dependency.rs        # Dependency resolution
│   │   └── health.rs            # Health checking utilities
│   │
│   ├── logging/                 # Logging and tracing
│   │   ├── mod.rs               # Export public logging API
│   │   ├── setup.rs             # Logger initialization
│   │   ├── context.rs           # Contextual logging
│   │   └── metrics.rs           # Standardized metrics collection
│   │
│   └── errors/                  # Error handling
│       ├── mod.rs               # Export public error API
│       ├── types.rs             # Common error types
│       └── context.rs           # Error context utilities
```

## Implementation Plan

### 1. Event System

The event system should provide:

- A strongly-typed event bus that components can subscribe to
- Support for both synchronous and asynchronous event handling
- Clean interfaces for event publication and subscription
- Support for scoped events (component-specific vs. global)
- Flexible filtering and routing capabilities
- High-performance event processing for systems with thousands of concurrent calls

Implementation approach:
- Use generics and type-safe channels for event dispatch
- Leverage Tokio for async event handling
- Provide both static (compile-time) and dynamic (runtime) dispatch options
- Support middleware for event transformation and filtering

### 1.1 High-Performance Event Bus Optimizations

The event bus needs specific optimizations to handle high-scale VoIP operations (10s of thousands of concurrent calls):

- Replace standard library locks with Tokio's async-aware locks
- Implement broadcast-based event distribution for efficient multi-subscriber delivery
- Add buffered dispatching with concurrency controls to prevent system overload
- Implement subscriber storage sharding to reduce lock contention
- Add timeout and circuit breaking for resilience against slow subscribers
- Support event prioritization to ensure critical events are processed first
- Implement object pooling for high-frequency events to reduce memory pressure

Implementation approach:
- Use tokio::sync::RwLock instead of std::sync::RwLock for non-blocking lock operations
- Leverage tokio::sync::broadcast for efficient multi-consumer event channels
- Use tokio::sync::Semaphore for limiting concurrent event dispatching
- Implement sharded storage with consistent hashing for distributing event subscriptions
- Add timeout handling with tokio::time::timeout for resilience
- Create event priority system with separate dispatch queues
- Build object pool using tokio::sync::Mutex for high-frequency event types

### 1.2 Ultra-High Performance Event Bus Enhancements

Additional optimizations to reach 100,000+ concurrent events:

- **Zero-Copy Architecture**: Replace serialization-based broadcast with Arc-wrapped events
- **Lock-Free Data Structures**: Replace RwLocks with concurrent data structures like DashMap
- **Specialized Fast Paths**: Add optimized code paths for high-volume event types
- **Batched Event Processing**: Support publishing and processing events in batches
- **SIMD-Accelerated Serialization**: Use SIMD when serialization is unavoidable
- **Sharded Event Bus**: Create multiple domain-specific event buses to reduce contention
- **Pre-allocation Strategy**: Pre-register event types at startup to avoid runtime allocation
- **Backpressure Handling**: Add proper backpressure for event producers
- **Channel-Only Design**: Migrate to a pure channel-based approach for all subscribers
- **Custom Memory Allocator**: Use specialized allocators like mimalloc for better performance

Implementation approach:
- Use Arc<E> instead of serialized Box<Vec<u8>> to avoid copying and serialization overhead
- Replace Arc<RwLock<HashMap<...>>> with DashMap for concurrent access without lock contention
- Implement StaticEvent trait with cached type information and senders
- Add batch processing APIs for high-volume event scenarios
- Implement SIMD-based serialization when needed (using libraries like simd-json)
- Create a TypeRegistry for type-based channel lookup and creation
- Implement both static (OnceCell) and dynamic type registration
- Add backpressure with channel capacity limits and producer flow control
- Eliminate direct subscribers in favor of uniform channel-based approach
- Configure mimalloc or jemalloc as global allocator for better memory performance

Performance targets:
- Single publisher/multiple subscribers: 100K+ events/sec (from 8K)
- Broadcast channels with 1000+ subscribers: 250K+ events/sec (from 12K)
- Memory usage: 60-70% reduction with zero-copy approach
- Latency: Under 1ms for critical path events

### 2. Configuration System

The configuration system should provide:

- A unified approach to configuration across components
- Support for hierarchical configuration
- Configuration validation against schemas
- Dynamic configuration updates (when possible)
- Environment-specific configuration overrides

Implementation approach:
- Use traits to define configuration requirements
- Support common formats (TOML, JSON, YAML)
- Provide builder patterns for configuration instantiation
- Include schema validation using serde
- Support configuration isolation for components

### 3. Lifecycle Management

The lifecycle system should provide:

- Standardized component lifecycle (init, start, stop, shutdown)
- Dependency resolution between components
- Graceful startup and shutdown sequences
- Health checks and readiness indicators

Implementation approach:
- Define Component trait with lifecycle methods
- Create a dependency graph for component orchestration
- Implement startup/shutdown ordering
- Provide timeout and cancellation handling

### 4. Logging and Metrics

The logging system should provide:

- Consistent logging patterns across all components
- Structured logging with contextual information
- Integration with tracing ecosystem
- Standardized metric collection points
- Performance monitoring utilities

Implementation approach:
- Build on tracing/tracing-subscriber ecosystem
- Define standard log contexts and levels
- Create convenience macros for common logging patterns
- Integrate with common metrics libraries (prometheus, etc.)

### 5. Error Handling

The error handling system should provide:

- Standard error types that components can extend
- Error context utilities for enriching error information
- Error mapping between component boundaries
- Consistent error reporting patterns

Implementation approach:
- Create a base Error enum with component-specific variants
- Use thiserror/anyhow for ergonomic error handling
- Define mapping traits for inter-component error conversion
- Provide context utilities for tracking error propagation

## Integration Guidelines

When integrating infra-common with other components:

1. Components should implement the Component trait for lifecycle management
2. Components should leverage the event system rather than creating custom solutions
3. Configuration should follow the established patterns
4. Logging should use the standardized approach
5. Error types should extend from or map to the common error types

## Design Considerations

- **Minimal Dependencies**: Keep external dependencies to a minimum
- **Flexibility**: Allow components to choose their level of integration
- **Performance**: Ensure the infrastructure layer adds minimal overhead
- **Testability**: Make all components easily testable in isolation
- **Backward Compatibility**: Define a clear versioning strategy 