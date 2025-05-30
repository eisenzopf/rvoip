# Call Engine - Call Center with Limbo Database

This crate provides call center orchestration functionality with integrated [Limbo](https://github.com/tursodatabase/limbo) database support for persistent storage.

## Features

- 🗄️ **Limbo Database Integration**: Modern SQLite-compatible database written in Rust
- 👥 **Agent Management**: Registration, skills, availability tracking
- 📞 **Call Records**: Complete call history and analytics
- 📋 **Queue Management**: Call queuing with overflow policies
- 🎯 **Routing Policies**: Skill-based and rule-based call routing
- 🚀 **Async-First**: Built on tokio for high performance

## Quick Start

### Add Dependencies

```toml
[dependencies]
rvoip-call-engine = { path = "../call-engine" }
tokio = { version = "1.0", features = ["full"] }
anyhow = "1.0"
tracing = "0.1"
tracing-subscriber = "0.3"
```

### Basic Usage

```rust
use rvoip_call_engine::prelude::*;
use rvoip_call_engine::database::{
    CallCenterDatabase,
    agent_store::{AgentStore, CreateAgentRequest, AgentStatus},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize database
    let db = CallCenterDatabase::new("call_center.db").await?;
    
    // Create agent store
    let agent_store = AgentStore::new(db);
    
    // Register an agent
    let agent_request = CreateAgentRequest {
        sip_uri: "sip:alice@company.com".to_string(),
        display_name: "Alice Johnson".to_string(),
        max_concurrent_calls: Some(2),
        department: Some("Support".to_string()),
        extension: Some("1001".to_string()),
        skills: Some(vec![
            ("english".to_string(), 5),
            ("technical_support".to_string(), 4),
        ]),
        ..Default::default()
    };
    
    let agent = agent_store.create_agent(agent_request).await?;
    println!("Created agent: {} ({})", agent.display_name, agent.id);
    
    // Set agent as available
    agent_store.update_agent_status(&agent.id, AgentStatus::Available).await?;
    
    // Find available agents with technical support skills
    let skilled_agents = agent_store.get_available_agents(
        Some(&["technical_support".to_string()])
    ).await?;
    
    println!("Found {} agents with technical support skills", skilled_agents.len());
    
    Ok(())
}
```

## Database Schema

### Agents Table
- **Agent Registration**: SIP URI, display name, department, extension
- **Availability Tracking**: Status (available/busy/away/offline), last seen
- **Capacity Management**: Max concurrent calls per agent
- **Skill Profiles**: Multi-level skills for routing

### Call Records Table  
- **Call Tracking**: Session ID, bridge ID, caller/callee information
- **Timing Data**: Start time, answer time, end time, duration
- **Quality Metrics**: Call quality scores, disconnect reasons
- **Agent Association**: Which agent handled the call

### Call Queues Table
- **Queue Configuration**: Name, description, priority, max wait time
- **Skill Requirements**: JSON array of required skills
- **Overflow Handling**: Overflow queue routing
- **Business Hours**: JSON business hours configuration

### Routing Policies Table
- **Policy Rules**: JSON conditions and actions
- **Priority Management**: Policy execution order
- **Dynamic Configuration**: Enable/disable policies at runtime

## Examples

### Running the Example

```bash
cd rvoip/crates/call-engine
cargo run --example call_center_with_database
```

This example demonstrates:
- 🏗️ Database initialization
- 👥 Creating agents with skills
- 📱 Updating agent availability status
- 🔍 Finding agents by skills
- 📊 Querying agent information

### Example Output

```
🚀 Starting Call Center with Limbo Database Example
✅ Database initialized successfully
👥 Creating sample agents...
✅ Created agent: Alice Johnson (a1b2c3d4-...)
✅ Created agent: Bob Smith (e5f6g7h8-...)
✅ Created agent: Carol Davis (i9j0k1l2-...)
📱 Setting agents to available status...
✅ Agent Alice Johnson is now available
✅ Agent Bob Smith is now available
🔍 Finding available agents...
Found 2 available agents:
  📞 Alice Johnson (1001) - sip:alice@company.com
  📞 Bob Smith (1002) - sip:bob@company.com
🎯 Finding agents with technical support skills...
Found 1 agents with technical support skills:
  🔧 Alice Johnson - Technical Support Agent
    - english (level 5)
    - technical_support (level 4)
    - billing (level 3)
💚 Testing database health...
✅ Database health check passed
🎉 Call Center Database Example Complete!
```

## Database Configuration

### File-based Database

```rust
// Production setup with persistent storage
let db = CallCenterDatabase::new("./data/call_center.db").await?;
```

### In-Memory Database

```rust
// Testing setup with in-memory storage  
let db = CallCenterDatabase::new_in_memory().await?;
```

## Performance Features

- **Async I/O**: Limbo's native async support for high concurrency
- **Optimized Queries**: Indexed columns for fast agent lookups
- **Connection Pooling**: Efficient database connection management
- **Batch Operations**: Bulk operations for high-volume scenarios

## Integration with Session-Core

The database layer integrates seamlessly with session-core for complete call center functionality:

```rust
// Call center orchestration with database persistence
let call_center = CallCenterEngine::new(session_manager, config).await?;

// Incoming call gets routed based on database policies
let session_id = call_center.handle_incoming_call(request).await?;

// Agent selection uses database skill matching
let agent = call_center.find_best_agent(call_requirements).await?;

// Bridge creation with database call record tracking
let bridge_id = call_center.bridge_to_agent(session_id, agent_id).await?;
```

## Testing

```bash
# Run all tests
cargo test

# Run database integration tests specifically
cargo test test_database_integration
```

## Architecture Benefits

### Why Limbo?

1. **Pure Rust**: Perfect integration with our Rust stack
2. **Async-First**: Built for high-performance async applications
3. **SQLite Compatible**: Familiar SQL dialect and tooling
4. **Embedded**: No separate database server required
5. **Modern**: Active development by database experts at Turso

### Performance Characteristics

- **Async I/O**: Non-blocking database operations
- **Zero-Copy**: Efficient memory usage for large datasets  
- **Indexing**: Fast lookups for agent availability and skills
- **Transactions**: ACID compliance for data consistency

## Future Enhancements

- **Clustering**: Multi-node database distribution
- **Replication**: Real-time data synchronization
- **Analytics**: Advanced call center metrics and reporting
- **Machine Learning**: Predictive routing and capacity planning

## Contributing

When adding new database functionality:

1. Create the data model in the appropriate store module
2. Add database schema in `schema.rs`
3. Implement the store methods with proper error handling
4. Add comprehensive tests
5. Update the example if needed

---

For more information about Limbo, visit: https://github.com/tursodatabase/limbo 