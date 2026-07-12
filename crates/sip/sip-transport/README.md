# rvoip-sip-transport

[![Crates.io](https://img.shields.io/crates/v/rvoip-sip-transport.svg)](https://crates.io/crates/sip/rvoip-sip-transport)
[![Documentation](https://docs.rs/rvoip-sip-transport/badge.svg)](https://docs.rs/rvoip-sip-transport)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

> **Beta scope notice:** for the `rvoip-sip` beta, transport claims are limited
> to the paths covered by the beta compatibility matrix. UDP is the primary
> release-gated transport; TCP/TLS/WS require their listed tests; WSS outbound
> remains a non-claim until the known incomplete path is finished and tested.

SIP transport layer implementation for the [rvoip](../../../README.md) VoIP stack, providing reliable and efficient transport mechanisms for SIP messages across different network protocols.

## Overview

`rvoip-sip-transport` is the transport layer of the rvoip stack that handles the reliable transmission and reception of SIP messages over various network protocols. It abstracts away the complexities of different transport types while providing a unified interface for higher-level SIP components.

## Features

### ✅ Completed Features

- **Multiple Transport Types**
  - ✅ UDP transport with connection-less messaging
  - ✅ TCP transport with connection management and message framing
  - ✅ TLS transport with secure encrypted communication
  - ✅ WebSocket transport with RFC 7118 compliance
  - ⚠️ Secure WebSocket (WSS) listener/lower-level support where tests pass; outbound WSS dialing is not a beta claim

- **Transport Management**
  - ✅ Unified `Transport` trait for all transport types
  - ✅ Transport factory for URI-based transport selection
  - ✅ Centralized transport manager with destination routing
  - ✅ Connection pooling and reuse for TCP/TLS transports
  - ✅ Automatic connection lifecycle management

- **Error Handling & Reliability** 
  - ✅ Comprehensive error types with categorization
  - ✅ Recoverable vs non-recoverable error classification
  - ✅ Connection timeout and keepalive mechanisms
  - ✅ Proper resource cleanup for terminated connections

- **Performance Optimizations**
  - ✅ Optimized buffer management to reduce allocations
  - ✅ Flow control for stream-based transports
  - ✅ Efficient message framing for TCP/TLS
  - ✅ Zero-copy techniques where possible

- **Integration**
  - ✅ Seamless integration with `transaction-core`
  - ✅ Event-driven architecture with `TransportEvent`
  - ✅ Compatible with rvoip's layered architecture

### 🚧 Planned Features

- **Enhanced Management**
  - 🚧 Transport failover capabilities
  - 🚧 Load balancing for outgoing connections
  - 🚧 Transport monitoring and health checks
  - 🚧 RFC 3263 procedures for SIP server location

- **Scalability Improvements**
  - 🚧 Backpressure mechanisms for high traffic
  - 🚧 Throttling capabilities
  - 🚧 Enhanced connection limit management

- **Event System Integration**
  - 🚧 Integration with infra-common event bus
  - 🚧 Priority-based transport event processing
  - 🚧 High-throughput event optimization

## Architecture

### Transport Trait

All transport implementations share a common `Transport` trait:

```rust
#[async_trait::async_trait]
pub trait Transport: Send + Sync + fmt::Debug {
    fn local_addr(&self) -> Result<SocketAddr>;
    async fn send_message(&self, message: Message, destination: SocketAddr) -> Result<()>;
    async fn close(&self) -> Result<()>;
    fn is_closed(&self) -> bool;
    // ... additional methods for transport capabilities
}
```

### Transport Types

- **UDP** (`UdpTransport`): Connection-less, best-effort delivery
- **TCP** (`TcpTransport`): Reliable, connection-oriented with message framing
- **TLS** (`TlsTransport`): Secure TCP with encryption and certificate validation
- **WebSocket** (`WebSocketTransport`): Full-duplex communication over HTTP

### Inbound TLS client authentication

SIP TLS and WSS listeners use a server-side client-authentication policy that
is independent from outbound `TlsClientConfig`:

```rust,no_run
use rvoip_sip_transport::transport::tls::{
    TlsServerClientAuthConfig, TlsTransport,
};

let client_auth = TlsServerClientAuthConfig::required("client-ca.pem");
let (transport, events) = TlsTransport::bind_server_only_with_client_auth(
    "0.0.0.0:5061".parse()?,
    "server-cert.pem".as_ref(),
    "server-key.pem".as_ref(),
    None,
    client_auth,
).await?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

`Disabled` is the default and preserves server-only TLS compatibility.
`Optional` verifies a certificate when supplied but permits anonymous clients;
`Required` rejects clients that do not present a trusted certificate. Accepted
client certificates are exposed as a verified SHA-256 leaf fingerprint in
`TransportEvent::MessageReceived::connection_metadata`. WSS exposes the same
policy through `WebSocketTransport::bind_with_tls_configs`.

### Event System

The transport layer emits events through the `TransportEvent` enum:

```rust
pub enum TransportEvent {
    MessageReceived { message: Message, source: SocketAddr, destination: SocketAddr },
    Error { error: String },
    Closed,
}
```

## Usage

### Basic Example

```rust
use rvoip_sip_transport::prelude::*;
use rvoip_sip_core::Message;

#[tokio::main]
async fn main() -> Result<()> {
    // Create a UDP transport
    let (transport, mut events) = bind_udp("127.0.0.1:5060".parse()?).await?;
    
    // Listen for incoming messages
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            match event {
                TransportEvent::MessageReceived { message, source, .. } => {
                    println!("Received message from {}: {}", source, message);
                }
                TransportEvent::Error { error } => {
                    eprintln!("Transport error: {}", error);
                }
                TransportEvent::Closed => {
                    println!("Transport closed");
                    break;
                }
            }
        }
    });
    
    // Send a message
    let message = Message::new_request(/* ... */);
    transport.send_message(message, "127.0.0.1:5061".parse()?).await?;
    
    Ok(())
}
```

### Transport Factory

```rust
use rvoip_sip_transport::factory::TransportFactory;

let factory = TransportFactory::new();

// Create transport based on URI scheme
let (transport, events) = factory
    .create_from_uri("sip:example.com:5060;transport=tcp")
    .await?;
```

### Transport Manager

```rust
use rvoip_sip_transport::manager::TransportManager;

let mut manager = TransportManager::new();

// Add multiple transports
manager.add_transport("udp", udp_transport).await?;
manager.add_transport("tcp", tcp_transport).await?;

// Send message with automatic transport selection
manager.send_message(message, destination).await?;
```

## Relationship to Other Crates

### Core Dependencies

- **`rvoip-sip-core`**: Provides SIP message types and parsing
- **`tokio`**: Async runtime for network operations
- **`async-trait`**: Async trait support

### Optional Dependencies

- **`tokio-rustls`**: TLS transport support
- **`tokio-tungstenite`**: WebSocket transport support

### Integration with rvoip Stack

```
┌─────────────────────────────────────────┐
│            Application Layer            │
├─────────────────────────────────────────┤
│          rvoip-session-core             │
├─────────────────────────────────────────┤
│         rvoip-transaction-core          │
├─────────────────────────────────────────┤
│         rvoip-sip-transport  ⬅️ YOU ARE HERE
├─────────────────────────────────────────┤
│            Network Layer                │
└─────────────────────────────────────────┘
```

The transport layer sits between the transaction layer and the network, providing:

- **Upward Interface**: Delivers received messages to transaction-core
- **Downward Interface**: Handles actual network I/O operations
- **Event Propagation**: Notifies upper layers of transport events

## Testing

Run the test suite:

```bash
# Run all tests
cargo test -p rvoip-sip-transport

# Run with specific features
cargo test -p rvoip-sip-transport --features "tls ws"

# Run integration tests
cargo test -p rvoip-sip-transport --test integration_tests
```

## Features

The crate supports the following optional features:

- **`udp`** (default): UDP transport support
- **`tcp`** (default): TCP transport support  
- **`tls`** (default): TLS transport support
- **`ws`** (default): WebSocket transport support

Disable default features and enable only what you need:

```toml
[dependencies]
rvoip-sip-transport = { version = "0.1", default-features = false, features = ["udp", "tcp"] }
```

## Performance Characteristics

### UDP Transport
- **Pros**: Lowest latency, minimal overhead
- **Cons**: No reliability guarantees, size limitations
- **Use Case**: Time-sensitive applications, simple request/response

### TCP Transport  
- **Pros**: Reliable delivery, no size limits, connection reuse
- **Cons**: Higher latency, connection overhead
- **Use Case**: Large messages, guaranteed delivery

### TLS Transport
- **Pros**: Encrypted communication, authentication
- **Cons**: Highest overhead, certificate management
- **Use Case**: Secure communications, enterprise deployments

### WebSocket Transport
- **Pros**: Firewall-friendly, full-duplex, HTTP compatibility
- **Cons**: Additional protocol overhead
- **Use Case**: Web browsers, NAT traversal scenarios

## Error Handling

The crate provides comprehensive error handling with categorized error types:

```rust
use rvoip_sip_transport::Error;

match transport_result {
    Err(Error::ConnectionTimeout(addr)) => {
        // Handle timeout - often recoverable
        if error.is_recoverable() {
            retry_connection(addr).await?;
        }
    }
    Err(Error::TlsCertificateError(msg)) => {
        // Handle TLS errors - typically not recoverable
        log::error!("Certificate validation failed: {}", msg);
    }
    Err(Error::MessageTooLarge(size)) => {
        // Handle protocol violations - not recoverable
        return Err(error);
    }
    Ok(result) => {
        // Handle success
    }
}
```

## Future Improvements

See [TODO.md](./TODO.md) for a comprehensive list of planned enhancements, including:

- Advanced failover and load balancing
- Integration with infra-common event bus
- Enhanced monitoring and diagnostics
- Performance optimizations for high-scale deployments

## Contributing

Contributions are welcome! Please see the main [rvoip contributing guidelines](../../../README.md#contributing) for details.

## License

This project is licensed under the [MIT license](LICENSE).
