# Global Event Coordinator Implementation Plan

## Overview
This document outlines the plan to implement a true global singleton for the `GlobalEventCoordinator` in monolithic deployments while stubbing out support for distributed deployments.

## Goals
1. **Immediate**: Implement a global singleton for monolithic deployments that "just works"
2. **Future**: Stub out distributed deployment support without implementing it
3. **Maintain**: Backward compatibility with existing code where possible
4. **Optimize**: Memory and CPU efficiency for monolithic deployments

## Current State Analysis

### Problems with Current Implementation
- `GlobalEventCoordinator::monolithic()` creates a new instance each time
- Every crate must create and pass around its own coordinator instance
- No true "global" coordination despite the name
- Unnecessary Arc wrapping and cloning throughout the codebase

### Existing Good Foundations
- `DeploymentMode` enum already exists (Monolithic/Distributed)
- `EventBusAdapter` trait provides transport abstraction
- `CrossCrateEvent` designed for serialization
- Event routing infrastructure already in place

## Proposed Architecture

### 1. Configuration System
```rust
// New configuration types in infra-common/src/events/config.rs
pub struct EventCoordinatorConfig {
    pub deployment: DeploymentConfig,
    pub service_name: String,
}

pub enum DeploymentConfig {
    /// Single process, all components in one binary
    Monolithic {
        // No config needed for monolithic
    },
    /// Multi-process, components communicate over network
    Distributed {
        transport: TransportConfig,
        discovery: ServiceDiscoveryConfig,
    },
}

pub enum TransportConfig {
    Nats { servers: Vec<String> },
    Grpc { endpoint: String },
    // Future: Redis, RabbitMQ, etc.
}

pub enum ServiceDiscoveryConfig {
    Static { endpoints: HashMap<String, String> },
    // Future: Consul, Kubernetes, etc.
}
```

### 2. Global Singleton Access Pattern

```rust
// For monolithic (default) - uses singleton
let coordinator = global_coordinator().await;

// For distributed (future) - creates instance with config
let coordinator = GlobalEventCoordinator::distributed(config).await?;
```

### 3. Environment-Based Configuration

```rust
// Automatically detect deployment mode from environment
pub async fn global_coordinator() -> &'static Arc<GlobalEventCoordinator> {
    static COORDINATOR: OnceCell<Arc<GlobalEventCoordinator>> = OnceCell::const_new();
    
    COORDINATOR.get_or_init(|| async {
        // Check environment for deployment mode
        let deployment = if std::env::var("RVOIP_DISTRIBUTED").is_ok() {
            // Read distributed config from env/file
            load_distributed_config()
        } else {
            DeploymentConfig::Monolithic {}
        };
        
        Arc::new(GlobalEventCoordinator::new(deployment).await.expect("..."))
    }).await
}
```

## Implementation Plan

### Phase 1: Monolithic Singleton (Implement Now)

1. **Update `coordinator.rs`**:
   - Add `OnceCell` for singleton storage
   - Implement `global_coordinator()` function
   - Keep existing `monolithic()` for backward compatibility
   - Add deprecation notice to `monolithic()`

2. **Create `config.rs`**:
   - Define configuration types
   - Implement config loading (stub for distributed)

3. **Update Existing Libraries**:
   - `session-core-v2`: Use `global_coordinator()` instead of creating instances
   - `dialog-core`: Same update
   - `media-core`: Same update
   - Remove unnecessary Arc wrapping

4. **Testing**:
   - Verify singleton behavior
   - Ensure thread safety
   - Test with `api_peer_audio` example

### Phase 2: Distributed Stubs (Implement Now, But Non-Functional)

1. **Create `transport/mod.rs`**:
   ```rust
   pub trait NetworkTransport: Send + Sync {
       async fn send(&self, target: &str, event: &[u8]) -> Result<()>;
       async fn receive(&self) -> Result<Vec<u8>>;
   }
   ```

2. **Create `transport/nats.rs`** (stubbed):
   ```rust
   pub struct NatsTransport;
   
   impl NetworkTransport for NatsTransport {
       async fn send(&self, _target: &str, _event: &[u8]) -> Result<()> {
           todo!("NATS transport not yet implemented")
       }
       // ...
   }
   ```

3. **Update `GlobalEventCoordinator`**:
   - Add `distributed()` constructor (returns error for now)
   - Add transport field (Option<Box<dyn NetworkTransport>>)
   - Route events based on deployment mode

### Phase 3: Migration Path

1. **Deprecation Warnings**:
   ```rust
   #[deprecated(note = "Use global_coordinator() for monolithic or GlobalEventCoordinator::distributed() for distributed deployments")]
   pub async fn monolithic() -> Result<Self>
   ```

2. **Migration Guide** (in README):
   - Show before/after code examples
   - Explain benefits
   - Provide migration script if needed

## Files to Modify

### infra-common
- `src/events/coordinator.rs` - Add singleton, update constructors
- `src/events/config.rs` (new) - Configuration types
- `src/events/transport/mod.rs` (new) - Transport trait
- `src/events/transport/nats.rs` (new) - NATS stub
- `src/events/mod.rs` - Export new modules

### session-core-v2
- `src/api/unified.rs` - Use `global_coordinator()`
- Remove coordinator passing through constructors

### dialog-core
- `src/api/unified.rs` - Use `global_coordinator()`
- `src/events/event_hub.rs` - Update to use singleton

### media-core
- `src/api/mod.rs` - Use `global_coordinator()`
- `src/events/event_hub.rs` - Update to use singleton

## Testing Strategy

1. **Unit Tests**:
   - Test singleton initialization
   - Test thread safety
   - Test configuration loading

2. **Integration Tests**:
   - Update existing tests to use singleton
   - Add test for multiple access points returning same instance

3. **Example Updates**:
   - Update `api_peer_audio` to demonstrate new pattern
   - Create new example showing distributed config (even if stubbed)

## Benefits

### Monolithic Mode
- **Memory**: Single instance instead of N instances
- **CPU**: No Arc cloning overhead
- **Simplicity**: Just call `global_coordinator()` anywhere
- **Thread-safe**: Guaranteed single initialization

### Distributed Mode (Future)
- **Flexibility**: Pluggable transports
- **Scalability**: Services can run on different machines
- **Resilience**: Network transport can handle failures

## Risks and Mitigations

1. **Risk**: Breaking existing code
   - **Mitigation**: Keep `monolithic()` working with deprecation warning

2. **Risk**: Thread safety issues
   - **Mitigation**: Use proven `OnceCell` pattern, extensive testing

3. **Risk**: Distributed stub confusion
   - **Mitigation**: Clear error messages, documentation

## Timeline

- **Day 1**: Implement singleton and config system
- **Day 2**: Update all libraries to use singleton
- **Day 3**: Add distributed stubs and transport trait
- **Day 4**: Testing and documentation
- **Day 5**: Migration guide and examples

## Success Criteria

1. `api_peer_audio` example works with singleton
2. All tests pass
3. Memory usage reduced (measurable)
4. Distributed mode returns clear "not implemented" errors
5. Migration path documented

## Future Work (Not in This Phase)

1. Implement NATS transport
2. Implement service discovery
3. Add gRPC transport option
4. Add Redis pub/sub transport
5. Metrics and monitoring
6. Circuit breakers for distributed mode

## Questions for Review

1. Should we use environment variables or config files for distributed configuration?
2. Should `monolithic()` be immediately deprecated or after a grace period?
3. Do we need a feature flag for distributed code or always compile it?
4. Should the singleton be lazy (first access) or eager (app startup)?
