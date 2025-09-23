# State Table Configuration

The session-core-v2 library supports three methods for loading state tables, with a clear priority order:

## Priority Order

1. **Config Path** (highest priority)
2. **Environment Variable** (`RVOIP_STATE_TABLE`)
3. **Embedded Default** (fallback)

## Configuration Methods

### 1. Using Config Path (Recommended for Production)

```rust
use rvoip_session_core_v2::api::simple::{SimplePeer, Config};

let config = Config {
    state_table_path: Some("/path/to/my-custom-states.yaml".to_string()),
    ..Default::default()
};
let peer = SimplePeer::with_config("alice", config).await?;
```

### 2. Using Environment Variable

```bash
# Set the environment variable
export RVOIP_STATE_TABLE="/path/to/custom-states.yaml"

# Run your application
cargo run
```

```rust
// In your code - no configuration needed
let peer = SimplePeer::new("alice").await?;
```

### 3. Using Embedded Default

```rust
// Just create a peer - the default state table is automatically used
let peer = SimplePeer::new("alice").await?;
```

## Default State Table

The library includes a comprehensive default state table with:
- Basic call states (Idle, Initiating, Ringing, Active, etc.)
- Advanced features (Hold, Mute, Transfer)
- Conference support
- Both UAC (caller) and UAS (callee) roles

## Creating Custom State Tables

Custom state tables should follow the YAML format:

```yaml
version: "1.0"

metadata:
  description: "My custom state machine"
  author: "Your Name"

states:
  - name: "Idle"
    description: "No active call"
  # ... more states

events:
  - MakeCall
  - IncomingCall
  # ... more events

transitions:
  - role: "UAC"
    from_state: "Idle"
    event: "MakeCall"
    to_state: "Initiating"
    actions:
      - CreateSession
      - SendINVITE
    # ... more transitions
```

## Examples

See the included state tables:
- `state_tables/default_state_table.yaml` - The embedded default
- `state_tables/enhanced_state_table.yaml` - Extended features
- `state_tables/session_coordination.yaml` - Coordination features

## Testing Your Configuration

Run the test example to verify loading behavior:

```bash
cargo run --example test_state_table_loading_simple
```

This will show you which state table is being loaded and from where.
