# session-core Interop CI Plan

This is the staged plan for turning today's examples into reproducible interop
gates.

## Phase 1: SIPp local scenarios

Scenarios:

- REGISTER 200 OK
- REGISTER 401/407 digest retry
- REGISTER 423 Min-Expires retry
- INVITE 180/200/ACK/BYE
- INVITE failure 4xx/5xx/6xx
- CANCEL before answer
- hold/resume re-INVITE
- REFER accepted/rejected
- REFER progress NOTIFY

Each scenario should assert both SIP wire behavior and public `Event` output.

## Phase 2: Docker Asterisk profile

Automate the current manual release gates:

- `examples/asterisk/run.sh`
- `ASTERISK_RUN_EXTENDED_TESTS=1 examples/asterisk/run.sh`
- `examples/asterisk_callback/run.sh`
- `ASTERISK_RUN_EXTENDED_TESTS=1 examples/asterisk_callback/run.sh`

The Docker profile must capture SIP logs, example logs, and audio analysis
artifacts.

## Phase 3: FreeSWITCH/Sofia profile

Add equivalent coverage for:

- UDP/RTP registration and calls
- TLS/SRTP where supported by the profile
- hold/resume
- DTMF
- CANCEL
- blind transfer

## Phase 4: Proxy plus RTPengine

Add Kamailio or OpenSIPS in front of RTPengine to validate:

- Record-Route / Route behavior
- advertised addresses
- proxy-mediated REGISTER and INVITE flows
- media relay assumptions
- REFER/NOTIFY routing

## Phase 5: Carrier/SBC scripts

Create manual and CI-safe fixtures for provider-like behavior:

- outbound proxy registration
- Service-Route / Path
- SRV/NAPTR DNS
- TLS policy
- NAT and Contact rewrite behavior
- flow failure and reconnect churn

No carrier/SBC row should be marked validated until the scenario has a
repeatable command and expected event assertions.
