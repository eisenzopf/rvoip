# SIP Client P2P Example

This example demonstrates peer-to-peer SIP voice calls between two computers without requiring a SIP server.

## Features

- Direct peer-to-peer SIP calls
- Real-time audio using system microphone and speakers
- G.711 μ-law/A-law codec support (standard telephony codecs)
- Audio level visualization during calls
- No SIP server required

## Requirements

- Two computers on the same network
- Microphone and speakers on both computers
- Rust toolchain installed

## Usage

### Step 1: Start the Receiver

On the first computer, run:

```bash
cargo run --release -- receive -n alice
```

Or to bind to a specific IP address (e.g., if you have multiple network interfaces):

```bash
cargo run --release -- receive -n alice --ip 192.168.1.100
```

This will display:
- Your IP address
- Your SIP address (e.g., `sip:alice@192.168.1.100:5060`)
- A command for the caller to use

### Step 2: Make a Call

On the second computer, use the command shown by the receiver. For example:

```bash
cargo run --release -- call -n bob -t 192.168.1.100
```

Where `192.168.1.100` is the IP address of the receiver.

### During the Call

- The receiver will auto-answer incoming calls
- Audio level meters show microphone and speaker activity
- Press Ctrl+C to end the call

## Command Options

### Receiver Mode
```
cargo run -- receive [OPTIONS]

OPTIONS:
    -n, --name <NAME>    Your name (e.g., "alice")
    -p, --port <PORT>    Port to listen on [default: 5060]
    -i, --ip <IP>        IP address to bind to [default: auto-detect]
```

### Caller Mode
```
cargo run -- call [OPTIONS]

OPTIONS:
    -n, --name <NAME>              Your name (e.g., "bob")
    -t, --target <TARGET>          Target IP address of the receiver
    -P, --target-port <PORT>       Target port [default: 5060]
    -p, --port <PORT>             Your local port [default: 5061]
```

## Troubleshooting

### No Audio
- Check microphone permissions in your system settings
- Ensure both computers can ping each other
- Try running with `RUST_LOG=debug` for more information

### Connection Failed
- Verify firewall settings allow UDP traffic on ports 5060-5061
- Make sure you're using the correct IP address
- Check that no other SIP applications are using the same ports

### Poor Audio Quality
- The example uses G.711 codec (8kHz) which is telephony quality
- Ensure stable network connection between computers
- Close other bandwidth-intensive applications

## Technical Details

- Uses RVoIP SIP client library for SIP signaling
- Audio captured at hardware native rate (usually 44.1kHz or 48kHz)
- Automatic format conversion to 8kHz for G.711 codec
- RTP transport for real-time audio streaming
- Direct peer-to-peer connection (no media server)