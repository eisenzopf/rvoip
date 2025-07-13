# Unified Audio Peer Guide

This guide shows how to use the new **unified audio peer** approach for cross-computer VoIP calls.

## Key Benefits

✅ **Symmetric Architecture**: Both peers are equivalent - no "server" or "client"  
✅ **Either Direction**: Any peer can call any other peer  
✅ **Simple Deployment**: Just specify IP addresses  
✅ **Full Audio**: Both peers use microphone and speakers  
✅ **Network Ready**: Designed for multi-computer deployment  

## Quick Examples

### Same Computer Demo
```bash
# Quick localhost test
./run_peer_demo.sh
```

### Two Computers on Same Network

**Computer A (192.168.1.100):**
```bash
# Start listener
cargo run --bin audio_peer -- \
    --local-ip 0.0.0.0 \
    --display-name "Alice" \
    --answer-delay 1
```

**Computer B (192.168.1.200):**
```bash
# Call Alice
cargo run --bin audio_peer -- \
    --local-ip 0.0.0.0 \
    --call 192.168.1.100 \
    --display-name "Bob" \
    --duration 60
```

### Reverse Direction (Bob calls Alice)

**Computer A (192.168.1.100):**
```bash
# Alice calls Bob this time
cargo run --bin audio_peer -- \
    --local-ip 0.0.0.0 \
    --call 192.168.1.200 \
    --display-name "Alice" \
    --duration 45
```

**Computer B (192.168.1.200):**
```bash
# Bob listens for Alice's call
cargo run --bin audio_peer -- \
    --local-ip 0.0.0.0 \
    --display-name "Bob"
```

## Network Deployment Scenarios

### Corporate Network (Different Subnets)
```bash
# Engineering department (10.1.0.100):
cargo run --bin audio_peer -- --local-ip 0.0.0.0 --display-name "Engineering"

# Sales department (10.2.0.50):
cargo run --bin audio_peer -- --call 10.1.0.100 --display-name "Sales"
```

### Home Network + VPN
```bash
# Home office (192.168.1.100):
cargo run --bin audio_peer -- --local-ip 0.0.0.0 --display-name "Home"

# Remote office via VPN (10.8.0.5):
cargo run --bin audio_peer -- --call 192.168.1.100 --display-name "Remote"
```

### Internet (Port Forwarding Required)
```bash
# Public server (203.0.113.10):
cargo run --bin audio_peer -- --local-ip 0.0.0.0 --display-name "Server"

# Client behind NAT:
cargo run --bin audio_peer -- --call 203.0.113.10 --display-name "Client"
```

## Command Line Reference

### Listener Mode (Default)
```bash
cargo run --bin audio_peer -- [OPTIONS]

# No --call option = listener mode
# Waits for incoming calls
```

### Caller Mode
```bash
cargo run --bin audio_peer -- --call <REMOTE_IP> [OPTIONS]

# --call option = caller mode
# Initiates call to remote peer
```

### Essential Options
- `--local-ip`: IP to bind to (use `0.0.0.0` for all interfaces)
- `--display-name`: Your name/identity
- `--call`: Remote peer's IP address (enables caller mode)
- `--duration`: Call length in seconds (caller mode only)
- `--answer-delay`: Auto-answer delay (listener mode only)

## Audio Quality

Both peers automatically configure:
- **Sample Rate**: 8000Hz (narrowband voice)
- **Codec**: PCMU (G.711 μ-law)
- **Channels**: 1 (mono)
- **Frame Size**: 20ms
- **Echo Cancellation**: Enabled
- **Noise Suppression**: Enabled

## Troubleshooting

### No Audio Devices
```
❌ [Alice] No audio devices found!
```
**Solution**: Ensure microphone and speakers are connected and working.

### Network Connection Issues
```
❌ [Bob] Failed to connect to 192.168.1.100:5060
```
**Solutions**:
1. Check IP address is correct
2. Verify firewall allows SIP (port 5060) and RTP (ports 20000+)
3. Ensure listener peer is running first
4. Test with `ping` or `telnet` to verify network connectivity

### Audio Feedback
```
🔊 Hearing echo or feedback
```
**Solution**: Use headphones or separate microphone/speaker by distance.

## Advanced Usage

### Custom Ports
```bash
# Custom SIP port
cargo run --bin audio_peer -- --local-port 5070

# Custom RTP port range
cargo run --bin audio_peer -- --rtp-port-start 25000
```

### Longer Calls
```bash
# 10 minute call
cargo run --bin audio_peer -- --call 192.168.1.100 --duration 600
```

### Instant Answer
```bash
# Answer immediately (no delay)
cargo run --bin audio_peer -- --answer-delay 0
```

## Production Deployment

For production use, consider:

1. **Security**: Use TLS/DTLS for encrypted audio
2. **Firewall**: Open necessary ports (5060 for SIP, 20000-21000 for RTP)
3. **NAT Traversal**: Use STUN/TURN servers for NAT situations
4. **Load Balancing**: Multiple peers for high availability
5. **Monitoring**: Log analysis and call quality metrics

The unified peer architecture makes it easy to deploy VoIP calling across any network topology! 