# SIP Proxy Features — Usage and Testing Guide

**Document ID**: GUIDE-SIP-PROXY
**Date**: 2026-03-27
**Implementation plan**: `docs/plans/2026-03-27-sip-proxy-features.md`

---

## Overview

Four SIP proxy features were added to the rvoip stack:

| Feature | RFC | Status |
|---------|-----|--------|
| Server-Side Digest Auth (401/407) | RFC 3261 §22 / RFC 2617 | Complete |
| Via NAT handling (received/rport) | RFC 3581 | Complete |
| Record-Route insertion | RFC 3261 §16.6 | Complete |
| Proxy INVITE forwarding | RFC 3261 §16 | Complete |

---

## Architecture

```
web-console (HTTP :3000)
  └── CallCenterEngine
        └── SessionCoordinator
              └── DialogManager (session-core wrapper)
                    └── UnifiedDialogApi
                          └── dialog-core DialogManager
                                ├── AuthProvider  (Arc<RwLock<Option<...>>>)  ← runtime injectable
                                ├── ProxyRouter   (Arc<RwLock<Option<...>>>)  ← runtime injectable
                                ├── register_handler  → 401 Unauthorized
                                └── invite_handler    → 407 Proxy Auth Required / proxy forward
```

Both providers are stored behind `Arc<parking_lot::RwLock<Option<Arc<dyn T>>>>`.
Setters take `&self`, so swapping a provider takes effect on the next request
with no restart required.

---

## Quick Start

### Prerequisites

- Rust 1.85+
- PostgreSQL 18 (local or container)

```bash
# Start PostgreSQL
podman run -d --name rvoip-postgres \
  -e POSTGRES_USER=rvoip \
  -e POSTGRES_PASSWORD=rvoip_dev \
  -e POSTGRES_DB=rvoip \
  -p 5432:5432 \
  docker.io/library/postgres:18-alpine

# Start the server (UDP :5060 + WebSocket :8080 + HTTP :3000)
DATABASE_URL=postgres://rvoip:rvoip_dev@localhost:5432/rvoip \
RVOIP_JWT_SECRET=dev-secret \
RVOIP_ADMIN_PASSWORD=dev-admin \
SIP_REALM=rvoip \
cargo run -p rvoip-web-console --example web_console_server
```

On startup the server automatically creates the `sip_credentials` table and
adds the `routing_prefix` column to `sip_trunks` if they do not exist.

---

## Feature 1: Digest Authentication

### How it works

```
SIP Client                          rvoip
   |--- REGISTER / INVITE --------->|
   |<-- 401/407 WWW-Authenticate ---|   (fresh nonce)
   |--- REGISTER (Authorization) -->|
   |<-- 200 OK ---------------------|
```

- REGISTER challenges with **401 WWW-Authenticate** (RFC 3261 §22.1).
- INVITE challenges with **407 Proxy-Authenticate** (RFC 3261 §22.2).
- The `DbAuthProvider` queries `sip_credentials` on every request,
  so credential changes take effect immediately.

### Add credentials

```sql
INSERT INTO sip_credentials (id, username, password, realm, enabled)
VALUES (gen_random_uuid()::text, 'alice', 'secret123', 'rvoip', true);

-- Disable a user (instant, no restart)
UPDATE sip_credentials SET enabled = false WHERE username = 'alice';

-- Change a password (instant)
UPDATE sip_credentials SET password = 'newpass' WHERE username = 'alice';
```

### Test with a softphone

Register any SIP softphone (LinPhone, MicroSIP, Zoiper) against `sip:alice@127.0.0.1:5060`
with password `secret123`. Expected result: **200 OK** after the 401 challenge round-trip.

### Test without credentials

```bash
# Install SIPp
brew install sipp   # macOS

# Expect 407 for an unauthenticated INVITE
sipp -sn uac 127.0.0.1:5060 -d 1000 -m 1 2>&1 | grep -E "407|401"
```

### Unit tests

```bash
cargo test -p rvoip-dialog-core auth -- --nocapture
cargo test -p rvoip-dialog-core invite -- --nocapture
```

### Expected 407 response

```
SIP/2.0 407 Proxy Authentication Required
Proxy-Authenticate: Digest realm="rvoip", nonce="<hex>", algorithm=MD5
```

---

## Feature 2: Via NAT Handling

When a request arrives from a different IP/port than the Via header states,
rvoip adds `received=<actual-ip>` and `rport=<actual-port>` to the top Via
of every response (RFC 3581).

### Verify with Wireshark

Wireshark filter: `sip && ip.dst == 127.0.0.1`

Inspect the Via header in any 200 OK or 1xx response.  When the source
differs from the Via address you will see:

```
Via: SIP/2.0/UDP 10.0.0.5:5060;received=203.0.113.1;rport=12345
```

---

## Feature 3: Record-Route

rvoip operates as a B2BUA. The proxy inserts itself into the dialog route
set via the server Contact header in the 200 OK, which is functionally
equivalent to Record-Route for a back-to-back user agent.

For pure proxy mode the `create_response()` builder copies Record-Route
headers from the request to the response (RFC 3261 §16.6 step 6).

### Verify

Build a call between two softphones registered to rvoip.  Send a
re-INVITE mid-call and confirm the request targets the proxy address,
not the remote party directly.

```bash
cargo test -p rvoip-session-core record_route -- --nocapture
```

---

## Feature 4: Proxy INVITE Forwarding

### How it works

When a `ProxyRouter` is installed, the INVITE handler calls
`router.route_request()` before creating a local session:

```
ProxyAction::Forward { destination }  → create_forwarded_request() + forward_request()
ProxyAction::Reject  { status, reason } → send error response
ProxyAction::LocalB2BUA               → normal call handling (default)
```

`create_forwarded_request()` performs the RFC 3261 §16.6 modifications:
- Decrement Max-Forwards
- Prepend a new Via with a fresh branch parameter
- Insert a Record-Route header with the `;lr` parameter

### Configure a route

```sql
-- Forward calls starting with "9" to an external SIP server
UPDATE sip_trunks
SET routing_prefix = '9', status = 'active'
WHERE name = 'pstn-trunk';

-- Review routing table
SELECT name, host, port, routing_prefix, status FROM sip_trunks;
```

---

## Why Proxy Forwarding Cannot Be Fully Tested Locally

End-to-end proxy forwarding requires three independent SIP participants:

```
[Originating softphone] → [rvoip proxy] → [Target SIP server]
                                           ^
                                           Not available locally
```

### Missing resources

| Resource | Notes | Where to get |
|----------|-------|--------------|
| SIP trunk provider | Carrier-grade SIP server (Twilio, Vonage, VoIP.ms, …) | Paid account — ~$5–20/month for a test trunk |
| Publicly routable IP | The proxy must be reachable from the PSTN | Cloud VM (Hetzner, DigitalOcean, …) or port-forwarded router |
| SIP softphone (originator) | Any UA that can place a call | LinPhone, MicroSIP, Zoiper (free) |
| SIP softphone (terminator) | Second UA to receive the forwarded call | Same options as above |
| Open UDP 5060 | Must not be blocked by firewall or NAT | Cloud VM or explicit port mapping |

### Local substitute: two rvoip instances

```bash
# Instance A — proxy on :5060, forwards "9" prefix to instance B
DATABASE_URL=... SIP_REALM=rvoip \
cargo run -p rvoip-web-console --example web_console_server

# Instance B — plain B2BUA on :15060
SIP_PORT=15060 DATABASE_URL=... \
cargo run -p rvoip-web-console --example web_console_server

# Set routing rule on instance A
psql ... -c "UPDATE sip_trunks SET routing_prefix='9', status='active'
             WHERE name='local-b';"
# (Create the trunk first via the web console or INSERT directly)
```

### Local substitute: FreeSWITCH as target

```bash
docker run --rm -p 15060:5060/udp freeswitch/freeswitch:latest
# Point sip_trunks.host = '127.0.0.1', port = 15060
```

### Full cloud test topology

```
┌─────────────────────────────────┐
│  Cloud VM  (x.x.x.x)           │
│  rvoip web-console :3000        │
│  SIP UDP :5060                  │
│  SIP WebSocket :8080            │
└───────────────┬─────────────────┘
                │ SIP trunk (TLS or UDP)
┌───────────────▼─────────────────┐
│  Twilio / Vonage SIP Trunking   │
└───────────────┬─────────────────┘
                │ PSTN / SIP
┌───────────────▼─────────────────┐
│  LinPhone (any network)         │
│  sip:alice@x.x.x.x:5060        │
└─────────────────────────────────┘
```

### Unit tests (what can be tested locally)

```bash
# create_forwarded_request: Max-Forwards decrement, Via branch, Record-Route ;lr
cargo test -p rvoip-dialog-core create_forwarded -- --nocapture

# forward_request transport path
cargo test -p rvoip-dialog-core forward_request -- --nocapture
```

---

## Running the Test Suite

```bash
# Compile check (fastest)
cargo check --workspace

# Unit tests
cargo test -p rvoip-dialog-core
cargo test -p rvoip-session-core
cargo test -p rvoip-sip-core

# All tests with database
DATABASE_URL=postgres://rvoip:rvoip_dev@localhost:5432/rvoip \
cargo test --workspace 2>&1 | grep -E "PASSED|FAILED|test "
```
