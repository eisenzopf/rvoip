# Simple Call Engine Example

This example demonstrates the functionality of the RVOIP Call Engine, showcasing:

- SIP message handling and routing
- Session management
- Basic call flow (REGISTER, INVITE, BYE)
- Policy enforcement

## Building the Example

From the repository root, run:

```bash
cargo build --release -p simple-call-engine
```

## Running the Example

Start the call engine:

```bash
cargo run --release -p simple-call-engine
```

By default, the server will bind to `0.0.0.0:5060` for SIP signaling. You can specify different addresses using command-line arguments:

```bash
cargo run --release -p simple-call-engine -- --sip-addr 127.0.0.1:5060 --media-addr 127.0.0.1:10000 --domain rvoip.local
```

## Testing the Example

### Manual Testing

Once the server is running, you can test it using simple `nc` (netcat) commands:

1. Send a REGISTER request:

```bash
echo -n "REGISTER sip:rvoip.local SIP/2.0
Via: SIP/2.0/UDP 127.0.0.1:5070;branch=z9hG4bK-524287-1
Max-Forwards: 70
From: <sip:alice@rvoip.local>;tag=12345
To: <sip:alice@rvoip.local>
Call-ID: register-alice-1@localhost
CSeq: 1 REGISTER
Contact: <sip:alice@127.0.0.1:5070>
Expires: 3600
User-Agent: RVOIP-Test-Client/0.1.0
Content-Length: 0

" | nc -u 127.0.0.1 5060
```

2. Send an OPTIONS request:

```bash
echo -n "OPTIONS sip:rvoip.local SIP/2.0
Via: SIP/2.0/UDP 127.0.0.1:5070;branch=z9hG4bK-524287-1
Max-Forwards: 70
From: <sip:alice@rvoip.local>;tag=12345
To: <sip:rvoip.local>
Call-ID: options-test-1@localhost
CSeq: 1 OPTIONS
User-Agent: RVOIP-Test-Client/0.1.0
Content-Length: 0

" | nc -u 127.0.0.1 5060
```

### Automated Call Flow Test

The example includes a shell script that simulates a complete call flow between two endpoints:

```bash
# From the repository root
./examples/simple-call-engine/test_call.sh

# Or from the example directory
cd examples/simple-call-engine
./test_call.sh
```

This script:

1. Registers two endpoints (Alice and Bob)
2. Initiates a call from Alice to Bob
3. Simulates Bob answering the call
4. Establishes a dialog
5. Terminates the call

The script captures and displays all SIP messages exchanged during the call.

## Understanding the Code

The example demonstrates key components of the RVOIP stack:

- `CallEngine`: Central component coordinating call handling
- `Registry`: Manages SIP endpoint registrations
- `Router`: Handles message routing based on SIP URIs
- `PolicyEngine`: Enforces security and access policies

## Architecture

```
┌─────────────────────────┐
│      UDP Transport      │
└─────────────┬───────────┘
              │
┌─────────────▼───────────┐
│  Transaction Manager    │
└─────────────┬───────────┘
              │
┌─────────────▼───────────┐
│      Call Engine        │
└─────────────┬───────────┘
              │
┌─────────────▼───────────┐
│    Session Manager      │
└─────────────────────────┘
```

This example provides a foundation that can be extended to implement a full-featured SIP server or softswitch. 