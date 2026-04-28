# Asterisk Regression Root-Cause Tracker

This tracks the Asterisk regression-risk investigation for the release that
highlights the `UnifiedCoordinator`, `StreamPeer`, and `CallbackPeer` API
surfaces.

## Status

Complete for the local release-validation lab.

The failure was not reproduced after fixing the local Asterisk profile. The
default and extended StreamPeer and CallbackPeer Asterisk suites pass with the
lab defining all endpoints required by the examples.

## Root Cause

The extended StreamPeer suite previously failed before placing a call because
endpoint `1003` could not register:

```text
REGISTER auth failed ... invalid credentials
```

The local Asterisk lab at `/Users/jonathan/Developer/asterisk` only defined
`1001`, `1002`, `2001`, and `2002`. The extended rvoip scenarios require
additional rvoip-controlled endpoints:

- `1003` for TLS/SRTP ring/cancel and transfer target scenarios.
- `2003` for UDP/RTP ring/cancel and transfer target scenarios.

This made the first failing signal a PBX lab profile mismatch, not a
`session-core` behavior regression.

## Lab Changes Applied

Updated `/Users/jonathan/Developer/asterisk/config/pjsip.conf`:

- Added TLS/SRTP endpoint `1003`.
- Added auth section `auth1003`.
- Added AOR `1003`.
- Added username/password `1003` / `password123`.
- Added caller ID `"rvoip 1003" <1003>`.
- Added UDP/RTP endpoint `2003`.
- Added auth section `auth2003`.
- Added AOR `2003`.
- Added username/password `2003` / `password123`.
- Added caller ID `"rvoip 2003" <2003>`.

Updated `/Users/jonathan/Developer/asterisk/config/extensions.conf`:

- Added `exten => 1003,... Dial(PJSIP/1003,45)`.
- Added `exten => 2003,... Dial(PJSIP/2003,45)`.

Updated `/Users/jonathan/Developer/asterisk/scripts/make-rvoip-env.sh` and
`/Users/jonathan/Developer/asterisk/rvoip-local.env`:

```sh
ENDPOINT_1003_LOCAL_PORT=5074
ENDPOINT_1003_TLS_LOCAL_PORT=5075
ENDPOINT_1003_MEDIA_PORT_START=16240
ENDPOINT_1003_MEDIA_PORT_END=16340
ENDPOINT_2003_LOCAL_PORT=5084
ENDPOINT_2003_MEDIA_PORT_START=17240
ENDPOINT_2003_MEDIA_PORT_END=17340
```

Updated `/Users/jonathan/Developer/asterisk/README.md` to document the six
supported endpoints and the extended rvoip scenarios.

Reloaded the local Asterisk container with `core reload` and confirmed six
PJSIP endpoints are available:

- `1001`, `1002`, `1003`
- `2001`, `2002`, `2003`

## Validation Results

Required checks:

| Check | Result |
| --- | --- |
| `cargo fmt --check` | Passed |
| `cargo check -p rvoip-session-core` | Passed |
| `cargo test -p rvoip-session-core` | Passed |
| Default StreamPeer Asterisk suite | Passed |
| Extended StreamPeer Asterisk suite | Passed |
| Default CallbackPeer Asterisk suite | Passed |
| Extended CallbackPeer Asterisk suite | Passed |

Supporting checks:

| Check | Result |
| --- | --- |
| `cargo doc -p rvoip-session-core --no-deps` | Passed with rustdoc warnings |

The rustdoc warnings are unresolved intra-doc links and one invalid HTML tag.
They are documentation quality issues and are not related to the Asterisk
registration failure.

## Current Conclusion

The evidence supports the original root-cause hypothesis: the extended Asterisk
failure was caused by a local PBX profile mismatch. No `session-core` behavior
change is needed to work around missing PBX endpoints.

If these suites fail again with the six-endpoint profile loaded, treat the new
failure as a potential `session-core` regression and debug in this order:

1. Registration flow: compare REGISTER, 401, and authenticated REGISTER for
   `1001` versus `1003`.
2. Ring/cancel: inspect typed dispatch for `IncomingCall`,
   `CallStateChanged(Ringing)`, `CallCancelled`, and `AckReceived`.
3. DTMF: inspect RFC 4733 receive events and direct API-bus DTMF delivery.
4. Transfer: inspect `TransferRequested`, REFER response, NOTIFY progress,
   `ReferReceived`, and the `referred_by` / `replaces` metadata path.
