# RVOIP Examples

This directory contains examples demonstrating various features of the RVOIP library.

## Available Examples

- [simple-session](#simple-session) - A basic SIP session example
- [rtp-loopback](#rtp-loopback) - RTP packet loopback demonstration
- [rtp-g711](#rtp-g711) - G.711 codec implementation with RTP
- [sip-message-test](#sip-message-test) - Test for SIP message types and transactions
- [simple-softswitch](#simple-softswitch) - Basic softswitch implementation

## Running the Examples

All examples are configured as separate crates within the workspace. To run them, navigate to the example directory and use cargo.

### simple-session

A basic example that demonstrates how to establish a SIP session.

```bash
cd rvoip/examples/simple-session
cargo run -- info
```

Example with help output:
```bash
cd rvoip/examples/simple-session
cargo run -- help
```

### rtp-loopback

Demonstrates the use of the `rtp-core` crate for sending and receiving RTP packets in a loopback configuration.

Basic usage:
```bash
cd rvoip/examples/rtp-loopback
cargo run
```

#### Example Commands:

With RTCP enabled:
```bash
cd rvoip/examples/rtp-loopback
cargo run -- --rtcp
```

Custom sender and receiver addresses:
```bash
cd rvoip/examples/rtp-loopback
cargo run -- -s 127.0.0.1:11000 -r 127.0.0.1:11001
```

Send 20 packets with 50ms interval:
```bash
cd rvoip/examples/rtp-loopback
cargo run -- -c 20 -i 50
```

Set payload type to 8 (PCMA/G.711 A-law):
```bash
cd rvoip/examples/rtp-loopback
cargo run -- -p 8
```

Combined options:
```bash
cd rvoip/examples/rtp-loopback
cargo run -- --rtcp -s 127.0.0.1:11000 -r 127.0.0.1:11001 -c 20 -i 50 -p 8
```

#### Available Options:

- `-s, --sender-addr <ADDR>`: Local address for the sender (default: 127.0.0.1:10000)
- `-r, --receiver-addr <ADDR>`: Local address for the receiver (default: 127.0.0.1:10001)
- `-c, --count <COUNT>`: Number of packets to send (default: 10)
- `-i, --interval <INTERVAL>`: Interval between packets in milliseconds (default: 100)
- `-p, --payload-type <PT>`: Payload type (default: 0)
- `--rtcp`: Enable RTCP

### rtp-g711

This example shows how to use the G.711 codec with RTP for audio encoding and decoding.

Basic usage:
```bash
cd rvoip/examples/rtp-g711
cargo run
```

If the example supports options, you can view them with:
```bash
cd rvoip/examples/rtp-g711
cargo run -- --help
```

### sip-message-test

Demonstrates and tests all SIP message types and the transaction state machine implementation in the RVOIP stack.

Start the server in one terminal:
```bash
cd rvoip/examples/sip-message-test
cargo run -- server
```

Then, start the client in another terminal:
```bash
cd rvoip/examples/sip-message-test
cargo run -- client
```

View available options:
```bash
cd rvoip/examples/sip-message-test
cargo run -- --help
```

This example tests various SIP methods including:
- INVITE
- ACK
- BYE
- REGISTER
- OPTIONS
- SUBSCRIBE
- MESSAGE
- UPDATE

### simple-softswitch

A basic softswitch implementation that demonstrates how to route calls between multiple endpoints.

Basic usage:
```bash
cd rvoip/examples/simple-softswitch
cargo run
```

View available options:
```bash
cd rvoip/examples/simple-softswitch
cargo run -- --help
```

## Building the Examples

To build all examples and crates in the workspace:

```bash
cd rvoip
cargo build
```

To build a specific example:

```bash
cd rvoip/examples/<example-name>
cargo build
```

## Documentation

Each example directory contains additional documentation specific to that example. Refer to the README.md file in each example directory for more detailed information. 