# SIP Resilience RFC Test Stubs

This directory tracks the RFC resilience backlog as ignored integration-test
stubs. The stubs compile and document ownership without failing normal CI.

| Area | Primary owner | Existing support | Next step |
| --- | --- | --- | --- |
| RFC 3261 transaction loss/retransmit/timers | Lower-library hardening | `sip-dialog` transaction tests; `rvoip-sip` teardown invariants | Add deterministic packet fault injection below `rvoip-sip`. |
| RFC 3262 reliable provisional / PRACK | `rvoip-sip` API | `prack_integration.rs`, `reliable_provisional_bridge.rs` | Add retransmitted 183 and missing-PRACK recovery fixtures. |
| RFC 3263 DNS SRV/NAPTR failover | Lower-library hardening | Transport recovery perf coverage, outbound routing tests | Add injectable DNS/candidate failure controls. |
| RFC 3311 UPDATE / re-INVITE glare | Mixed | `glare_retry_integration.rs`, state-table rollback rules | Add UPDATE-vs-re-INVITE races and retained-owner assertions. |
| RFC 3581 rport / NAT | Lower-library hardening | `sip-dialog` stamps received/rport | Add NAT/source-port rewrite harness. |
| RFC 4028 session timers | `rvoip-sip` API | session timer success/failure and 422 retry tests | Add repeated refresh and post-failure retention assertions. |
| RFC 5626 outbound flow recovery | Mixed | flow-failure event handling and refresh throttling | Add flow-failure injection and keepalive policy tests. |
| 481/408/503/Retry-After/forking recovery | Mixed | generic final-response, redirect, timeout handling | Define per-method API contract and forked-dialog lower-layer diagnostics. |
| PBX/proxy recovery interop | External interop + mixed hardening | Asterisk/FreeSWITCH beta matrices, SIPp parity | Add restart/drop/failover scenarios and route failures to the owning layer. |
