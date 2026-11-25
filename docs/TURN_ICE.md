# Check TURN and ICE with real media

Use this example to check that rvoip can allocate a TURN relay, complete ICE,
and send audio packets in both directions. It exits unsuccessfully if any check
fails. These are developer-preview WebRTC checks; they do not change the SIP
release qualification.

## 1. Prepare your checkout

Run these commands from the rvoip repository root on a branch containing the
TURN implementation. Install Rust with rustup; the checkout selects its pinned
Rust toolchain automatically. Install the build dependencies once:

```sh
# Ubuntu / Debian
sudo apt-get install -y build-essential pkg-config cmake protobuf-compiler libopus-dev libasound2-dev libssl-dev

# macOS (Homebrew)
brew install pkg-config cmake protobuf opus
```

The first run builds the dependencies and can take several minutes. No AWS
account, browser, Docker container, microphone, or production credentials are
needed for the local checks.

## 2. Run the TURN example

```sh
cargo run --locked -p rvoip-webrtc --example turn_ice
```

The example starts a temporary TURN server on loopback and connects two peers
using relay-only ICE. It verifies all of the following:

- Both peers allocate a relay and advertise only relay candidates.
- ICE nominates a relay-to-relay pair on both peers.
- Each peer receives the exact audio packet sent by the other, including its
  payload and a distinct timestamp. Initial track-discovery packets cannot
  accidentally satisfy this check.
- Closing the peers releases both TURN allocations and shuts down the server.

Success prints two `PASS` lines. The server uses synthetic local credentials
and an ephemeral port; nothing is left running. No public STUN server is used.

## 3. Check direct ICE without any ICE servers

```sh
cargo run --locked -p rvoip-webrtc --example turn_ice -- --direct
```

This runs the same ICE and bidirectional packet checks with `ice_servers: []`
and loopback host candidates. Success prints `PASS: host/host ICE nominated`.
It shows that a STUN server is not required when peers can reach each other
already; it does not promise direct connectivity across arbitrary NATs.

## 4. Try your own TURN server (optional)

Save this JSON to a private file outside the checkout, replacing the uppercase
placeholders. Use a TURN server that permits its clients to exchange media
through two allocations, including relay-to-relay traffic on that server.

```json
{
  "udp_bind": "0.0.0.0:0",
  "ice_servers": [{
    "urls": ["turn:YOUR_TURN_HOST:3478?transport=udp"],
    "username": "YOUR_USERNAME",
    "credential": "YOUR_PASSWORD"
  }],
  "ice_transport_policy": "relay",
  "gather_timeout_secs": 10,
  "connection_timeout_secs": 30,
  "turn_attempt_timeout_secs": 2,
  "turn_send_timeout_secs": 2,
  "turn_close_timeout_secs": 2
}
```

```sh
chmod 600 /path/to/private-turn.json
cargo run --locked -p rvoip-webrtc --example turn_ice -- --config /path/to/private-turn.json
```

The file is a `WebRtcConfig`: server URLs, credentials, bind address, UDP port
range, and timeouts remain yours to configure. Custom mode requires relay-only
policy and fails if it cannot allocate a relay; it never falls back to the
built-in server or direct media. Only UDP TURN is supported by this path.

The program prints no credentials, SDP, or endpoint addresses. It verifies
relay-only candidates, nominated pairs, two-way packets, and successful peer
close. Unlike the local mode, it cannot inspect your server's allocation count;
use your TURN server's logs or metrics to verify allocation release.

For a restricted-egress check, run this mode inside your controlled test host
or container with UDP egress allowed only to your TURN server and direct peer
media destinations blocked. Permit DNS separately if the URL uses a hostname.
Keep packet counters or firewall logs as evidence of the actual policy. The
local loopback example alone does **not** qualify a customer's firewall, Amazon
Connect, or Vapi integration. Connect credentials are contact-scoped; use the
[Connect TURN integration](CONNECT_TURN.md) for that lifecycle instead of
copying live contact credentials into this example.

## 5. Run the regression suite

```sh
cargo test --locked -p rvoip-webrtc --test turn_relay --test turn_relay_e2e
cargo test --locked -p rvoip-webrtc-stack --test configuration_test
```

These also check missing/bad credentials, allocation timeouts, fallback from an
unreachable TURN server to a working one, and cleanup when a peer is dropped.
A missing TURN server causes a failure, never a silently skipped green test.
The PR `webrtc-runtime` gate runs the relay suite and both local example modes;
the regular crate gates run the other tests and Clippy.

If setup fails, check UDP reachability and credentials first. If allocation
works but ICE or audio times out, check relay-to-relay permissions and your
server's peer-address policy. Increasing timeouts cannot fix blocked traffic.
