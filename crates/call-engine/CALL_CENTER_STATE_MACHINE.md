# Call Center State Machine Design

This document preserves the call center specific states, events, and actions that were initially designed for session-core-v2 but are better suited for implementation in call-engine.

## Call Center States

### 1. **Queued** State
Call in queue waiting for agent.

| Event | Role | Next State | Actions | Guards | Conditions Set |
|-------|------|------------|---------|--------|----------------|
| **AssignToAgent** | Both | AgentRinging | - RouteToAgent<br>- NotifyAgent | None | None |
| **QueueTimeout** | Both | Terminating | - RemoveFromQueue<br>- RouteToVoicemail | None | None |
| **HangupCall** | Both | Terminating | - RemoveFromQueue<br>- SendBYE | None | None |

### 2. **AgentRinging** State
Ringing at agent endpoint.

| Event | Role | Next State | Actions | Guards | Conditions Set |
|-------|------|------------|---------|--------|----------------|
| **AgentAccept** | Both | Active | - BridgeToAgent<br>- StartCallRecording | None | InQueue: false |
| **AgentNoAnswer** | Both | Queued | - ReleaseAgent<br>- FindNextAgent | None | None |
| **HangupCall** | Both | Terminating | - SendBYE<br>- ReleaseAgent | None | None |

### 3. **WrapUp** State
Agent in post-call work.

| Event | Role | Next State | Actions | Guards | Conditions Set |
|-------|------|------------|---------|--------|----------------|
| **CompleteWrapUp** | Both | Idle | - SaveCallNotes<br>- UpdateAgentStats | None | AgentAvailable: true |
| **WrapUpTimeout** | Both | Idle | - ForceCompleteWrapUp<br>- UpdateAgentStats | None | AgentAvailable: true |

## Call Center Events

### Queue Management Events
- **QueueCall** - Add incoming call to queue
- **AssignToAgent** - Assign queued call to available agent
- **QueueTimeout** - Call waited too long in queue

### Agent Events  
- **AgentAccept** - Agent accepts assigned call
- **AgentNoAnswer** - Agent didn't answer in time
- **CompleteWrapUp** - Agent completes post-call work
- **WrapUpTimeout** - Wrap-up time exceeded

## Call Center Actions

### Queue Actions
- **AddToQueue** - Add call to queue with priority
- **RemoveFromQueue** - Remove call from queue
- **RouteToVoicemail** - Send to voicemail on timeout
- **PlayQueueMusic** - Play hold music while queued

### Agent Actions
- **RouteToAgent** - Route call to specific agent
- **NotifyAgent** - Alert agent of incoming call
- **ReleaseAgent** - Free agent for next call
- **FindNextAgent** - Find next available agent

### Call Management Actions
- **BridgeToAgent** - Connect customer to agent
- **StartCallRecording** - Begin recording for quality
- **SaveCallNotes** - Save agent's wrap-up notes
- **UpdateAgentStats** - Update agent metrics
- **ForceCompleteWrapUp** - Auto-complete wrap-up on timeout

## Implementation Notes

### Architecture Considerations

1. **Multi-Session Coordination**: Call center operations require coordinating multiple sessions:
   - Customer leg
   - Agent leg
   - Potential supervisor monitoring
   - Conference bridges for coaching

2. **Queue Management**: Requires external state beyond individual sessions:
   - Queue priorities and strategies (FIFO, skills-based, etc.)
   - Agent availability tracking
   - Real-time queue statistics

3. **Agent State Machine**: Agents have their own state machine:
   - Available
   - Ringing
   - On Call
   - Wrap Up
   - Break
   - Offline

4. **Integration Points**:
   - Session-core: For individual call leg management
   - Database: For queue persistence and agent state
   - Events: For real-time updates and monitoring
   - External systems: CRM, workforce management, etc.

### Example Flow

```rust
// Pseudo-code for call center flow in call-engine

// 1. Incoming call arrives
let customer_session = session_core.create_session(incoming_call).await?;

// 2. Add to queue
let queue_position = queue_manager.add_call(customer_session.id, priority).await?;

// 3. Find available agent
let agent = agent_manager.find_available_agent(skills_required).await?;

// 4. Create agent session
let agent_session = session_core.create_outbound_session(agent.extension).await?;

// 5. Bridge when both answer
bridge_manager.bridge_sessions(customer_session.id, agent_session.id).await?;

// 6. Handle wrap-up
agent_manager.start_wrap_up(agent.id, call_id).await?;
```

### Database Schema Considerations

```sql
-- Queue table
CREATE TABLE call_queue (
    id UUID PRIMARY KEY,
    session_id UUID NOT NULL,
    priority INTEGER DEFAULT 0,
    enqueued_at TIMESTAMP NOT NULL,
    skills_required JSONB,
    queue_name VARCHAR(255)
);

-- Agent state table
CREATE TABLE agent_state (
    agent_id UUID PRIMARY KEY,
    current_state VARCHAR(50) NOT NULL,
    available_at TIMESTAMP,
    skills JSONB,
    current_session_id UUID
);

-- Call records for reporting
CREATE TABLE call_records (
    id UUID PRIMARY KEY,
    customer_session_id UUID,
    agent_session_id UUID,
    queue_time_seconds INTEGER,
    talk_time_seconds INTEGER,
    wrap_up_time_seconds INTEGER,
    recording_url VARCHAR(255),
    notes TEXT
);
```

## Benefits of Call-Engine Implementation

1. **Separation of Concerns**: Session-core remains focused on SIP session management
2. **Scalability**: Queue and agent management can scale independently
3. **Flexibility**: Different queue strategies can be implemented without touching session logic
4. **Integration**: Easier to integrate with external call center systems
5. **Reporting**: Centralized location for call center metrics and analytics

## Migration Path

To implement these features in call-engine:

1. Create agent management module
2. Implement queue management with pluggable strategies
3. Add bridge coordination for multi-party calls
4. Implement supervisor features (monitoring, coaching, barging)
5. Add reporting and analytics
6. Create REST/gRPC APIs for external integration

This design preserves the original concepts while acknowledging that call center operations belong at a higher orchestration layer than individual session management.
