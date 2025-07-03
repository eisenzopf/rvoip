# RVOIP Examples

This directory contains practical examples demonstrating how to use the RVOIP library components.

## Available Examples

### 📞 peer-to-peer/
A minimal peer-to-peer SIP call demo using the `client-core` library exclusively. Demonstrates:
- Full SIP call establishment between two User Agents
- Bidirectional RTP media exchange
- Comprehensive logging and statistics
- Clean shutdown and error handling

**Quick start:**
```bash
cd peer-to-peer
./run_demo.sh
```

This example serves as an excellent starting point for understanding RVOIP's client-core API and SIP call flows.

### 🏢 call-center/
A complete call center demonstration using both `call-engine` and `client-core` libraries. Demonstrates:
- Call center server with agent registration and call routing
- Agent clients that register and handle customer calls
- Customer client calling the support line
- Intelligent call queuing and routing
- B2BUA call bridging between customers and agents
- RTP media exchange in call center environment
- Comprehensive logging and monitoring

**Quick start:**
```bash
cd call-center
./run_demo.sh
```

This example shows how to build production-ready call center applications with minimal code, featuring automatic agent assignment, call queuing, and media bridging.

## Additional Examples

For more advanced examples and use cases, also see:
- `crates/client-core/examples/` - Client-core specific examples
- `crates/call-engine/examples/` - Call engine examples
- `crates/session-core/examples/` - Session management examples