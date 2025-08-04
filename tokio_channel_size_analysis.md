# Tokio Channel Size Analysis

## Overview
This analysis examines tokio channel sizes across the rvoip codebase to identify potential bottlenecks that could cause dropped messages or performance issues under load.

## Critical Findings

### rtp-core (Most Important)
- **Session scheduling**: `32` - Might be low for high throughput scenarios
- **Session sender/receiver**: `1000` ✓ - Recently fixed from 100
- **Session events broadcast**: `1000` ✓ - Recently fixed from 100
- **UDP/TCP transport events**: `100` - Potential bottleneck under heavy RTP traffic
- **DTLS handshake**: `1` - Probably OK since handshake is one-time
- **Security transport**: `100`
- **API server broadcast**: `16` - Quite low for broadcast scenarios
- **API client frames**: `100`

### media-core (Important)
- Mostly uses `unbounded_channel()` ✓ - Good practice for media processing
- Test channels: `1000` ✓
- No low bounded channels found

## Other Crates

### session-core
- **Core channels**: `1000` ✓
- **Event channels**: `1000` ✓
- **Test channels**: `10`, `100`, `200` - Varies by test requirements

### dialog-core
- **Most channels**: `100`
- **Events channel in client API**: `1000` ✓

### transaction-core
- **Command channels**: `32` - Could be low for high call volumes
- **Event channels**: `100`
- **Test channels**: `10-100`
- **Timer manager**: `1-10`

### client-core
- **Event broadcast**: `256`
- **Test channels**: `10`

### sip-transport
- **Events**: `1` - Very low! Could drop SIP messages under load
- **Tests**: `100`
- **TCP connection**: `1-2`

## Potential Bottlenecks

1. **rtp-core**: Transport event channels at `100` could drop events under heavy RTP traffic
2. **rtp-core**: API server broadcast at `16` is quite low for high-concurrency scenarios
3. **sip-transport**: Events channel at `1` is extremely low and could drop SIP messages
4. **transaction-core**: Command channels at `32` might be low for high call volumes

## Recent Fix
The critical issue with RTP session channels being limited to `100` was already fixed by increasing to `1000`. This resolved the "100-frame problem" where audio streams would stop after exactly 100 frames.

## Recommendations
While the tests are currently passing, consider reviewing and potentially increasing:
1. sip-transport event channels (currently at 1)
2. rtp-core transport event channels (currently at 100)
3. transaction-core command channels (currently at 32)
4. rtp-core API server broadcast (currently at 16)

These changes would improve resilience under high load conditions.