# Example 3: SIP Dialog Example

This example demonstrates a complete SIP dialog between two User Agents, including dialog establishment, management, and termination. It simulates a basic call flow with proper dialog state tracking.

## What You'll Learn

- How to establish, maintain, and terminate SIP dialogs
- The complete call flow for a SIP session (INVITE → 200 OK → ACK → BYE → 200 OK)
- How to track dialog state according to RFC specifications
- How to handle tags, Call-IDs, and sequence numbers
- How to create and respond to in-dialog requests
- How User Agents maintain dialog state on both sides

## Running the Example

```bash
# Run the example
cargo run --example 03_sip_dialog

# Run with debug logs to see the actual SIP messages
RUST_LOG=debug cargo run --example 03_sip_dialog
```

## Code Walkthrough

The example implements a complete SIP dialog simulation:

1. **User Agent Implementation**
   - Maintains dialog state and handles SIP message creation/processing
   - Tracks active dialogs using Call-ID as the key
   - Manages sequence numbers and generates unique identifiers

2. **Dialog State Management**
   - Creates dialog state from INVITE/200 OK exchange
   - Updates dialog based on received messages
   - Tracks local and remote sequence numbers
   - Properly handles tags in From/To headers

3. **Complete Call Flow**
   - **Dialog Establishment**:
     - Alice sends INVITE to Bob
     - Bob responds with 180 Ringing and 200 OK
     - Alice acknowledges with ACK
   - **Dialog Termination**:
     - Alice sends BYE
     - Bob sends 200 OK
     - Both sides clean up dialog state

4. **Message Processing**
   - Proper extraction of headers for dialog identification
   - Creating responses that maintain dialog state
   - Handling in-dialog requests (ACK, BYE)

## Key Concepts

### SIP Dialog Establishment

A SIP dialog is established through a three-way handshake:
1. User Agent Client (UAC) sends INVITE
2. User Agent Server (UAS) responds with a final response (typically 200 OK)
3. UAC sends ACK

The dialog is identified by the combination of:
- Call-ID
- From tag (local tag for the initiator)
- To tag (remote tag for the initiator)

### Dialog State

Both participants maintain dialog state, which includes:
- Dialog identifiers (Call-ID, local tag, remote tag)
- Local and remote URIs
- Remote target (where to send requests)
- Sequence numbers for both sides

### Creating In-Dialog Requests

In-dialog requests must:
- Use the remote target as the Request-URI
- Include the proper From/To tags that identify the dialog
- Use an incremented sequence number
- Reuse the Call-ID

### Dialog Termination

A dialog is terminated when:
- A BYE request is sent and acknowledged with 200 OK
- Both sides remove the dialog state

## Next Steps

Once you understand SIP dialogs, you can move on to Example 4 which covers SDP (Session Description Protocol) integration for media negotiation. 