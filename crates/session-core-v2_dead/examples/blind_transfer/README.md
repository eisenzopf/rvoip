# Blind Transfer Example

This example demonstrates blind call transfer functionality in session-core-v2 using three peers.

## Scenario

1. **Alice (Peer1)** calls **Bob (Peer2)** at `sip:bob@127.0.0.1:5061`
2. Alice and Bob talk for a few seconds
3. **Bob** initiates a blind transfer to **Charlie (Peer3)** at `sip:charlie@127.0.0.1:5062`
4. Alice's call is transferred to Charlie
5. Alice and Charlie are now connected, Bob drops out

## Files

- `peer1_caller.rs` - Alice (caller on port 5060)
- `peer2_transferor.rs` - Bob (receives call, performs transfer on port 5061)
- `peer3_target.rs` - Charlie (transfer target on port 5062)
- `run_blind_transfer.sh` - Test orchestration script

## Running the Example

### Quick Start

```bash
cd crates/session-core-v2/examples/blind_transfer
./run_blind_transfer.sh
```

### With Debug Logging

```bash
./run_blind_transfer.sh --debug
```

### With Trace Logging

```bash
./run_blind_transfer.sh --trace
```

## How It Works

### SIP Blind Transfer (REFER Method)

1. Alice establishes a call with Bob (INVITE → 200 OK → ACK)
2. Bob sends a **REFER** message to Alice:
   ```
   REFER sip:alice@127.0.0.1:5060 SIP/2.0
   Refer-To: sip:charlie@127.0.0.1:5062
   ```
3. Alice receives the REFER and:
   - Sends `202 Accepted` to Bob
   - Initiates a new INVITE to Charlie
   - Sends NOTIFY to Bob about transfer progress
4. Bob's call with Alice terminates (BYE)
5. Alice establishes a new call with Charlie
6. Alice and Charlie are now connected

### Code Flow

**Peer1 (Alice):**
- Calls Bob
- Receives REFER from Bob (handled by state machine)
- Automatically initiates new call to Charlie
- Continues call with Charlie

**Peer2 (Bob):**
- Accepts incoming call from Alice
- Waits a bit
- Calls `transfer(&call_id, "sip:charlie@127.0.0.1:5062")`
- Terminates after transfer initiated

**Peer3 (Charlie):**
- Waits for incoming call
- Accepts the call from Alice (transferred)
- Talks with Alice

## Expected Output

```
🔄 Session-Core-V2 Blind Transfer Test
======================================

▶️  Starting Charlie (peer3 - transfer target) on port 5062...
▶️  Starting Bob (peer2 - transferor) on port 5061...
▶️  Starting Alice (peer1 - caller) on port 5060...

[CHARLIE] Starting - Will receive transferred call from Alice...
[CHARLIE] ✅ Listening on port 5062...
[BOB] Starting - Will receive call from Alice and transfer to Charlie...
[BOB] ✅ Listening on port 5061...
[ALICE] Starting - Will call Bob and be transferred to Charlie...
[ALICE] 📞 Calling Bob at sip:bob@127.0.0.1:5061...
[BOB] 📞 Received call from Alice!
[BOB] 💬 Talking to Alice...
[BOB] 🔄 Initiating blind transfer to Charlie...
[CHARLIE] 📞 Received transferred call!
[ALICE] 💬 Now talking to Charlie (post-transfer)...

✅ Blind transfer test completed successfully!
```

## Implementation Details

### State Machine

The blind transfer is implemented using the state table in `session-core-v2`:

- **Active** state handles the `BlindTransfer` event
- Transitions to **Transferring** state
- Executes `SendREFER` action
- Sends SIP REFER to the remote party

### SimplePeer API

```rust
// Bob performs the transfer
bob.transfer(&call_id, "sip:charlie@127.0.0.1:5062").await?;
```

This calls the underlying state machine which:
1. Sends REFER to Alice
2. Waits for NOTIFY responses
3. Terminates the call when transfer is accepted

## Testing

The script monitors all three processes and reports:
- Exit codes for each peer
- Timeout detection (30s max)
- Log file locations for debugging

Logs are saved to `logs/` directory with timestamps:
- `alice_YYYYMMDD_HHMMSS.log`
- `bob_YYYYMMDD_HHMMSS.log`
- `charlie_YYYYMMDD_HHMMSS.log`

## Troubleshooting

**Transfer fails:**
- Check that blind transfer is implemented in state table
- Verify REFER handling in dialog-core
- Check logs for SIP message exchange

**Processes hang:**
- Check port availability (5060, 5061, 5062)
- Verify state machine transitions
- Enable trace logging: `./run_blind_transfer.sh --trace`

**Call doesn't transfer:**
- Ensure Alice receives and processes REFER
- Check that new INVITE to Charlie is sent
- Verify dialog termination after REFER
