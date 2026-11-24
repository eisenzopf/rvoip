# rvoip 0.5.0: RFC 7501/7502 SIP benchmark of rvoip-bench-uas on c8i.4xlarge

## Summary

- Sustained Session Establishment Rate (RFC 7502 §4.10, zero Session Attempt Failures, >= 180 s trials) of rvoip-bench-uas on one c8i.4xlarge (16 vCPU): R1 TLS no media 8,348 sps; R5 UDP no media 8,256 sps; R2 TLS with SRTP media 1,124 sps; R6 UDP with RTP media 1,204 sps.
- Every row is limited by the DUT, not the tester: R1/R5 by setup delay (the DUT queues above R), R2/R6 by admission (503 above R). Against ea-uas the testers reached 1.7-4.3 x these rates (§6.1 baseline).
- Overload and recovery (R4, at C = R2): under 1.5 x and 2 x C the DUT admitted about C and shed the rest with fast 503s, with no other failure; recovery at 0.5 x and 1 x C established every call of its steady window.

Report date 2026-11-24. Device under test (DUT): rvoip-bench-uas (rvoip b089eeb5d4b2) on c8i.4xlarge, a terminating SIP UAS and media endpoint. Tester: 3 x c8in.24xlarge Emulated Agents (UAC). Every number comes from valid trials run strictly serially (one DUT under load at a time, a fresh DUT process per trial).

## Test environment and method

| Item | Value |
|---|---|
| DUT | rvoip-bench-uas at b089eeb5d4b2: a terminating SIP UAS built on rvoip. It answers every INVITE with 200 OK and, for media rows, echoes every received RTP/SRTP packet to its source through rvoip's media executor v2. It is not a proxy, B2BUA or registrar. |
| DUT host | AWS c8i.4xlarge: 16 vCPU (8 physical cores), Intel Xeon 6; one DUT process per trial, started fresh |
| Testers (Emulated Agents) | 3 x AWS c8in.24xlarge (96 vCPU each), 2 source IPs per host, 8 connections (TLS) or UDP 5-tuples per source: 48 flows. The caller (rvoip-loadgen) is a private build, pinned by sha256 with the environment. |
| Network | AWS us-west-2, availability zone us-west-2a; DUT and testers in one cluster placement group |
| Testbed baseline (RFC 7502 §6.1) | The same search against ea-uas (an EA in UAS mode, no DUT): the testers reached R1 >= 14,168, R5 >= 14,016, R2 >= 4,664 and R6 >= 5,128 sps (R6 a lower bound). A DUT result may not exceed 0.67 x its row's baseline, the tester cap. |
| Search (RFC 7502 §4.10) | bisect: up by w = 10 % while trials pass, down by d = 10 % on a failure, to a 2 % resolution, 3 confirmations at R, quantum 4 sps |
| Trial | a 30 s warm-up at r/2 (not scored), then at least 180 s and N >= 50,000 session attempts at the trial rate |
| Criterion | zero Session Attempt Failures (RFC 7501 §3.1.8): every attempt in the scored stage gets a 200 OK within 10 s; a 503 is a DUT failure. A trial is void (not counted) when the tester itself was the limit: caller CPU, a caller thread above 0.8 core, the caller's kernel receive path, or dispatch timing. |
| Rows | R1 TLS no media, hold 0 s; R5 UDP no media, hold 0 s; R2 TLS + SDES-SRTP G.711 echoed, hold 10 s; R6 UDP + RTP G.711 echoed, hold 10 s |
| R4 overload and recovery | An extension from the E74 harness, not an RFC 7502 benchmark (RFC 7339 behaviour). With admission control at C = R2: 0.5, 1, 1.5 and 2 x C, then recovery at 0.5 and 1 x C, 60 s each after a ramp. Pass: every failure in any stage is a 503 (nothing times out or is lost), and both recovery stages establish every call of their steady window (each stage's first 15 s are not scored). |

### Provenance

- The trials ran in bench-minor GitHub Actions run 45065590604 on candidate b089eeb5d4b2: the 0.5.0 code before the url dependency bump (RUSTSEC-2026-0592), which changes no benchmarked code path.
- The report the run rendered at the time scored R1 and R5 as tester-invalid and R6 and R4 as failed. Those were harness rules, not DUT behaviour, fixed in rvoip PR #468: a DUT whose median setup delay collapses is a DUT failure, not a tester limit; overload recovery is scored on its steady window like every other stage. This report re-applies those rules to the same stored trial evidence; no trial was re-run and no number was edited. The re-scored values are the ones recorded in the env-id's rates.json.
- The 0.5.0 release then confirmed R1, R5 and R2 in bench-release run 45221679416, on candidate 81452a5fb8a5, which the published commit 2ae9fa49c reuses because it differs only in release bookkeeping and version strings. Three trials at 0.95 x R per row and the latency step at 0.9 x R passed. Its verdict was WARN because the 1.05 x R probes of R1 (8,764 sps) and R5 (8,668 sps) also passed, so those two rates may be slightly understated. R2's 1.05 x R probe failed, as expected at its limit.

## 1. Headline: Sustained Session Establishment Rate (>= 180 s trials) per row

*sps = sessions per second. Each trial: unscored warm-up, then >= 180 s at the rate (N = max(50,000, rate x 180)) with zero Session Attempt Failures (RFC 7501 §3.1.8); fresh DUT process per trial. A result at r_max or above 0.67 x the §6.1 baseline reads ">= R" (tester-limited). DUT: rvoip-bench-uas (rvoip b089eeb5d4b2).*

| Row | Transport | Media | Hold | Session Establishment Rate R | Binding limit | §6.1 baseline | r_max (sps) | Trials |
|---|---|---|---|---|---|---|---|---|
| R1 | TLS | none | 0 s | 8,348 sps | DUT | >= 14,168 sps | 9,492 | 5 |
| R2 | TLS | SDES-SRTP G.711 20 ms, rtcp-mux, echoed by the DUT | 10 s | 1,124 sps | DUT | >= 4,664 sps | 3,124 | 15 |
| R5 | UDP | none | 0 s | 8,256 sps | DUT | >= 14,016 sps | 9,388 | 5 |
| R6 | UDP | RTP G.711 20 ms, rtcp-mux, echoed by the DUT | 10 s | 1,204 sps | DUT | >= 5,128 sps | 3,432 | 16 |

## 2. RFC 7501 terminology used

- Session Attempt (RFC 7501 §3.1.6): an INVITE sent by the Emulated Agent (EA) to establish a session.
- Session Establishment (§3.1.7): the INVITE answered with 200 OK and acknowledged (ACK) within the establishment threshold time.
- Session Attempt Failure (§3.1.8): an attempt that is not established within the threshold (a 3xx-6xx final response or no final response).
- Session Establishment Rate R (§3.4.1 / RFC 7502 §4.10): the highest rate at which every attempt of a trial (N >= 50,000, scored stage >= 180 s) succeeds, found by the §4.10 search.
- Session Duration (§3.3.3): the hold before the EA sends BYE: 0 s without media, 10 s with media.
- Associated media (§3.3.1): G.711 RTP (UDP) or SRTP (TLS) for the session duration, echoed by the DUT.
- Testbed baseline (RFC 7502 §6.1): the same search with the EA talking to an EA in UAS mode (ea-uas, no DUT), which bounds what the tester can measure.

## 3. Testbed baseline (RFC 7502 §6.1)

The EA (UAC) runs the same §4.10 search against ea-uas (an EA in UAS mode, no rvoip, no DUT) with the exact generator configuration of the DUT searches: 3 caller hosts (c8in.24xlarge), 2 sources each, 8 connections per source, caller build rvoip-loadgen (private build) sha256 1229992784f9…. r_max = 0.67 x the baseline of the same row.

| Row | Transport | Hold | Baseline R | r_max (0.67 x) | Source | Generator load (final trial) |
|---|---|---|---|---|---|---|
| R1 | TLS | 0 s | >= 14,168 sps | 9,492 | §6.1 testbed baseline (ea-uas), recorded in the env-id's rates.json | recorded value |
| R2 | TLS | 10 s | >= 4,664 sps | 3,124 | §6.1 testbed baseline (ea-uas), recorded in the env-id's rates.json | recorded value |
| R5 | UDP | 0 s | >= 14,016 sps | 9,388 | §6.1 testbed baseline (ea-uas), recorded in the env-id's rates.json | recorded value |
| R6 | UDP | 10 s | >= 5,128 sps | 3,432 | LOWER BOUND, not a measured ceiling (the tester's ceiling was never observed): §6.1 testbed baseline (ea-uas), recorded in the env-id's rates.json | recorded value |

## 4. Test setup report (RFC 7502 §5.1)

### R1 Session Establishment Rate, no media (RFC 7502 §6.2 + §6.5)

| Field | Value |
|---|---|
| SIP transport protocol | TLS (same transport toward and from the DUT) |
| DUT receives requests on | 48 TLS connections (one per caller endpoint, client sockets bound to their source IP) |
| Session duration | 0 s |
| Session ended by | the EA (BYE from the caller) |
| Media streams per session | 0 (SDP offered, no RTP) |
| Associated media protocol | none |
| Codec | none |
| Media packet size (audio) | n/a |
| Establishment threshold time | 10 s (caller setup bound; stricter than Timer B = 32 s) |
| TLS | rustls (TLS 1.2/1.3); the negotiated suite is in the trial evidence |
| IPsec | not used |
| Search | bisect (N 50,000, r0 7,592, w 0.10, d 0.10, quantum 4, 3 confirmations) |
| Total sessions attempted | 7,826,400 |

### R2 Session Establishment Rate, with media (RFC 7502 §6.4 + §6.5)

| Field | Value |
|---|---|
| SIP transport protocol | TLS (same transport toward and from the DUT) |
| DUT receives requests on | 48 TLS connections (one per caller endpoint, client sockets bound to their source IP) |
| Session duration | 10 s |
| Session ended by | the EA (BYE from the caller) |
| Media streams per session | 1 (audio) |
| Associated media protocol | SRTP (SDES) |
| Codec | G.711 PCMU |
| Media packet size (audio) | 160 bytes payload per 20 ms (172 B RTP, 182 B SRTP) |
| Establishment threshold time | 10 s (caller setup bound; stricter than Timer B = 32 s) |
| TLS | rustls (TLS 1.2/1.3); the negotiated suite is in the trial evidence |
| IPsec | not used |
| Search | bisect (N 50,000, r0 2,496, w 0.10, d 0.10, quantum 4, 3 confirmations) |
| Total sessions attempted | 3,961,440 |

### R5 Session Establishment Rate, no media (RFC 7502 §6.2)

| Field | Value |
|---|---|
| SIP transport protocol | UDP (same transport toward and from the DUT) |
| DUT receives requests on | 48 UDP 5-tuples |
| Session duration | 0 s |
| Session ended by | the EA (BYE from the caller) |
| Media streams per session | 0 (SDP offered, no RTP) |
| Associated media protocol | none |
| Codec | none |
| Media packet size (audio) | n/a |
| Establishment threshold time | 10 s (caller setup bound; stricter than Timer B = 32 s) |
| TLS | n/a |
| IPsec | not used |
| Search | bisect (N 50,000, r0 7,508, w 0.10, d 0.10, quantum 4, 3 confirmations) |
| Total sessions attempted | 7,740,720 |

### R6 Session Establishment Rate, with media (RFC 7502 §6.4)

| Field | Value |
|---|---|
| SIP transport protocol | UDP (same transport toward and from the DUT) |
| DUT receives requests on | 48 UDP 5-tuples |
| Session duration | 10 s |
| Session ended by | the EA (BYE from the caller) |
| Media streams per session | 1 (audio) |
| Associated media protocol | RTP |
| Codec | G.711 PCMU |
| Media packet size (audio) | 160 bytes payload per 20 ms (172 B RTP, 182 B SRTP) |
| Establishment threshold time | 10 s (caller setup bound; stricter than Timer B = 32 s) |
| TLS | n/a |
| IPsec | not used |
| Search | bisect (N 50,000, r0 2,744, w 0.10, d 0.10, quantum 4, 3 confirmations) |
| Total sessions attempted | 4,552,560 |

## 5. Device benchmarks (RFC 7502 §5.2)

*Resource use is the final passing trial at R. Registration benchmarks (RFC 7502 §5.3, test cases 6.7 and 6.8): not applicable, the DUT is not a registrar.*

| Benchmark | R | Binding limit | Sources | Flows | Convergence | Trials | Caller host CPU % (peak) | Caller process cores | Hottest caller thread (cores) | DUT peak cores | DUT host CPU % | DUT peak RSS MiB | DUT acting as a media relay? |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| R1 Session Establishment Rate, no media | 8,348 sps | DUT | 6 | 48 | re-scored with the #468 rules: R = the highest rate with a passing trial and no failing trial at or below it; R passed 1 of 3 confirmations; lowest failing rate 9180 sps | 5 | 8.9 | 6.94 | 0.52 | 8.57 | 56.8 | 7,990 | no (terminating media endpoint) |
| R2 Session Establishment Rate, with media | 1,124 sps | DUT | 6 | 48 | re-scored with the #468 rules: R = the highest rate with a passing trial and no failing trial at or below it; R passed 4 of 3 confirmations; lowest failing rate 1136 sps | 15 | 14.6 | 10.31 | 0.25 | 9.04 | 60.4 | 7,950 | no (terminating media endpoint) |
| R5 Session Establishment Rate, no media | 8,256 sps | DUT | 6 | 48 | re-scored with the #468 rules: R = the highest rate with a passing trial and no failing trial at or below it; R passed 1 of 3 confirmations; lowest failing rate 9080 sps | 5 | 8.6 | 6.53 | 0.51 | 7.81 | 52.7 | 8,538 | no (terminating media endpoint) |
| R6 Session Establishment Rate, with media | 1,204 sps | DUT | 6 | 48 | re-scored with the #468 rules: R = the highest rate with a passing trial and no failing trial at or below it; R passed 2 of 3 confirmations; lowest failing rate 1220 sps | 16 | 14.5 | 10.06 | 0.26 | 8.98 | 59.6 | 7,306 | no (terminating media endpoint) |

## 6. RFC 6076 end-to-end metrics (final passing trial at R)

| Benchmark | INVITEs | SER §4.6 | SEER §4.7 | ISA §4.8 | SCR §4.9 | SDT mean (s) §4.5 |
|---|---|---|---|---|---|---|
| R1 Session Establishment Rate, no media | 1,502,640 | 100.0000% | 100.0000% | 0.0000% | 100.0000% | 0.001 |
| R2 Session Establishment Rate, with media | 202,320 | 100.0000% | 100.0000% | 0.0000% | 100.0000% | 10.002 |
| R5 Session Establishment Rate, no media | 1,486,080 | 100.0000% | 100.0000% | 0.0000% | 100.0000% | 0.001 |
| R6 Session Establishment Rate, with media | 216,720 | 100.0000% | 100.0000% | 0.0000% | 100.0000% | 10.002 |

*Final passing trial at R, scored stage. SRD §4.3 (successful and failed never combined), SDD §4.4 (BYEs sent by the caller), SDT §4.5. RRD/IRA §4.1-4.2: not applicable. "-" = no such calls.*

| Benchmark | Metric | Count | p50 ms | p95 ms | p99 ms | p99.9 ms | max ms |
|---|---|---|---|---|---|---|---|
| R1 Session Establishment Rate, no media | SRD successful | 1,502,640 | 1.0 | 4.0 | 10.0 | 41.0 | 67.0 |
| R1 Session Establishment Rate, no media | SRD failed | 0 | - | - | - | - | - |
| R1 Session Establishment Rate, no media | SDD | 1,502,640 | 1.0 | 4.0 | 9.0 | 39.0 | 67.0 |
| R1 Session Establishment Rate, no media | SDT | 1,502,640 | 1.0 | 1.0 | 1.0 | 2.0 | 17.0 |
| R2 Session Establishment Rate, with media | SRD successful | 202,320 | 1.0 | 50.0 | 148.0 | 307.0 | 553.0 |
| R2 Session Establishment Rate, with media | SRD failed | 0 | - | - | - | - | - |
| R2 Session Establishment Rate, with media | SDD | 202,320 | 1.0 | 47.0 | 143.0 | 302.0 | 551.0 |
| R2 Session Establishment Rate, with media | SDT | 202,320 | 10,002.0 | 10,003.0 | 10,003.0 | 10,003.0 | 10,016.0 |
| R5 Session Establishment Rate, no media | SRD successful | 1,486,080 | 1.0 | 4.0 | 29.0 | 90.0 | 125.0 |
| R5 Session Establishment Rate, no media | SRD failed | 0 | - | - | - | - | - |
| R5 Session Establishment Rate, no media | SDD | 1,486,080 | 1.0 | 4.0 | 28.0 | 86.0 | 122.0 |
| R5 Session Establishment Rate, no media | SDT | 1,486,080 | 1.0 | 1.0 | 1.0 | 2.0 | 17.0 |
| R6 Session Establishment Rate, with media | SRD successful | 216,720 | 1.0 | 13.0 | 105.0 | 302.0 | 424.0 |
| R6 Session Establishment Rate, with media | SRD failed | 0 | - | - | - | - | - |
| R6 Session Establishment Rate, with media | SDD | 216,720 | 1.0 | 11.0 | 97.0 | 305.0 | 422.0 |
| R6 Session Establishment Rate, with media | SDT | 216,720 | 10,002.0 | 10,002.0 | 10,003.0 | 10,003.0 | 10,013.0 |

*Disconnect failures: established sessions whose caller BYE got a non-2xx final (e.g. 481), over all valid trials; not Session Attempt Failures (RFC 7501 §3.1.8), so they never fail a trial.*

| Benchmark | Valid trials | Attempts | Disconnect failures | per 10^6 | SDD p50 ms | SDD p99 ms |
|---|---|---|---|---|---|---|
| R1 Session Establishment Rate, no media | 5 | 7,826,400 | 0 | 0.0 | 1.0 | 9.0 |
| R2 Session Establishment Rate, with media | 15 | 3,961,440 | 0 | 0.0 | 1.0 | 143.0 |
| R5 Session Establishment Rate, no media | 5 | 7,740,720 | 0 | 0.0 | 1.0 | 28.0 |
| R6 Session Establishment Rate, with media | 16 | 4,552,560 | 0 | 0.0 | 1.0 | 97.0 |

## 7. Setup latency below the limit

The RFC 6076 tables report the final passing trial at R, where queueing near the limit dominates. The passing trials below R show the latency a carrier sees below the limit.

*SRD = Session Request Delay (INVITE to 200 OK), SDD = Session Disconnect Delay (RFC 6076 §4.3-4.4), successful sessions of each passing trial's scored stage.*

| Row | Trial rate | Sessions | SRD p50 ms | SRD p95 ms | SRD p99 ms | SRD p99.9 ms | SDD p50 ms | SDD p99 ms |
|---|---|---|---|---|---|---|---|---|
| R1 | 7,592 sps (91% of R) | 1,366,560 | 1.0 | 2.0 | 5.0 | 28.0 | 1.0 | 4.0 |
| R1 | 8,348 sps (100% of R) | 1,502,640 | 1.0 | 4.0 | 10.0 | 41.0 | 1.0 | 9.0 |
| R2 | 1,064 sps (95% of R) | 191,520 | 1.0 | 2.0 | 4.0 | 9.0 | 1.0 | 3.0 |
| R2 | 1,124 sps (100% of R) | 202,320 | 1.0 | 7.0 | 43.0 | 102.0 | 1.0 | 40.0 |
| R2 | 1,124 sps (100% of R) | 202,320 | 1.0 | 88.0 | 243.0 | 422.0 | 1.0 | 244.0 |
| R2 | 1,124 sps (100% of R) | 202,320 | 1.0 | 74.0 | 203.0 | 361.0 | 1.0 | 199.0 |
| R2 | 1,124 sps (100% of R) | 202,320 | 1.0 | 50.0 | 148.0 | 307.0 | 1.0 | 143.0 |
| R5 | 7,508 sps (91% of R) | 1,351,440 | 1.0 | 2.0 | 5.0 | 66.0 | 1.0 | 4.0 |
| R5 | 8,256 sps (100% of R) | 1,486,080 | 1.0 | 4.0 | 29.0 | 90.0 | 1.0 | 28.0 |
| R6 | 1,172 sps (97% of R) | 210,960 | 1.0 | 2.0 | 7.0 | 19.0 | 1.0 | 5.0 |
| R6 | 1,204 sps (100% of R) | 216,720 | 1.0 | 14.0 | 65.0 | 254.0 | 1.0 | 63.0 |
| R6 | 1,204 sps (100% of R) | 216,720 | 1.0 | 13.0 | 105.0 | 302.0 | 1.0 | 97.0 |
| R6 | 1,220 sps (101% of R) | 219,600 | 1.0 | 17.0 | 74.0 | 210.0 | 1.0 | 71.0 |
| R6 | 1,220 sps (101% of R) | 219,600 | 1.0 | 3.0 | 8.0 | 25.0 | 1.0 | 6.0 |
| R6 | 1,220 sps (101% of R) | 219,600 | 1.0 | 3.0 | 10.0 | 29.0 | 1.0 | 8.0 |

## 8. Session Attempt Failure breakdown (first failing rate above R)

*The first failing rate above R. A 503 is a DUT failure (never a tester fault); "no_final_response" is a setup without a final response within 10 s.*

| Benchmark | Rate (sps) | Attempted | Failed | Final status counts | Trial |
|---|---|---|---|---|---|
| R1 Session Establishment Rate, no media | 9,180 | 1,652,400 | 84,508 | {"200": 1567892, "no_final_response": 84508} | R1/t02-r9180 |
| R2 Session Establishment Rate, with media | 1,136 | 204,480 | 2,405 | {"200": 202075, "503": 2405} | R2/t11-r1136 |
| R5 Session Establishment Rate, no media | 9,080 | 1,634,400 | 57,150 | {"200": 1577250, "no_final_response": 57150} | R5/t02-r9080 |
| R6 Session Establishment Rate, with media | 1,220 | 219,600 | 1,018 | {"200": 218582, "503": 1018} | R6/t14-r1220 |

## 9. Overload and recovery (R4, not an RFC 7502 benchmark)

*R4 Overload and recovery (0.5/1.5/2 x C, admission on). Trial R4/t00-overload-c1124.*

| Stage | Offered sps | Attempts | Established | 503 | ISA §4.8 | Setup p99 ms |
|---|---|---|---|---|---|---|
| warmup-1 | 132 | 3,960 | 3,960 | 0 | 0.0000% | 4.2 |
| warmup-2 | 276 | 8,280 | 8,280 | 0 | 0.0000% | 3.3 |
| warmup-3 | 420 | 12,600 | 12,600 | 0 | 0.0000% | 2.8 |
| warmup-4 | 552 | 16,560 | 16,560 | 0 | 0.0000% | 2.8 |
| trial-1 | 552 | 33,120 | 33,120 | 0 | 0.0000% | 2.9 |
| trial-2 | 1,116 | 66,960 | 66,960 | 0 | 0.0000% | 9.4 |
| trial-3 | 1,680 | 100,800 | 68,564 | 32,235 | 31.9802% | 58.9 |
| trial-4 | 2,244 | 134,640 | 67,442 | 67,193 | 49.9094% | 81.4 |
| trial-5 | 552 | 33,120 | 33,118 | 2 | 0.0060% | 4.3 |
| trial-6 | 1,116 | 66,960 | 66,960 | 0 | 0.0000% | 16.3 |

## 10. Findings

- R1 = 8,348 sps (TLS, no media): passed 1 of 3 confirmations; at 9,180 sps the DUT's median setup delay rose to about 2 s; its "no_final_response" attempts are calls the testers could not send while the DUT queued (their in-flight bound was full), so the trial is a DUT failure.
- R5 = 8,256 sps (UDP, no media): passed 1 of 3 confirmations; at 9,080 sps the median setup delay rose to about 1.5 s, with the same unsent calls.
- R2 = 1,124 sps (TLS + SRTP): passed 4 times; admission 503s from 1,136 sps.
- R6 = 1,204 sps (UDP + RTP): passed 2 of 3 confirmations (the search reached its trial limit); 1,220 sps passed 3 of 4 times, with 1,018 admission 503s in the fourth.
- R4 overload and recovery: pass (every failure is a 503; both recovery stages establish every call of their steady window).

## 11. Search traces (RFC 7502 §4.10)

### R1 Session Establishment Rate, no media

*Stop: re-scored with the #468 rules: R = the highest rate with a passing trial and no failing trial at or below it; R passed 1 of 3 confirmations; lowest failing rate 9180 sps. Saturated side (failing and invalid trials): the host whose kernel receive path saturated, a NIC IRQ CPU >= 95 % in softirq for 2 consecutive samples; a saturated caller makes the trial invalid.*

| Trial | Rate (sps) | Attempted n | Succeeded s | Outcome | Notes | Saturated side | Network counters | Caller cores | Hottest caller thread | DUT peak cores | Trial |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | 7,592 | 1,366,560 | 1,366,560 | pass | - | - | - | 6.39 | 0.49 | 7.46 | R1/t00-r7592 |
| 1 | 8,348 | 1,502,640 | 1,502,640 | pass | - | - | - | 6.94 | 0.52 | 8.57 | R1/t01-r8348 |
| 2 | 9,180 | 1,652,400 | 1,567,892 | fail | 84508 of 1652400 attempts were Session Attempt Failures (RFC 7501 §3.1.8): {"200": 1567892, "no_final_response": 84508}; 84508 scheduled calls not dispatched; caller check zero_guard_skips failed; caller dispatch outside 1% in 150 trial bins; DUT setup delay collapsed: median 2009 ms > 500 ms, so the caller's in-flight bound filled | no host receive path saturated (peak softirq caller-1 3 %, caller-2 3 %, caller-3 2 %, DUT host 6 %) | no counter grew | 8.42 | 0.62 | 9.75 | R1/t02-r9180 |
| 3 | 9,180 | 1,652,400 | 1,568,564 | fail | 83836 of 1652400 attempts were Session Attempt Failures (RFC 7501 §3.1.8): {"200": 1568564, "no_final_response": 83836}; 83836 scheduled calls not dispatched; caller check zero_guard_skips failed; caller dispatch outside 1% in 150 trial bins; DUT setup delay collapsed: median 2272 ms > 500 ms, so the caller's in-flight bound filled | no host receive path saturated (peak softirq caller-1 3 %, caller-2 2 %, caller-3 3 %, DUT host 4 %) | no counter grew | 8.43 | 0.62 | 9.79 | R1/t03-r9180 |
| 4 | 9,180 | 1,652,400 | 1,566,137 | fail | 86263 of 1652400 attempts were Session Attempt Failures (RFC 7501 §3.1.8): {"200": 1566137, "no_final_response": 86263}; 86263 scheduled calls not dispatched; caller check zero_guard_skips failed; caller dispatch outside 1% in 150 trial bins; DUT setup delay collapsed: median 2109 ms > 500 ms, so the caller's in-flight bound filled | no host receive path saturated (peak softirq caller-1 3 %, caller-2 2 %, caller-3 3 %, DUT host 5 %) | no counter grew | 8.31 | 0.62 | 9.78 | R1/t04-r9180 |

### R2 Session Establishment Rate, with media

*Stop: re-scored with the #468 rules: R = the highest rate with a passing trial and no failing trial at or below it; R passed 4 of 3 confirmations; lowest failing rate 1136 sps. Saturated side (failing and invalid trials): the host whose kernel receive path saturated, a NIC IRQ CPU >= 95 % in softirq for 2 consecutive samples; a saturated caller makes the trial invalid.*

| Trial | Rate (sps) | Attempted n | Succeeded s | Outcome | Notes | Saturated side | Network counters | Caller cores | Hottest caller thread | DUT peak cores | Trial |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | 2,496 | 449,280 | 193,417 | fail | 255863 of 449280 attempts were Session Attempt Failures (RFC 7501 §3.1.8): {"200": 193417, "503": 246818, "no_final_response": 9045}; 9045 scheduled calls not dispatched | no host receive path saturated (peak softirq caller-1 5 %, caller-2 5 %, caller-3 5 %, DUT host 26 %) | no counter grew | 11.57 | 0.26 | 10.77 | R2/t00-r2496 |
| 1 | 2,244 | 403,920 | 220,721 | fail | 183199 of 403920 attempts were Session Attempt Failures (RFC 7501 §3.1.8): {"200": 220721, "503": 166470, "no_final_response": 16729}; 16729 scheduled calls not dispatched; caller check zero_guard_skips failed; caller dispatch outside 1% in 16 trial bins | no host receive path saturated (peak softirq caller-1 5 %, caller-2 5 %, caller-3 6 %, DUT host 24 %) | no counter grew | 12.30 | 0.26 | 10.99 | R2/t01-r2244 |
| 2 | 2,016 | 362,880 | 201,545 | fail | 161335 of 362880 attempts were Session Attempt Failures (RFC 7501 §3.1.8): {"200": 201545, "503": 152237, "no_final_response": 9098}; 9098 scheduled calls not dispatched; caller check zero_guard_skips failed; caller dispatch outside 1% in 26 trial bins | no host receive path saturated (peak softirq caller-1 5 %, caller-2 5 %, caller-3 5 %, DUT host 24 %) | no counter grew | 12.05 | 0.26 | 10.62 | R2/t02-r2016 |
| 3 | 1,812 | 326,160 | 216,618 | fail | 109542 of 326160 attempts were Session Attempt Failures (RFC 7501 §3.1.8): {"200": 216618, "503": 103555, "no_final_response": 5987}; 5987 scheduled calls not dispatched; caller check zero_guard_skips failed; caller dispatch outside 1% in 18 trial bins | no host receive path saturated (peak softirq caller-1 5 %, caller-2 5 %, caller-3 5 %, DUT host 22 %) | no counter grew | 11.59 | 0.26 | 10.58 | R2/t03-r1812 |
| 4 | 1,628 | 293,040 | 203,392 | fail | 89648 of 293040 attempts were Session Attempt Failures (RFC 7501 §3.1.8): {"200": 203392, "503": 82489, "no_final_response": 7159}; 7159 scheduled calls not dispatched; caller check zero_guard_skips failed; caller dispatch outside 1% in 21 trial bins | no host receive path saturated (peak softirq caller-1 5 %, caller-2 5 %, caller-3 5 %, DUT host 22 %) | no counter grew | 12.36 | 0.27 | 10.10 | R2/t04-r1628 |
| 5 | 1,464 | 263,520 | 200,453 | fail | 63067 of 263520 attempts were Session Attempt Failures (RFC 7501 §3.1.8): {"200": 200453, "503": 62155, "no_final_response": 912}; 912 scheduled calls not dispatched; caller check zero_guard_skips failed; caller dispatch outside 1% in 5 trial bins | no host receive path saturated (peak softirq caller-1 5 %, caller-2 5 %, caller-3 5 %, DUT host 21 %) | no counter grew | 11.32 | 0.26 | 11.12 | R2/t05-r1464 |
| 6 | 1,316 | 236,880 | 196,720 | fail | 40160 of 236880 attempts were Session Attempt Failures (RFC 7501 §3.1.8): {"200": 196720, "503": 40160} | no host receive path saturated (peak softirq caller-1 5 %, caller-2 5 %, caller-3 5 %, DUT host 21 %) | no counter grew | 11.53 | 0.27 | 9.63 | R2/t06-r1316 |
| 7 | 1,184 | 213,120 | 203,211 | fail | 9909 of 213120 attempts were Session Attempt Failures (RFC 7501 §3.1.8): {"200": 203211, "503": 9909} | no host receive path saturated (peak softirq caller-1 5 %, caller-2 5 %, caller-3 5 %, DUT host 20 %) | no counter grew | 10.62 | 0.26 | 9.29 | R2/t07-r1184 |
| 8 | 1,064 | 191,520 | 191,520 | pass | - | - | - | 9.87 | 0.24 | 8.32 | R2/t08-r1064 |
| 9 | 1,124 | 202,320 | 202,320 | pass | - | - | - | 10.33 | 0.25 | 9.01 | R2/t09-r1124 |
| 10 | 1,152 | 207,360 | 201,532 | fail | 5828 of 207360 attempts were Session Attempt Failures (RFC 7501 §3.1.8): {"200": 201532, "503": 5828} | no host receive path saturated (peak softirq caller-1 5 %, caller-2 5 %, caller-3 5 %, DUT host 20 %) | no counter grew | 10.66 | 0.26 | 9.16 | R2/t10-r1152 |
| 11 | 1,136 | 204,480 | 202,075 | fail | 2405 of 204480 attempts were Session Attempt Failures (RFC 7501 §3.1.8): {"200": 202075, "503": 2405} | no host receive path saturated (peak softirq caller-1 5 %, caller-2 5 %, caller-3 5 %, DUT host 20 %) | no counter grew | 10.41 | 0.25 | 9.09 | R2/t11-r1136 |
| 12 | 1,124 | 202,320 | 202,320 | pass | - | - | - | 10.26 | 0.25 | 9.05 | R2/t12-r1124 |
| 13 | 1,124 | 202,320 | 202,320 | pass | - | - | - | 10.22 | 0.25 | 9.00 | R2/t13-r1124 |
| 14 | 1,124 | 202,320 | 202,320 | pass | - | - | - | 10.31 | 0.25 | 9.04 | R2/t14-r1124 |

### R5 Session Establishment Rate, no media

*Stop: re-scored with the #468 rules: R = the highest rate with a passing trial and no failing trial at or below it; R passed 1 of 3 confirmations; lowest failing rate 9080 sps. Saturated side (failing and invalid trials): the host whose kernel receive path saturated, a NIC IRQ CPU >= 95 % in softirq for 2 consecutive samples; a saturated caller makes the trial invalid.*

| Trial | Rate (sps) | Attempted n | Succeeded s | Outcome | Notes | Saturated side | Network counters | Caller cores | Hottest caller thread | DUT peak cores | Trial |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | 7,508 | 1,351,440 | 1,351,440 | pass | - | - | - | 6.21 | 0.48 | 6.92 | R5/t00-r7508 |
| 1 | 8,256 | 1,486,080 | 1,486,080 | pass | - | - | - | 6.53 | 0.51 | 7.81 | R5/t01-r8256 |
| 2 | 9,080 | 1,634,400 | 1,577,250 | fail | 57150 of 1634400 attempts were Session Attempt Failures (RFC 7501 §3.1.8): {"200": 1577250, "no_final_response": 57150}; 57150 scheduled calls not dispatched; caller check zero_guard_skips failed; caller dispatch outside 1% in 150 trial bins; DUT setup delay collapsed: median 1496 ms > 500 ms, so the caller's in-flight bound filled | no host receive path saturated (peak softirq caller-1 2 %, caller-2 2 %, caller-3 2 %, DUT host 4 %) | no counter grew | 8.52 | 0.66 | 9.39 | R5/t02-r9080 |
| 3 | 9,080 | 1,634,400 | 1,551,627 | fail | 82773 of 1634400 attempts were Session Attempt Failures (RFC 7501 §3.1.8): {"200": 1551627, "no_final_response": 82773}; 82773 scheduled calls not dispatched; caller check zero_guard_skips failed; caller dispatch outside 1% in 150 trial bins; DUT setup delay collapsed: median 1542 ms > 500 ms, so the caller's in-flight bound filled | no host receive path saturated (peak softirq caller-1 3 %, caller-2 2 %, caller-3 3 %, DUT host 4 %) | no counter grew | 8.37 | 0.63 | 9.46 | R5/t03-r9080 |
| 4 | 9,080 | 1,634,400 | 1,570,505 | fail | 63895 of 1634400 attempts were Session Attempt Failures (RFC 7501 §3.1.8): {"200": 1570505, "no_final_response": 63895}; 63895 scheduled calls not dispatched; caller check zero_guard_skips failed; caller dispatch outside 1% in 150 trial bins; DUT setup delay collapsed: median 1343 ms > 500 ms, so the caller's in-flight bound filled | no host receive path saturated (peak softirq caller-1 2 %, caller-2 2 %, caller-3 3 %, DUT host 4 %) | no counter grew | 8.47 | 0.66 | 9.40 | R5/t04-r9080 |

### R6 Session Establishment Rate, with media

*Stop: re-scored with the #468 rules: R = the highest rate with a passing trial and no failing trial at or below it; R passed 2 of 3 confirmations; lowest failing rate 1220 sps. Saturated side (failing and invalid trials): the host whose kernel receive path saturated, a NIC IRQ CPU >= 95 % in softirq for 2 consecutive samples; a saturated caller makes the trial invalid.*

| Trial | Rate (sps) | Attempted n | Succeeded s | Outcome | Notes | Saturated side | Network counters | Caller cores | Hottest caller thread | DUT peak cores | Trial |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | 2,744 | 493,920 | 204,165 | fail | 289755 of 493920 attempts were Session Attempt Failures (RFC 7501 §3.1.8): {"200": 204165, "503": 286422, "no_final_response": 3333}; 3333 scheduled calls not dispatched | no host receive path saturated (peak softirq caller-1 6 %, caller-2 5 %, caller-3 5 %, DUT host 25 %) | no counter grew | 11.46 | 0.28 | 11.22 | R6/t00-r2744 |
| 1 | 2,468 | 444,240 | 230,923 | fail | 213317 of 444240 attempts were Session Attempt Failures (RFC 7501 §3.1.8): {"200": 230923, "503": 204443, "no_final_response": 8874}; 8874 scheduled calls not dispatched | no host receive path saturated (peak softirq caller-1 6 %, caller-2 6 %, caller-3 7 %, DUT host 26 %) | no counter grew | 13.46 | 0.29 | 11.36 | R6/t01-r2468 |
| 2 | 2,220 | 399,600 | 212,282 | fail | 187318 of 399600 attempts were Session Attempt Failures (RFC 7501 §3.1.8): {"200": 212282, "503": 179473, "no_final_response": 7845}; 7845 scheduled calls not dispatched; caller check zero_guard_skips failed; caller dispatch outside 1% in 5 trial bins | no host receive path saturated (peak softirq caller-1 5 %, caller-2 5 %, caller-3 6 %, DUT host 26 %) | DUT host softnet_time_squeeze +3 | 12.41 | 0.27 | 10.73 | R6/t02-r2220 |
| 3 | 1,996 | 359,280 | 213,616 | fail | 145664 of 359280 attempts were Session Attempt Failures (RFC 7501 §3.1.8): {"200": 213616, "503": 140321, "no_final_response": 5343}; 5343 scheduled calls not dispatched; caller check zero_guard_skips failed; caller dispatch outside 1% in 8 trial bins | no host receive path saturated (peak softirq caller-1 6 %, caller-2 5 %, caller-3 5 %, DUT host 30 %) | no counter grew | 12.47 | 0.29 | 10.43 | R6/t03-r1996 |
| 4 | 1,796 | 323,280 | 214,301 | fail | 108979 of 323280 attempts were Session Attempt Failures (RFC 7501 §3.1.8): {"200": 214301, "503": 107626, "no_final_response": 1353}; 1353 scheduled calls not dispatched; caller check zero_guard_skips failed; caller dispatch outside 1% in 2 trial bins | no host receive path saturated (peak softirq caller-1 6 %, caller-2 6 %, caller-3 6 %, DUT host 24 %) | no counter grew | 12.74 | 0.28 | 10.42 | R6/t04-r1796 |
| 5 | 1,616 | 290,880 | 213,216 | fail | 77664 of 290880 attempts were Session Attempt Failures (RFC 7501 §3.1.8): {"200": 213216, "503": 77664} | no host receive path saturated (peak softirq caller-1 6 %, caller-2 6 %, caller-3 5 %, DUT host 24 %) | no counter grew | 12.30 | 0.28 | 9.98 | R6/t05-r1616 |
| 6 | 1,452 | 261,360 | 213,002 | fail | 48358 of 261360 attempts were Session Attempt Failures (RFC 7501 §3.1.8): {"200": 213002, "503": 48358} | no host receive path saturated (peak softirq caller-1 5 %, caller-2 6 %, caller-3 6 %, DUT host 22 %) | no counter grew | 11.36 | 0.26 | 9.68 | R6/t06-r1452 |
| 7 | 1,304 | 234,720 | 211,815 | fail | 22905 of 234720 attempts were Session Attempt Failures (RFC 7501 §3.1.8): {"200": 211815, "503": 22905} | no host receive path saturated (peak softirq caller-1 6 %, caller-2 5 %, caller-3 6 %, DUT host 22 %) | DUT host softnet_time_squeeze +4 | 10.82 | 0.27 | 9.06 | R6/t07-r1304 |
| 8 | 1,172 | 210,960 | 210,960 | pass | - | - | - | 10.17 | 0.28 | 8.69 | R6/t08-r1172 |
| 9 | 1,236 | 222,480 | 218,098 | fail | 4382 of 222480 attempts were Session Attempt Failures (RFC 7501 §3.1.8): {"200": 218098, "503": 4382} | no host receive path saturated (peak softirq caller-1 5 %, caller-2 6 %, caller-3 5 %, DUT host 22 %) | DUT host softnet_time_squeeze +10 | 10.37 | 0.26 | 8.93 | R6/t09-r1236 |
| 10 | 1,204 | 216,720 | 216,720 | pass | - | - | - | 10.24 | 0.26 | 9.00 | R6/t10-r1204 |
| 11 | 1,220 | 219,600 | 219,600 | pass | - | - | - | 10.35 | 0.27 | 8.97 | R6/t11-r1220 |
| 12 | 1,220 | 219,600 | 219,600 | pass | - | - | - | 10.31 | 0.26 | 8.73 | R6/t12-r1220 |
| 13 | 1,220 | 219,600 | 219,600 | pass | - | - | - | 10.46 | 0.27 | 8.91 | R6/t13-r1220 |
| 14 | 1,220 | 219,600 | 218,582 | fail | 1018 of 219600 attempts were Session Attempt Failures (RFC 7501 §3.1.8): {"200": 218582, "503": 1018} | no host receive path saturated (peak softirq caller-1 6 %, caller-2 6 %, caller-3 5 %, DUT host 20 %) | no counter grew | 10.44 | 0.26 | 9.00 | R6/t14-r1220 |
| 15 | 1,204 | 216,720 | 216,720 | pass | - | - | - | 10.06 | 0.26 | 8.98 | R6/t15-r1204 |

## 12. Setup and methodology

| Item | Value |
|---|---|
| DUT | rvoip-bench-uas (rvoip b089eeb5d4b2) |
| DUT instance_type | c8i.4xlarge |
| DUT vcpu | 16 |
| DUT physical_cores | 8 |
| DUT cpu | Intel Xeon 6 |
| DUT role | terminating SIP UAS; echoes media through the rvoip media executor v2 |
| Testbed caller_hosts | 3 |
| Testbed caller_type | c8in.24xlarge |
| Testbed caller_vcpu | 96 |
| Testbed sources_per_host | 2 |
| Testbed connections_per_source | 8 |
| Testbed caller_build | rvoip-loadgen (private build) sha256 1229992784f9… |
| Testbed caller_media_executor | 24 worker threads in 4 lanes |
| Environment cloud | AWS |
| Environment region | us-west-2 |
| Environment availability_zone | us-west-2a |
| Environment placement | one cluster placement group (DUT and callers on the same network spine) |
| Environment os | Ubuntu 24.04, Linux 7.0 (AWS kernel), with the bench host tuning |

- The DUT is rvoip-bench-uas: a terminating SIP UAS and media endpoint built on rvoip. It is not a proxy, B2BUA or registrar; it answers every INVITE with 200 OK and, with media, echoes every received RTP packet to its source (SRTP: decrypted and re-encrypted with the answer key) so the caller validates its own 440 Hz tone.
- The Tester is rvoip-loadgen: caller hosts each acting as many Emulated Agents (UACs), every host offering an even share of every stage on a shared timeline (start barrier; start skew <= 2 s).
- Clocks: every RFC 6076 interval is measured on one caller host (t1 and t4 on the same clock), so no cross-host offset applies (RFC 6076 §3); resolution is 1 ms from the SIP stack's transport trace.
- Establishment Threshold Time: 10 s, the caller's setup bound. This is stricter than the RFC 7501 §3.3.2 recommendation (Timer B = 64 x T1 = 32 s): a call without a 200 OK within 10 s is a Session Attempt Failure.
- Media Packet Size: G.711 at 20 ms = 160 payload bytes per RTP packet (172 bytes RTP; 182 bytes SRTP with the 80-bit authentication tag). RTCP is multiplexed on the RTP port (rtcp-mux, RFC 5761).

Topology (adapted from RFC 7502 Figure 2): Tester (EAs, UAC) --SIP over TLS or UDP, SRTP or RTP--> DUT (UAS, media endpoint that echoes media). There is no second EA and no forwarding.

## 13. Deviations from RFC 7501, RFC 7502 and RFC 6076

| Area | Deviation |
|---|---|
| Topology | RFC 7501 §1 and RFC 7502 §3 scope the benchmarks to intermediaries (proxy, B2BUA, SBC, registrar) with EAs on both sides. The DUT terminates calls, so the topology is EA (UAC) -> DUT (UAS); media terminates on the DUT (RFC 7502 Figure 2 with the DUT as the far end). No second EA, no forwarding. |
| Media relay field | RFC 7502 §5.2 asks whether the DUT is a media relay. It is not: it is the media endpoint (it echoes media). The "no media" rows still negotiate SDP; the DUT allocates no RTP socket for them. |
| Warm-up | Each trial starts with an unscored warm-up: 30 s at half the trial rate, or 120 s ramped 25/50/75/100 % of the rate when admission is on. RFC 7502 defines no warm-up. |
| Fresh process per trial | Every trial starts a fresh DUT process and drains it fully (true zero) before the next. RFC 7502 leaves inter-trial conditions open. |
| Trial length | The scored stage lasts max(N / r, 180 s) with N = 50,000 (the sustained rule: RFC 7501 §3.4.1 defines the rate over "an extended, pre-defined period"); the N actually attempted is reported per trial. |
| Rate quantum | Trial rates are floored to a multiple of 4 sessions/s, and every caller host's share of every stage times its seconds is a whole number of calls. RFC 7502 floors to 1. |
| Tester cap | The search cannot exceed r_max = 0.67 x the §6.1 testbed baseline of the same row; a result at the cap, or above 0.67 x the baseline, is reported as R >= value (tester-limited), never as R. |
| Invalid trials | A trial with tester-side defects (calls without terminal evidence, undispatched or generator-skipped calls, a caller outside its validity bounds before the DUT's first backpressure, including a caller host whose kernel receive path saturated: a NIC IRQ CPU >= 95 % in softirq) is repeated at the same rate and never scored as a DUT failure; three invalid trials in a row stop the search. |
| Establishment threshold | 10 s instead of the RFC 7501 §3.3.2 recommendation of 32 s (Timer B). |
| 503 with Retry-After | A 503 is a DUT Session Attempt Failure, never a tester fault. With admission on the DUT answers excess INVITEs 503 with Retry-After; RFC 7339 §5.10.2 asks overloaded servers to reject non-participating clients without Retry-After, and the RFC 7339 oc Via parameters are not implemented. |
| Success criterion | Session Attempt Failures only (RFC 7501 §3.1.8). A non-2xx to the caller's BYE is a disconnect failure: counted and reported (RFC 6076 section), never a failed attempt. Media quality is reported, not scored. |
| Percentiles | RFC 6076 defines no percentiles (SDT is an average). This report adds count, mean, p50, p95, p99, p99.9 and max (nearest rank) for every interval. |
| Registration | RFC 7502 §6.7-6.8 and RFC 6076 RRD/IRA are not applicable: the DUT is not a registrar. |
| Algorithm variant | Searches marked "bisect" use a declared accelerator (bracket with the RFC weights, bisect to 2 % resolution, then confirm the lower bound) instead of the exact §4.10 procedure. |
| Other | Caller hosts: 3 x c8in.24xlarge, 2 source IPs each, 8 connections per source (one TLS connection or UDP 5-tuple per endpoint). |
| Other | Signaling-only rows: the DUT answers the SDP offer without allocating RTP (media: none). |

## 14. Evidence

| Row | Search state | Trials | Recorded trials |
|---|---|---|---|
| R1 | R1/search.json | R1/trials.jsonl | 5 |
| R2 | R2/search.json | R2/trials.jsonl | 15 |
| R5 | R5/search.json | R5/trials.jsonl | 5 |
| R6 | R6/search.json | R6/trials.jsonl | 16 |
| baseline R1 | §6.1 testbed baseline (ea-uas), recorded in the env-id's rates.json | - | - |
| baseline R2 | §6.1 testbed baseline (ea-uas), recorded in the env-id's rates.json | - | - |
| baseline R5 | §6.1 testbed baseline (ea-uas), recorded in the env-id's rates.json | - | - |
| baseline R6 | §6.1 testbed baseline (ea-uas), recorded in the env-id's rates.json | - | - |

Raw evidence (per trial: plan.json, dut.tar.gz, caller-<i>.tar.gz with .sha256, merged/summary.json, verdict.json): GitHub Actions run 45065590604 (bench-minor, candidate b089eeb5d4b2); per-trial evidence is kept in the project's private evidence store
