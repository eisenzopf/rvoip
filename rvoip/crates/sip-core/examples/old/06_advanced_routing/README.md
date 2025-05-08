# Example 6: Advanced Routing

This example demonstrates SIP routing mechanisms that are essential for building proxies, registrars, and other infrastructure components. It focuses on how SIP messages navigate through the network using various header fields and routing techniques.

## What You'll Learn

- How Via headers are used to track the path of SIP requests and responses
- How Record-Route and Route headers enable proxies to stay in the signaling path
- How to implement basic proxy functionality in SIP
- How multi-hop communication works in a SIP network
- How loose routing works with the `lr` parameter
- How to handle routing sets for in-dialog requests

## Running the Example

```bash
# Run the example
cargo run --example 06_advanced_routing

# Run with debug logs to see the actual SIP messages
RUST_LOG=debug cargo run --example 06_advanced_routing
```

## Code Walkthrough

The example is divided into three parts:

1. **Via Header Processing**
   - Demonstrates how proxies add Via headers to requests
   - Shows how responses traverse the same path in reverse
   - Illustrates how each proxy removes its Via header from responses
   - Explains the importance of Via headers for request/response correlation

2. **Record-Route and Route Header Handling**
   - Shows how proxies insert Record-Route headers to stay in the signaling path
   - Demonstrates how UAs extract Record-Route headers from responses
   - Illustrates transforming Record-Route headers into Route headers for subsequent requests
   - Shows the reverse ordering of Route headers compared to Record-Route

3. **SIP Proxy Simulation**
   - Implements a complete SIP network with multiple domains and proxies
   - Demonstrates the full routing process for requests and responses
   - Shows how Max-Forwards prevents routing loops
   - Illustrates a complete call flow through multiple proxies

## Key Concepts

### Via Headers

SIP uses Via headers to track the path of a request so that responses can be sent back along the same path:

1. Each proxy adds its Via header at the beginning of the Via list
2. The Via headers record the path the request takes through the network
3. When a response is generated, it includes all Via headers from the request
4. Each proxy removes its own Via header before forwarding the response

### Record-Route and Route Headers

These headers enable proxies to remain in the signaling path for the entire dialog:

1. **Record-Route**: Added by proxies to INVITE requests and included in 2xx responses
2. **Route**: Used in subsequent in-dialog requests to ensure they follow the same path
3. **Order**: Record-Routes are used in reverse order when constructing the Route set
4. **Loose Routing**: The `lr` parameter indicates that a proxy supports loose routing (RFC 3261)

### SIP Routing Logic

The example demonstrates the complete routing logic used in SIP networks:

1. Request generation by User Agents
2. Outbound proxy processing
3. Request forwarding based on domains
4. Response path through the reverse proxy chain
5. Dialog establishment with proper route sets
6. In-dialog routing using stored Route sets

### The `lr` Parameter

This parameter in Record-Route/Route URIs indicates loose routing support:

1. Without `lr`, the proxy must use strict routing (older approach)
2. With `lr`, the Request-URI is preserved during routing (modern approach)
3. All modern SIP implementations should use loose routing

## Next Steps

After mastering advanced routing, you can move on to Example 7 which covers multipart message handling in SIP. 