# RTP Loopback Example

This example demonstrates the use of the `rtp-core` crate for sending and receiving RTP packets in a loopback configuration. It creates two RTP sessions - a sender and a receiver - and sends packets between them over UDP.

## Features Demonstrated

- Creating RTP sessions
- Configuring RTP session parameters
- Sending RTP packets with timestamps
- Receiving and parsing RTP packets
- Basic RTP statistics
- RTCP packet handling and reporting (in RTCP mode)

## Running the Example

From the workspace root, run:

```bash
cargo run --example rtp-loopback
```

This will start the example with default settings:
- Sender bound to 127.0.0.1:10000
- Receiver bound to 127.0.0.1:10001
- 10 packets sent with 100ms interval
- Payload type 0 (PCMU)

### RTCP Example

The example also includes an RTCP demonstration that can be run with:

```bash
cargo run --example rtp-loopback -- --rtcp
```

The RTCP example shows:
- RTP packet flow between sender and receiver
- Generation of RTCP Sender Reports
- Statistics tracking for RTCP reporting
- Handling of packet sequence and timing for RTCP reports

## Command-Line Options

You can customize the behavior with these options:

```bash
cargo run --example rtp-loopback -- --help
```

Available options:
- `-s, --sender-addr <ADDR>`: Local address for the sender (default: 127.0.0.1:10000)
- `-r, --receiver-addr <ADDR>`: Local address for the receiver (default: 127.0.0.1:10001)
- `-c, --count <COUNT>`: Number of packets to send (default: 10)
- `-i, --interval <INTERVAL>`: Interval between packets in milliseconds (default: 100)
- `-p, --payload-type <PT>`: Payload type (default: 0)
- `--rtcp`: Run the RTCP example instead of the basic loopback

Example with custom settings:

```bash
cargo run --example rtp-loopback -- -s 127.0.0.1:20000 -r 127.0.0.1:20001 -c 20 -i 50
```

## Expected Output

The example outputs logging information about packets sent and received. A successful run should show:
- Initialization of sender and receiver
- Sent and received packet information
- Final statistics
- Confirmation that the test completed successfully

When running the RTCP example, you'll also see:
- RTCP Sender Reports that would be sent (serialization not fully implemented yet)
- Packet flow statistics used for RTCP reporting

## Architecture

The example follows this structure:
1. Parse command-line arguments
2. Create sender and receiver RTP sessions
3. Start async task for receiving packets
4. Send test packets with incremental timestamps
5. Wait for receiver to process all packets
6. Display statistics and results

The RTCP example adds:
1. RTCP sender/receiver setup
2. Packet flow monitoring for statistics
3. Periodic generation of RTCP Sender Reports
4. Tracking of packet sequence and loss 