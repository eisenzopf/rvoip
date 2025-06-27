# SIP Client-Server Example

This example demonstrates a complete SIP call flow between a UAC (User Agent Client) and UAS (User Agent Server) using the `rvoip_session_core` library. The example includes both signaling (SIP) and media (RTP) handling.

## Components

1. **UAS Server (`uas_server.rs`)**: A SIP server that listens for incoming calls and auto-answers them.
2. **UAC Client (`uac_client.rs`)**: A SIP client that makes outgoing calls to the server.
3. **Test Script (`run_test.sh`)**: A shell script that builds and runs both components, capturing logs and analyzing results.

## Prerequisites

- Rust toolchain (cargo, rustc)
- Bash shell environment

## Usage

### Running the Complete Test

The simplest way to run the example is using the provided test script:

```bash
cd rvoip/crates/session-core/examples/client-server
./run_test.sh
```

This will:
1. Build both the server and client binaries
2. Start the UAS server in the background
3. Run the UAC client to make calls to the server
4. Analyze the results and generate a report
5. Clean up processes when done

### Configuration

You can customize the test by setting environment variables:

```bash
# Set custom ports and parameters
SERVER_PORT=5062 CLIENT_PORT=5061 NUM_CALLS=3 CALL_DURATION=5 ./run_test.sh
```

Available configuration options:

| Variable | Default | Description |
|----------|---------|-------------|
| SERVER_PORT | 5062 | Port for the UAS server |
| CLIENT_PORT | 5061 | Port for the UAC client |
| NUM_CALLS | 1 | Number of calls to make |
| CALL_DURATION | 10 | Duration of each call in seconds |
| LOG_LEVEL | info | Logging verbosity (debug, info, warn, error) |

### Running Components Manually

You can also build and run the components manually:

#### Build the binaries:

```bash
cd rvoip
cargo build --bin uas_server
cargo build --bin uac_client
```

#### Run the UAS server:

```bash
cargo run --bin uas_server -- --port 5062
```

#### Run the UAC client:

```bash
cargo run --bin uac_client -- --port 5061 --target 127.0.0.1:5062 --calls 1 --duration 10
```

## Logs and Reports

The test script generates several log files in the `logs` directory:

- `uas_server_TIMESTAMP.log`: Server-specific logs
- `uac_client_TIMESTAMP.log`: Client-specific logs
- `sip_test_TIMESTAMP.log`: Combined logs and test output
- `test_report_TIMESTAMP.txt`: Test summary and results

## Understanding the Code

### UAS Server

The UAS server demonstrates:
- Setting up a SIP listener using `SessionManager`
- Implementing the `CallHandler` trait to handle incoming calls
- Generating SDP answers for media negotiation
- Managing call lifecycle events

### UAC Client

The UAC client demonstrates:
- Initiating outbound SIP calls
- Creating SDP offers for media negotiation
- Managing active calls
- Handling call termination

### SIP/RTP Flow

A successful test shows the complete SIP call flow:
1. UAC sends an INVITE with SDP offer to UAS
2. UAS responds with 200 OK with SDP answer
3. UAC sends ACK to complete call setup
4. RTP media can flow between endpoints
5. After the specified duration, UAC sends BYE
6. UAS responds with 200 OK to complete call teardown

## Extending the Example

You can extend this example in several ways:
- Add media handling to actually send/receive audio
- Implement more complex call scenarios (transfer, hold, etc.)
- Add TLS for secure SIP signaling
- Implement authentication mechanisms
- Add support for multiple concurrent calls

## Troubleshooting

If you encounter issues:

1. Check the log files for detailed error messages
2. Ensure ports are not already in use by other applications
3. Try running with `LOG_LEVEL=debug` for more verbose output
4. Make sure your firewall allows the specified ports 