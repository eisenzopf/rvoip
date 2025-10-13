# Blind Transfer Example - Callback-Based Approach

This example demonstrates the new callback-based approach to handling SIP REFER requests in session-core-v3. Unlike the previous automatic transfer system, this approach gives developers full control over transfer handling through callbacks.

## Key Changes from v2

- **Single Session Constraint**: Each peer can only have one active session at a time
- **Callback-Based REFER Handling**: REFER requests are handled via registered callbacks
- **Manual Session Management**: Developers manually terminate and create sessions during transfers
- **No Automatic Transfer Logic**: All transfer logic is explicit and developer-controlled

## Architecture

```
Alice (Peer1)          Bob (Peer2)           Charlie (Peer3)
    |                      |                       |
    |------ INVITE ------->|                       |
    |<----- 200 OK --------|                       |
    |------ ACK ---------->|                       |
    |                      |                       |
    |<===== RTP Audio ====>|                       |
    |                      |                       |
    |<----- REFER ---------|  (to Charlie)         |
    |------ 202 --------->|                       |
    |                      |                       |
    | [Callback Triggered] |                       |
    | [Terminate Session]  |                       |
    |                      |                       |
    |------------- INVITE ---------------->|
    |<------------ 200 OK -----------------|
    |------------- ACK ------------------>|
    |                      |                       |
    |<========== RTP Audio ===============>|
```

## How It Works

### 1. Alice (Caller/Transferee)
- Makes initial call to Bob
- Registers `on_refer` callback to handle transfer requests
- When REFER is received:
  - Callback is invoked with transfer details
  - Returns `CallbackResult::Accept` to accept the transfer
  - Manually terminates current session
  - Manually creates new session to transfer target (Charlie)

### 2. Bob (Transferor)
- Receives call from Alice
- Initiates transfer by sending REFER message
- In this example, simulated by hanging up (real implementation would send REFER)

### 3. Charlie (Transfer Target)
- Waits for incoming call from Alice (after transfer)
- Accepts and handles the transferred call normally

## Running the Example

1. **Terminal 1 - Start Charlie (Transfer Target)**:
   ```bash
   cd crates/session-core-v3
   cargo run --example blind_transfer_peer3_target
   ```

2. **Terminal 2 - Start Bob (Transferor)**:
   ```bash
   cd crates/session-core-v3
   cargo run --example blind_transfer_peer2_transferor
   ```

3. **Terminal 3 - Start Alice (Caller)**:
   ```bash
   cd crates/session-core-v3
   cargo run --example blind_transfer_peer1_caller
   ```

Or use the provided script:
```bash
cd crates/session-core-v3/examples/blind_transfer
./run_blind_transfer.sh
```

## Code Example

### Registering REFER Callback

```rust
use rvoip_session_core_v3::api::simple::{SimplePeer, CallbackResult, ReferEvent};

let alice = SimplePeer::new("alice").await?;

// Register callback to handle REFER requests
alice.on_refer(|refer_event: ReferEvent| {
    println!("Received REFER to: {}", refer_event.refer_to);
    
    // Developer decides how to handle the transfer
    if refer_event.refer_to.contains("charlie") {
        CallbackResult::Accept  // Accept and handle manually
    } else {
        CallbackResult::Reject(603, "Decline")  // Reject unwanted transfers
    }
}).await?;
```

### Manual Transfer Handling

```rust
// After callback returns Accept, manually handle the transfer:

// 1. Terminate current session
alice.terminate_current_session().await?;

// 2. Create new session to transfer target
let new_call_id = alice.call("sip:charlie@example.com").await?;

// 3. Continue with new session
println!("Transfer complete! Now talking to Charlie");
```

## Benefits of Callback Approach

1. **Developer Control**: Full control over transfer logic
2. **Flexibility**: Can implement custom transfer policies
3. **Simplicity**: No complex automatic transfer state machine
4. **Transparency**: Transfer handling is explicit and visible
5. **Single Session**: Enforces clean session management

## Callback Results

- `CallbackResult::Accept`: Accept REFER and handle transfer manually
- `CallbackResult::Reject(code, reason)`: Reject REFER with specific SIP response
- `CallbackResult::Handle`: Callback handled everything (sent custom response)

## Audio Output

The example generates audio files in `examples/blind_transfer/output/`:
- `alice_sent.wav`: Audio sent by Alice
- `alice_received.wav`: Audio received by Alice  
- `bob_sent.wav`: Audio sent by Bob
- `charlie_sent.wav`: Audio sent by Charlie

Each peer generates a different frequency sine wave for identification:
- Alice: 440 Hz (A4)
- Bob: 880 Hz (A5) 
- Charlie: 660 Hz (E5)

## Troubleshooting

- Ensure all three peers are started in the correct order
- Check that ports 5060, 5061, and 5062 are available
- Enable debug logging with `RUST_LOG=debug` for detailed output
- Audio files help verify the transfer worked correctly

## Next Steps

This example demonstrates the basic callback approach. In a production system, you might:

1. **Implement proper REFER sending**: Use dialog adapter to send actual REFER messages
2. **Add error handling**: Handle transfer failures gracefully  
3. **Support attended transfers**: Implement consultation call callbacks
4. **Add transfer policies**: Implement business logic for when to accept/reject transfers
5. **Integrate with UI**: Connect callbacks to user interface for transfer decisions