# Comparison with Production VoIP Systems

## How Asterisk Manages State

### Architecture: Channel Driver Model
Asterisk uses a **channel-centric** architecture where each call leg is a "channel" with its own state machine.

```c
// Asterisk channel states (simplified)
enum ast_channel_state {
    AST_STATE_DOWN,           // Channel is down
    AST_STATE_RESERVED,       // Channel is reserved
    AST_STATE_OFFHOOK,        // Channel is off hook
    AST_STATE_DIALING,        // Digits (or equivalent) have been dialed
    AST_STATE_RING,           // Line is ringing
    AST_STATE_RINGING,        // Remote end is ringing
    AST_STATE_UP,             // Line is up
    AST_STATE_BUSY,           // Line is busy
    AST_STATE_DIALING_OFFHOOK, // Digits have been dialed while offhook
    AST_STATE_PRERING,        // Channel has detected an incoming call
};
```

### State Management
- **Channel Driver**: Each protocol (SIP, IAX2, PJSIP) has its own driver
- **State Callbacks**: Drivers register callbacks for state transitions
- **Bridge Module**: Handles connecting two channels (legs) together
- **Dialplan**: External configuration drives the state machine

```c
// Simplified Asterisk channel callback structure
static struct ast_channel_tech sip_tech = {
    .type = "SIP",
    .requester = sip_request_call,
    .call = sip_call,
    .hangup = sip_hangup,
    .answer = sip_answer,
    .indicate = sip_indicate,
    .fixup = sip_fixup,
    .send_digit_begin = sip_senddigit_begin,
    // ... many more callbacks
};
```

### Key Differences from Master Table
1. **Callback-driven** vs Table-driven
2. **Per-protocol handlers** vs Unified state machine
3. **Runtime registration** vs Compile-time table
4. **Mutable state** vs Immutable transitions

## How FreeSWITCH Manages State

### Architecture: Core State Machine
FreeSWITCH uses a **centralized state machine** with a core that's protocol-agnostic.

```c
// FreeSWITCH channel states
typedef enum {
    CS_NEW,           // Channel is newly created
    CS_INIT,          // Channel is initializing
    CS_ROUTING,       // Channel is looking for an extension
    CS_SOFT_EXECUTE,  // Channel is executing a soft action
    CS_EXECUTE,       // Channel is executing an application
    CS_EXCHANGE_MEDIA, // Channel is exchanging media
    CS_PARK,          // Channel is parked
    CS_CONSUME_MEDIA, // Channel is consuming media
    CS_HIBERNATE,     // Channel is hibernating
    CS_RESET,         // Channel is resetting
    CS_HANGUP,        // Channel is hanging up
    CS_REPORTING,     // Channel is reporting
    CS_DESTROY,       // Channel is being destroyed
} switch_channel_state_t;
```

### State Machine Implementation
FreeSWITCH uses a **function pointer table** that's very similar to our proposed master table:

```c
// Simplified FreeSWITCH state handler table
static switch_state_handler_table_t state_handlers = {
    .on_init = my_on_init,
    .on_routing = my_on_routing,
    .on_execute = my_on_execute,
    .on_hangup = my_on_hangup,
    .on_exchange_media = my_on_exchange_media,
    .on_soft_execute = my_on_soft_execute,
    .on_consume_media = my_on_consume_media,
    .on_hibernate = my_on_hibernate,
    .on_reset = my_on_reset,
    .on_park = my_on_park,
    .on_reporting = my_on_reporting,
    .on_destroy = my_on_destroy
};
```

### Threading Model
- **One thread per channel** initially
- **Thread pool** for media processing
- **Event queue** for async operations
- **State machine runs in channel thread**

## How Kamailio/OpenSIPS Manage State

### Architecture: Transaction Stateful Proxy
These are **SIP proxies** not B2BUAs, so they manage state differently:

```c
// Transaction state machine (simplified)
enum {
    TS_UNDEFINED = 0,
    TS_TRYING,
    TS_PROCEEDING,
    TS_COMPLETED,
    TS_CONFIRMED,
    TS_TERMINATED
};

// Dialog state tracking
struct dlg_cell {
    unsigned int h_id;
    unsigned int state;
    unsigned int lifetime;
    unsigned int init_ts;
    struct dlg_callback *cbs;
    // ... profile data
};
```

### Key Approach: Modular Callbacks
```c
// Route script drives the state machine
route {
    if (is_method("INVITE")) {
        if (!has_sdp()) {
            sl_send_reply("488", "Not Acceptable Here");
            exit;
        }
        route(INVITE_PROCESSING);
    }
    // ... more routing logic
}
```

## How PJSIP (Library) Manages State

### Architecture: Layered State Machines
PJSIP has **separate state machines** for each layer:

```c
// INVITE session state
typedef enum pjsip_inv_state {
    PJSIP_INV_STATE_NULL,
    PJSIP_INV_STATE_CALLING,
    PJSIP_INV_STATE_INCOMING,
    PJSIP_INV_STATE_EARLY,
    PJSIP_INV_STATE_CONNECTING,
    PJSIP_INV_STATE_CONFIRMED,
    PJSIP_INV_STATE_DISCONNECTED,
} pjsip_inv_state;

// Media state (separate)
typedef enum pjmedia_state {
    PJMEDIA_STATE_NULL,
    PJMEDIA_STATE_CREATED,
    PJMEDIA_STATE_LOCAL_OFFER,
    PJMEDIA_STATE_REMOTE_OFFER,
    PJMEDIA_STATE_NEGOTIATED,
    PJMEDIA_STATE_RUNNING,
} pjmedia_state;
```

### Callback-Based Coordination
```c
// Application registers callbacks
static pjsip_inv_callback inv_cb = {
    .on_state_changed = &on_call_state,
    .on_new_session = &on_call_forked,
    .on_media_update = &on_call_media_state,
    .on_rx_offer = &on_rx_offer,
    .on_create_offer = &on_create_offer,
    // ... more callbacks
};
```

## Comparison Table

| System | Architecture | State Management | Pros | Cons |
|--------|-------------|------------------|------|------|
| **Asterisk** | Channel drivers + callbacks | Per-protocol state machines | Flexible, extensible | Complex, hard to reason about |
| **FreeSWITCH** | Core state machine + handlers | Centralized with function pointers | Clean separation, predictable | Monolithic, harder to extend |
| **Kamailio** | Transaction stateful | Script-driven routing | Highly configurable | Not full B2BUA, limited media |
| **PJSIP** | Layered state machines | Callback coordination | Modular, reusable | Complex callback chains |
| **Master Table** | Single lookup table | Declarative transitions | Simple, verifiable | Less flexible, static |

## How They Handle Complex Scenarios

### Race Condition: Media Before SDP
**Asterisk**: Uses flags and mutexes
```c
if (ast_test_flag(&p->flags[0], SIP_PENDING_INVITE)) {
    ast_queue_control(p->owner, AST_CONTROL_PROGRESS);
}
```

**FreeSWITCH**: State machine prevents it
```c
if (channel->state < CS_EXCHANGE_MEDIA) {
    return SWITCH_STATUS_NOT_READY;
}
```

**Master Table**: Impossible by design
```rust
// No transition exists for (Initiating, StartMedia, _)
```

### Edge Case: Simultaneous Actions
**Asterisk**: Locking and priority
```c
ast_channel_lock(chan);
// ... do work
ast_channel_unlock(chan);
```

**FreeSWITCH**: Serial event queue
```c
switch_event_fire(&event); // Queued and processed serially
```

**Master Table**: Deterministic order
```rust
// Events processed in defined order
```

## Why Production Systems Don't Use Pure State Tables

### 1. **Historical Evolution**
- Asterisk (1999): Started simple, grew organically
- FreeSWITCH (2006): Learned from Asterisk, but kept compatibility needs
- Standards evolved over time (SIP had 150+ RFCs)

### 2. **Flexibility Requirements**
- Need to support non-standard behavior
- Customer-specific modifications
- Protocol extensions and variants

### 3. **Performance Considerations**
```c
// Direct function call (Asterisk/FreeSWITCH)
tech->answer(channel);  // ~50ns

// vs Table lookup
transition = table[state][event];  // ~100-200ns with cache miss
execute(transition);
```

### 4. **Dynamic Configuration**
Production systems need runtime changes:
- Dialplan reloading
- Module loading/unloading
- Configuration without restart

## Where State Tables Are Used

### 1. **Protocol Stacks**
TCP/IP implementations often use state tables:
```c
// Linux TCP state table (simplified)
static int tcp_state_table[TCP_MAX_STATES][TCP_MAX_EVENTS] = {
    [TCP_ESTABLISHED] = {
        [TCP_EV_RCV_FIN] = TCP_CLOSE_WAIT,
        [TCP_EV_RCV_RST] = TCP_CLOSED,
        // ...
    },
    // ...
};
```

### 2. **Embedded Systems**
Where predictability > flexibility:
- Hardware SIP phones
- VoIP gateways
- Media servers

### 3. **Formal Verification**
Academic and high-reliability systems:
- Aerospace communications
- Emergency services (E911)
- Military systems

## Hybrid Approach (Best of Both)

Modern systems are moving toward a hybrid:

```rust
// Core state table for standard flows
let standard_transition = STATE_TABLE.get(state, event);

// Plugin system for extensions
let plugin_result = plugin_chain.process(state, event);

// Combine results
match (standard_transition, plugin_result) {
    (Some(t), None) => execute(t),
    (None, Some(p)) => execute_plugin(p),
    (Some(t), Some(p)) => execute_with_override(t, p),
    (None, None) => log_unhandled(state, event),
}
```

## Recommendations for Your System

### Use Master Table For:
1. **Core SIP flows** (INVITE, 200 OK, ACK, BYE)
2. **Standard states** (Initiating, Ringing, Active, Terminated)
3. **Critical timing** (MediaFlowEstablished events)
4. **Verification** (Prove no deadlocks or race conditions)

### Keep Flexible For:
1. **Custom headers** and extensions
2. **Non-standard codecs** or media types
3. **Advanced features** (conference, transfer, forward)
4. **Debugging hooks** and monitoring

### Implementation Strategy:
```rust
pub struct HybridStateMachine {
    // Core state table (immutable, verified)
    core_table: &'static MasterStateTable,
    
    // Dynamic extensions
    extensions: Vec<Box<dyn StateExtension>>,
    
    // Override rules
    overrides: HashMap<(State, Event), OverrideAction>,
}
```

## Conclusion

Production VoIP systems generally **don't use pure state tables** because:
1. **Legacy**: They evolved before formal methods were common
2. **Flexibility**: They need to handle non-standard scenarios
3. **Performance**: Function pointers are marginally faster
4. **Politics**: Standards committees compromise led to complex specs

However, your **master state table approach is actually more modern** and similar to:
- How FreeSWITCH's core works (function pointer table)
- How modern protocol stacks are built (state tables)
- How formally verified systems are designed

The key insight: **You're building a greenfield system**, so you can use modern approaches without legacy constraints. The master state table gives you:
- **Correctness** (easier to verify)
- **Simplicity** (easier to understand)
- **Predictability** (easier to debug)

This is actually how many people **wish** Asterisk or FreeSWITCH worked internally!