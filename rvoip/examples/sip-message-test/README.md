# SIP Message Test Example

This example demonstrates and tests all SIP message types and the transaction state machine implementation in the rvoip stack. It consists of a client and server that exchange various SIP messages, simulating a complete SIP call flow and other SIP transactions.

## Features Demonstrated

- SIP message creation and parsing for all supported methods
- SIP transaction state machine (client and server transactions)
- Transaction layer timers and retransmission handling
- Provisional and final responses
- Dialog establishment and termination
- UDP transport for SIP messages

## SIP Methods Tested

- INVITE: Call setup
- ACK: INVITE response confirmation
- BYE: Call termination
- REGISTER: Registration with a SIP registrar
- OPTIONS: Capability query
- SUBSCRIBE: Event subscription
- MESSAGE: Instant messaging
- UPDATE: Session update

## How to Run

First, start the server in one terminal:

```bash
cargo run --bin rvoip-sip-message-test -- server
```

Then, start the client in another terminal:

```bash
cargo run --bin rvoip-sip-message-test -- client
```

The client will initiate various SIP transactions with the server, and both sides will log the progress of each transaction.

## Example Output

The server will output log messages as it receives and processes different SIP requests, including:
- Receiving INVITE and sending 100 Trying, 180 Ringing, and 200 OK
- Receiving ACK for the INVITE
- Processing various other method requests
- Receiving BYE and terminating the call

The client will output log messages as it sends requests and receives responses, including:
- Sending INVITE and receiving provisional and final responses
- Sending ACK for the 200 OK
- Sending BYE and receiving 200 OK
- Sending other method requests and receiving responses

## Implementation Details

This example tests the core functionality of:
- `rvoip-sip-core`: SIP message types, parsing, and serialization
- `rvoip-sip-transport`: UDP transport for SIP messages
- `rvoip-transaction-core`: SIP transaction state machine

It demonstrates how these components work together to provide a complete implementation of the SIP protocol transaction layer as specified in RFC 3261. 