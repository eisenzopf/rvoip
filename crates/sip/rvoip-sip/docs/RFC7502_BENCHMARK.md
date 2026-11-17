# RFC 7501/7502 SIP benchmark (release gate)

Status: in development (Phase 3 of the release-process plan). This document is
the design and the contract between the four parts of the benchmark:

| Part | Where | Published |
|---|---|---|
| DUT: `rvoip-bench-uas` | `crates/tools/rvoip-bench-uas` | `publish = false` |
| Caller (Emulated Agent, UAC) `rvoip-loadgen` and the testbed UAS `ea-uas` | `crates/tools/rvoip-loadgen` | `publish = false` |
| Search, metrics, report, lane orchestrator | `scripts/bench/` | n/a |
| Testbed infrastructure | `infra/aws/bench/` (own Terraform root and state) | n/a |

The design is a port of the E74 benchmark that Vapi ran on its media worker
(E74 plan and final report: c8i.4xlarge, R1 5,384 sps TLS, R5 4,948 sps UDP).
The caller and `ea-uas` are copied from Vapi's code and are held back until
Vapi signs off.

## 1. What is measured

- **Session Establishment Rate R** (RFC 7501 §3.4.1): the highest Session
  Attempt Rate the DUT sustains with **zero** Session Attempt Failures
  (RFC 7501 §3.1.8: a 3xx-6xx final, or no 200 within the establishment
  threshold, here 10 s).
- **RFC 6076** SRD (successful and failed kept apart), SDD, SDT, SER, SEER,
  ISA and SCR, from the caller's per-call wire timeline.
- Rows (E74 numbering):

| Row | Transport | Media | Hold | Used by |
|---|---|---|---|---|
| R1 | TLS | none (signaling only) | 0 s | release, minor, full |
| R2 | TLS | SDES-SRTP G.711 20 ms, rtcp-mux, echoed by the DUT | 10 s | release, minor, full |
| R4 | TLS | SRTP | 10 s | minor (overload 0.5/1.5/2 x C, then recovery, admission on) |
| R5 | UDP | none | 0 s | release, minor, full |
| R6 | UDP | RTP G.711 20 ms, rtcp-mux, echoed | 10 s | minor, full |

The DUT is a terminating UAS and media endpoint (RFC 7502 Figure 2 adapted:
EA -> DUT, media terminates on the DUT). Registration benchmarks do not apply.

## 2. Trial rules (kept from E74)

- **Sustained**: each trial is an unscored warm-up of 30 s at r/2 (120 s ramped
  25/50/75/100 % when admission is on), then a scored stage of
  max(N / r, 180) s at r. N = 50,000.
- **Fresh DUT process per trial**, started and drained by the orchestrator.
- **Flows**: >= 32. Default: 2 caller hosts x 2 source IPs x 8 connections
  (one TLS connection, or one UDP 5-tuple, per endpoint; TLS client sockets
  bound to their source IP).
- **Rate quantum 4**: every per-host stage share times its seconds is a whole
  number of calls.
- **Caller validity**, judged on the scored stage and before the DUT's first
  backpressure (first 503 or first setup timeout):
  dispatch within max(1, 1 %) per 1 s bin with neighbour-bin make-up;
  schedule-to-write p99 <= 5 ms; caller process CPU <= 0.75 x the host's
  physical cores, measured during generation only; no caller thread above
  0.8 core. An invalid trial reruns at the same rate; three invalid in a row
  stop the search.
- **Tester cap** r_max = 0.67 x the §6.1 testbed baseline of the same row; a
  result at the cap reads "R >= cap (tester-limited)".
- **Failure classes**: a 503 is a DUT failure (never a tester fault); a non-2xx
  (for example 481) to the caller's BYE is a *disconnect failure*, counted and
  reported, not a Session Attempt Failure.
- **Ops guards**: disk guard (refuse a trial under 20 GiB free on any host;
  about 8 GiB of evidence per 180 s trial), port reservations
  (`ip_local_reserved_ports` covers SIP and RTP ranges), and a media-plane
  check (an R2/R6 baseline only counts if the UAS echoed RTP).

## 3. DUT contract: `rvoip-bench-uas`

```
rvoip-bench-uas --config <dut.json>
```

`dut.json` (unknown fields rejected):

```json
{
  "run_dir": "/var/lib/rvoip-bench/run",
  "transport": "tls" | "udp",
  "bind": "10.90.200.10:5061",
  "advertise_ip": "10.90.200.10",
  "tls_cert": "/etc/rvoip-bench/tls/dut.crt",
  "tls_key": "/etc/rvoip-bench/tls/dut.key",
  "media": "none" | "rtp" | "srtp",
  "rtcp_mux": true,
  "rtp_port_range": [10000, 64999],
  "admission": {"mode": "off"} | {"mode": "max-cps", "max_cps": 4200, "retry_after_secs": 2},
  "max_concurrent_calls": 60000,
  "metrics_listen": "0.0.0.0:9464",
  "stats_interval_ms": 1000,
  "drain_timeout_secs": 120,
  "worker_threads": null
}
```

Behaviour:

- Answers every INVITE with 200 OK (SDP answer: `RTP/SAVP` + `a=crypto` for
  `srtp`, `RTP/AVP` for `rtp` and `none`; `a=rtcp-mux` when offered and
  enabled). `media: none` still answers SDP but allocates no RTP socket.
- With media, echoes every received RTP packet back to its source (SRTP:
  decrypt and re-encrypt with the answer key), so the caller validates its own
  tone. RTCP is muxed on the RTP port.
- Answers BYE with 200 OK; absorbs ACK.
- Admission `off`: never sheds. `max-cps`: GCRA with a 1 s burst; excess
  INVITEs get `503 Service Unavailable` with `Retry-After`.
- Writes `<run_dir>/ready.json` once listening:
  `{"pid", "started_at", "transport", "listen", "media", "metrics_listen",
  "version", "git_sha", "config_sha256"}`.
- Appends one JSON line per `stats_interval_ms` to `<run_dir>/stats.jsonl`
  with the counters below plus `"t"` (unix seconds, float).
- **Drain**: when `<run_dir>/finish` appears it stops accepting new calls
  (503), waits until active calls, pending setups and pending cleanups are all
  zero or `drain_timeout_secs` passes, then writes `<run_dir>/summary.json`
  and exits 0 (exit 3 if the drain timed out).
- `summary.json`: final counters, `"true_zero"` (active = pending_setups =
  pending_cleanups = open dialogs = 0 after drain), `"clean_shutdown"`,
  `"drain_seconds"`, `"started_at"`, `"ended_at"`.
- Prometheus text on `GET /metrics` (`metrics_listen`), all prefixed
  `rvoip_bench_uas_`:

| Metric | Type |
|---|---|
| `invites_received_total` | counter |
| `responses_sent_total{code}` | counter |
| `calls_established_total` | counter |
| `byes_received_total`, `byes_answered_total{code}` | counter |
| `active_calls`, `pending_setups`, `pending_cleanups`, `open_dialogs` | gauge |
| `connections` (TLS) / `udp_flows` | gauge |
| `admission_rejected_total` | counter |
| `rtp_packets_received_total`, `rtp_packets_sent_total`, `srtp_unprotect_failures_total` | counter |
| `process_cpu_seconds_total`, `process_resident_memory_bytes`, `process_threads` | process |

## 4. Caller contract: `rvoip-loadgen` (held)

`rvoip-loadgen <caller.json>`; the E74 `Settings` minus every Vapi-specific
field. Kept unchanged: `run_id`, `scenario`, `bind_ip`, `bind_ips`,
`connections_per_source`, `calls`, `cps`, `stages`, `max_concurrent`, `codec`,
`rtcp_mux_required`, `media_port_start`/`end`, `target_sip_port`,
`local_sip_port`, `tls_cert_path`, `tls_key_path`, `tls_ca_path`, `hold_ms`,
`signaling_only`, `transport` (`tls-srtp` | `udp-rtp`), `journal_sample`,
`soak`, `output`, `linger_seconds`, `did`.

Changed:
- `targets: [{"name": "dut", "host": "10.90.200.10"}]` (IP or DNS name; the
  TLS server name is `host`).
- `contact_host` replaces `generator_hostname` (any host or IP; secondary
  sources use their IP literal).
- Media is always echo-validated (E74 `echo_reference`): the DUT and `ea-uas`
  reflect the caller's 440 Hz tone. The recording-fixture model, `fixture`,
  `fixture_repeats`, `routes_file`, `routing_seed`, `retry_503`,
  `retry_cps_fraction`, `borrowed_sources` and `capture_audio` are removed.
- `limits` replaces the research-lane checks:
  `{"max_cps_per_source": 6000, "max_cps": 40000, "max_calls_per_source": 6000000,
  "max_concurrent_per_source": 35000}` (defaults shown). Targets must be
  loopback or private (RFC 1918) addresses unless `"allow_public_target": true`.
- Env: `RVOIP_LOADGEN_SIGNALING_THREADS`, `RVOIP_LOADGEN_LANES`,
  `RVOIP_LOADGEN_WORKERS_PER_LANE`, `RVOIP_MEDIA_WORKER_THREADS`; thread names
  `loadgen-signaling`, `loadgen-call-<n>`, `loadgen-validation`.

Outputs are unchanged from E74: `<output>/events.jsonl`, `<output>/report.json`
(with `rfc6076_trace`, per-call `rfc6076`, the lateness histogram and the
per-second dispatch bins), `<output>/analysis.json`, and Prometheus on the
caller's `/metrics`.

`ea-uas <ea.json>`: the E74 RFC 7502 §6.1 testbed UAS (raw rustls, no rvoip),
unchanged apart from names. Used only by the `baseline` profile.

## 5. Host agent and evidence layout

Each trial is one *run* `<lane-run-id>/<row>/t<k>-r<rate>`. Evidence in S3
under `s3://<evidence-bucket>/bench/<lane-run-id>/`:

```
lane.json                       lane inventory (instances, IPs, AMI, builds)
bin/<name> + bin/<name>.sha256  the binaries under test
<row>/search.json               rfc7502.Search state (resumable)
<row>/trials.jsonl              one line per trial
<row>/t<k>-r<rate>/
    plan.json                   trial plan (stages, per-host shares, configs)
    barrier/ready-<host>        start-barrier markers;  barrier/go.json
    dut.tar.gz(+.sha256)        ready.json stats.jsonl summary.json metrics-*.prom dut.log resources.jsonl host/*
    caller-<i>.tar.gz(+.sha256) caller.json events.jsonl report.json analysis.json caller.log resources.jsonl host/*
    merged/summary.json         merged + summarized view (scripts/bench/summarize_trial.py)
    verdict.json                pass | fail | invalid, with reasons
report/rfc7502-report.md|html   rendered report (minor/full/baseline)
receipt.json                    the gate receipt (docs/BENCH_GATE_INTERFACE.md)
```

`resources.jsonl` is written by `scripts/bench/host_sampler.py` (2 s samples
of host CPU, and per-thread CPU of the process under test).

## 6. Infrastructure

`infra/aws/bench` is its own Terraform root with its own local state, so it
can never change the release-qualification resources in `infra/aws`. It reads
the release VPC by tag and creates:

- the lane subnet: the release VPC's public subnet in an AZ that offers both
  `c8i.4xlarge` and `c8in.24xlarge` (checked), and a security group open only between bench hosts
  (SIP 5060-5100 UDP/TCP, RTP 10000-64999 UDP, metrics 9464-9465 TCP);
- a cluster placement group;
- launch templates `rvoip-bench-dut` (c8i.4xlarge) and `rvoip-bench-caller`
  (c8in.24xlarge, one secondary private IP per extra source), both on the
  bench AMI, IMDSv2 only, root gp3 (DUT 100 GB, caller 300 GB);
- the bench host role (SSM core, S3 read/write on `bench/` of the evidence
  bucket);
- a vCPU quota check (`L-1216C47A`) as a Terraform `check` block; the
  orchestrator repeats it before every launch.

Instances are **not** Terraform resources: the orchestrator launches them from
the templates for one lane run, tags them `managed-by=github-actions` and
`rvoip-expires-at` (so the release janitor reaps a stranded lane), and
terminates them at the end. Lanes run strictly serially.

The templates name the release VPC's public subnet in `var.availability_zone`
(us-west-2b), but a lane is not tied to it. `run_lane.py --az <az>` launches
the whole lane in another AZ with no Terraform change: it reuses the
templates' interfaces with the same-named public subnet of that AZ
(`rvoip-release-public-<az>`) and overrides their placement. The cluster
placement group is empty between lanes, so it takes the AZ of each lane's first
instance. `pin_env.py --az` records the AZ in `env.json`. From then on every
run on that env-id launches there, and a different `--az` is a usage error, as
is a resumed leg in an AZ other than its first leg's. A baseline and the runs
gated on it therefore never span AZs. When an AZ runs short of capacity,
re-pin a new env-id in another AZ and re-baseline; never fall back between
AZs mid-environment. The CI orchestrator policy allows `RunInstances` only in
the template subnet, so `--az` works from an admin session until that policy
lists the other subnets (a Terraform change).

`minor` and `full` also hold an On-Demand Capacity Reservation while each leg
runs (`--reserve-capacity`; on by default for those two, off for the others,
`--no-reserve-capacity` turns it off). Before a leg launches, the orchestrator
creates one targeted reservation per instance type (1 DUT + N callers; one
reservation when the types match) in the lane's AZ and cluster placement group,
and launches only into it. A capacity miss while reserving is INFRA before
anything is recorded, exactly like a launch miss; there is no fallback to
unreserved launches. Every exit path cancels the reservations: a verdict,
INFRA, a `--max-wall` stop, and SIGINT/SIGTERM. A resumed leg reserves again.
Each reservation also ends by itself at the lane's `rvoip-expires-at`, and
every lane start cancels reservations tagged for its env-id that no live run
holds, so a crashed orchestrator cannot keep billing. A reservation in use
costs nothing beyond the instances it holds. The receipt lists the
reservations under `environment.capacity_reservations`. The orchestrator
policy has no Capacity Reservation actions yet (the same Terraform item as the
subnets).

The AMI (`scripts/bench/ami/bake.py`) is Ubuntu 24.04 plus the E74 host
tuning, applied at every boot by `rvoip-bench-tune.service`: XPS, NIC IRQs on
the SMT sibling of the busy core, 500 us RX/TX coalescing, RX ring 8,192,
conntrack 4M, `ip_local_reserved_ports`, THP `always`, file limits.

## 7. Profiles

| Profile | Rows | Method | Wall clock | Cost |
|---|---|---|---|---|
| `release` | R1, R5, R2 | 3 x 0.95 R + 1 x 1.05 R per row; RFC 6076 latency at 0.9 R | ~3.5 h | ~$50 |
| `minor` | R1, R2, R5, R6 bisect; R4 overload | bisect (w/d steps, 2 % resolution, 3 confirmations); renders the IETF report | ~11 h | ~$150 |
| `full` | as minor with exact §4.10 | quarterly | ~2-3 days | ~$1k |
| `baseline` | R1, R2, R5, R6 against `ea-uas` | §4.10 EA-to-EA; sets r_max | ~2 h | ~$30 |

Recorded R values live in
`crates/sip/rvoip-sip/perf-baselines/rfc7502/<env-id>/rates.json`, keyed by
the environment id (instance types, AMI id, caller build sha256).
