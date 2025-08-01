# SIP P2P Voice Call Example

This example demonstrates how to make direct peer-to-peer SIP voice calls between two computers without needing a SIP server.

## Prerequisites

- Two computers on the same network (or with network connectivity)
- Microphone and speakers on both computers
- Rust toolchain installed

## Running the Example

### Step 1: Start the Receiver

On the first computer, run:

```bash
cargo run -- receive -n alice
```

This will display:
- Your IP address (e.g., `192.168.1.100`)
- Your SIP address (e.g., `sip:alice@192.168.1.100:5060`)
- A command for the caller to use

### Step 2: Start the Caller

On the second computer, use the command shown by the receiver:

```bash
cargo run -- call -n bob -t 192.168.1.100
```

Replace `192.168.1.100` with the actual IP address shown by the receiver.

### Step 3: Have a Conversation!

- The call will connect automatically
- You'll see audio level meters showing microphone and speaker activity
- Talk naturally - the voice data is transmitted in real-time
- Press `Ctrl+C` to end the call

## Command Options

### Receiver Mode
```bash
cargo run -- receive --help
```
- `-n, --name <NAME>`: Your name (required)
- `-p, --port <PORT>`: Port to listen on (default: 5060)

### Caller Mode
```bash
cargo run -- call --help
```
- `-n, --name <NAME>`: Your name (required)
- `-t, --target <TARGET>`: Target IP address (required)
- `-P, --target-port <PORT>`: Target port (default: 5060)
- `-p, --port <PORT>`: Your local port (default: 5061)

## Troubleshooting

### No Audio
- Check that your microphone and speakers are properly connected
- The example will list available audio devices when starting
- Make sure your system audio settings allow the application to access the microphone

### Connection Failed
- Ensure both computers are on the same network
- Check firewall settings - SIP uses UDP port 5060/5061 by default
- Verify the IP address is correct

### Poor Audio Quality
- This example uses G.711 μ-law codec by default
- Network congestion can affect quality
- Try moving closer to your WiFi router

## How It Works

This example demonstrates the simplicity of the `rvoip-sip-client` library:

```rust
// Create a SIP client with one line
let client = SipClientBuilder::new()
    .sip_identity(format!("sip:{}@{}:{}", name, ip, port))
    .local_address(format!("0.0.0.0:{}", port).parse()?)
    .build()
    .await?;

// Make a call
let call = client.call("sip:alice@192.168.1.100:5060").await?;

// Or receive calls by listening to events
let mut events = client.event_iter();
while let Some(event) = events.next().await {
    match event {
        SipClientEvent::IncomingCall { call, from, .. } => {
            client.answer(&call.id).await?;
        }
        // ... handle other events
    }
}
```

The library handles all the complexity of:
- SIP protocol negotiation
- Audio device management
- Codec selection and encoding/decoding
- Network transport
- Error recovery and reconnection