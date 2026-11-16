# scripts/bench: RFC 7501/7502 search, metrics and report

The analysis side of the RFC 7502 release benchmark
(`crates/sip/rvoip-sip/docs/RFC7502_BENCHMARK.md`): trial plans, the §4.10
search, trial scoring, RFC 6076 metrics, caller validity, the per-trial
summary and the rendered report. Ported from the E74 harness; standard
library only, Python 3.10+.

| Module | Role |
|---|---|
| `rfc7502.py` | `Search` (§4.10 exact + bisect), trial profiles, quantum and host shares, tester cap, `evaluate_trial`, `trial_verdict` |
| `rfc6076.py` | SER, SEER, ISA, SCR, SRD (successful and failed apart), SDD, SDT, disconnect failures |
| `caller_policy.py` | caller thread policy (48/96/192 vCPU) and the caller validity predicate |
| `merge_callers.py` | N caller hosts merged onto one clock (library + CLI) |
| `summarize_trial.py` | one trial directory to `merged/summary.json` (library + CLI) |
| `search.py` | `run_search`: drives one search through the orchestrator's `run_trial` |
| `rfc_report.py`, `rfc_report_ietf.py` | report `.md` + `.html` from a report spec |
| `host_sampler.py` | per-host sampler writing `resources.jsonl` |
| `disk_guard.py`, `evidence.py` | free-disk guard; sha256 sidecars, manifests, tar helpers, layout names |

Tests: `python3 -m unittest discover -s scripts/bench/tests`.

## Orchestrator API

### Running a search

```python
import search, summarize_trial, rfc7502

def run_trial(rate: int, index: int, profile: dict) -> search.TrialResult:
    # fresh DUT, barrier, load the profile, drain, extract dut/ and caller-<i>/ ...
    path, summary = summarize_trial.write(trial_dir)          # trial_dir/merged/summary.json
    return search.TrialResult(summary=summary, run='R1/t%02d-r%d' % (index, rate), trial_dir=trial_dir)

result = search.run_search(spec, 'R1/search.json', 'R1/trials.jsonl', run_trial)
```

```text
search.run_search(spec, state_path, trials_path, run_trial, clock=time, sanity=None, log=_log, deadline=None) -> dict
search.TrialResult(summary=None, run=None, trial_dir=None, rc=0, wall_seconds=None, note=None, extra={})
search.halt_reasons(summary, spec=None) -> list[str]
search.sanity_gate(result, expected, tester_cap_ok=False) -> dict | None
search.trial_need_seconds(spec, profile) -> float
```

`spec` keys (unknown keys ignored): `row`, `criterion` (`rfc`|`full`), `r0`,
`n` (50000), `w` (0.10), `d`, `confirmations` (10), `algorithm`
(`rfc7502`|`bisect`), `r_max` or `baseline_sps` (r_max = 0.67 x baseline,
floored to the quantum; `r_max_cap` optional), `r_min` (10), `quantum` (4),
`resolution` (0.02), `max_trials` (60), `max_hours` (8), `warmup_seconds`
(30; 120 with `warmup_ramp`), `warmup_ramp` (false), `minimum_trial_seconds`
(180), `max_stage_seconds` (600), `hosts` (caller hosts, 1), `hold_ms`,
`expected` (sanity gate), `tester_cap_ok`, `halt_on_missing_dut_evidence`
(true), `trial_overhead_seconds` (900).

`run_trial(rate, index, profile)` gets `profile = rfc7502.profile_for_spec(spec, rate)`:
`{"name", "stages": [{"name": "warmup"|"warmup-k"|"trial-k", "cps", "seconds"}], "transient_stages", "hosts", "basis"}`.
Every stage's `cps x seconds` is divisible by `hosts`; each caller host runs
`rfc7502.host_shares(cps, seconds, hosts)[i]` (cps may be fractional, calls are
whole). Write the profile into the trial's `plan.json` (below).

The result is `Search.summary()` plus `row`, `criterion`, `runs`, `halted`
(halt text or None), `paused` (bool) and `sanity_gate` (when the search ended
and an expectation or `sanity` callable exists). Side effects: `state_path`
(Search JSON, resumed when present; a resumed search continues from its saved
next rate, `r0` is ignored), one line per recorded trial appended to
`trials_path`, and `<trial_dir>/verdict.json` when `trial_dir` is set.

`deadline` (absolute epoch seconds, e.g. the lane's expiry) pauses the search
before a trial that would not fit (stages + hold + `trial_overhead_seconds`);
`max_hours` is the budget of this invocation and stops the search.

### rfc7502

```text
rfc7502.Search(r, n=50000, w=0.10, d=None, confirmations=10, algorithm='rfc7502', max_trials=60,
               r_max=None, r_min=10, quantum=1, resolution=0.02, floor_weight=0.10)
    .next_rate() -> int | None   .record(rate, attempted, succeeded, valid=True, detail=None) -> 'pass'|'fail'|'invalid'
    .stop(reason)   .summary() -> dict   .save(path)   Search.load(path)   .done   .result
rfc7502.new_search(spec) -> Search                 # quantum 4 by default
rfc7502.tester_cap(baseline_sps, factor=0.67, quantum=4, cap=None) -> int | None
rfc7502.trial_profile(rate, n=50000, warmup_seconds=30, minimum_seconds=180, max_stage_seconds=600,
                      warmup_ramp=False, hosts=1) -> dict
rfc7502.profile_for_spec(spec, rate, hosts=None) -> dict
rfc7502.host_shares(cps, seconds, hosts) -> list[dict]      # ValueError unless every share is whole
rfc7502.validate_shares(profile, hosts) -> True
rfc7502.evaluate_trial(summary, criterion='rfc') -> (attempted, succeeded, valid, detail)
rfc7502.trial_verdict(summary, criterion='rfc') -> {"outcome": "pass"|"fail"|"invalid", "reasons", ...}
```

### summarize_trial, merge_callers

```text
summarize_trial.summarize(trial_dir, criterion=None) -> dict     # adds "evaluation" when criterion is given
summarize_trial.write(trial_dir, out_path=None, criterion=None) -> (Path, dict)   # default merged/summary.json
python3 summarize_trial.py <trial-dir> [--out PATH] [--criterion rfc|full]

merge_callers.caller_dirs(trial_dir) -> list[Path]
merge_callers.load_caller(directory) -> dict
merge_callers.merge(callers) -> dict
python3 merge_callers.py <trial-dir> [--out DIR]                 # default merged/caller/
```

Memory and time: about 6 KB of Python objects per call; a 180 s trial at
5,000 sps (~975,000 calls) needs ~6 GB and ~30 s. Run it on a caller-class
host.

### caller_policy

```text
caller_policy.policy_for(logical_cpus) -> dict      # 48, 96 (c8in.24xlarge) or 192
caller_policy.validate(env, logical_cpus) -> int    # ValueError when the budget exceeds the host
caller_policy.predicate(bins, scheduler_lateness_ms, samples, report, backpressure_second=None,
                        physical_cores=None, process_core_limit=None, run_start_epoch=None,
                        judged_from_second=0) -> dict
caller_policy.combine(by_caller, backpressure_second=None) -> dict
```

Env: `RVOIP_LOADGEN_SIGNALING_THREADS`, `RVOIP_LOADGEN_LANES`,
`RVOIP_LOADGEN_WORKERS_PER_LANE`, `RVOIP_MEDIA_WORKER_THREADS`,
`RVOIP_MEDIA_RUNTIME_LANES`. 96 vCPU: 8 + 8 x 6 + 40, 8 media lanes.

### Report

```text
rfc_report.render(spec, out_dir, stem='rfc7502-report') -> (md_path, html_path)
rfc_report_ietf.render(spec, out_dir, stem='rfc7502-ietf-report') -> (md_path, html_path)
python3 rfc_report.py SPEC.json OUT_DIR | --template OUT_DIR
python3 rfc_report_ietf.py SPEC.json OUT_DIR
```

Report spec:

```json
{"title": "...", "report_date": "2026-11-16", "status": "final", "profile": "minor",
 "evidence_root": "<local copy of bench/<lane-run-id>/>",
 "evidence_uri": "s3://<evidence-bucket>/bench/<lane-run-id>/",
 "dut": {"name": "rvoip-bench-uas", "version": "0.4.1", "git_sha": "<sha>", "binary_sha256": "...",
         "instance_type": "c8i.4xlarge", "vcpu": 16, "physical_cores": 8, "config": {}},
 "testbed": {"caller_hosts": 2, "caller_type": "c8in.24xlarge", "sources_per_host": 2,
             "connections_per_source": 8, "caller_build": "...", "caller_sha256": "..."},
 "environment": {"env_id": "...", "region": "...", "ami": "..."},
 "rows": [{"row": "R1", "search": "R1/search.json", "trials": "R1/trials.jsonl"}],
 "baseline": {"R1": {"search": "baseline/R1/search.json"},
              "R5": {"result_sps": 8548, "tester_limited": false, "source": "perf-baselines/.../rates.json"}},
 "overload": {"row": "R4", "trial": "R4/t00-overload"},
 "summary": [], "findings": [], "deviations": [], "assumptions": []}
```

The DUT renders as `rvoip-bench-uas (rvoip <sha12>)`. Every recorded trial's
`detail.run` is a trial directory relative to `evidence_root`; its
`merged/summary.json` supplies the RFC 6076 tables and the load figures.
Missing evidence renders `pending`; an empty distribution renders `-`.
A result reads `>= R (tester-limited)` at r_max or above 0.67 x the row's
baseline. A media baseline counts only when its final trial's
`media_echo_observed` is true.

### Host tools

```text
host_sampler.py --pid <pid> --out resources.jsonl [--interval 2] [--max-samples N]
host_sampler.run(pid, out, interval=2.0, max_samples=None) -> int
disk_guard.require_free(path, minimum_gib=20) -> float      # raises DiskGuardError
disk_guard.prune_unpacked(directories, min_bytes=2 MiB, keep=()) -> int
disk_guard.py <path> [--min-gib 20]                          # exit 1 below the minimum
evidence.trial_dir_name(index, rate) -> 't03-r4000'; evidence.trial_path(row, index, rate) -> 'R1/t03-r4000'
evidence.write_sha256(path) / verify_sha256(path) / manifest(dir) / write_manifest(dir) / verify_manifest(dir, rows)
evidence.pack(dir, out_tar_gz, arcname=None) -> sha256     # deterministic, writes .sha256
evidence.unpack(archive, dest, require_sidecar=False)       # refuses '..', absolute paths, escaping links
```

## Trial directory input (what summarize_trial reads)

```
<trial-dir>/plan.json
<trial-dir>/dut/{ready.json, stats.jsonl, summary.json, metrics-*.prom, resources.jsonl}
<trial-dir>/caller-<i>/{caller.json, events.jsonl, report.json, analysis.json, resources.jsonl}
```

`plan.json` (written by the orchestrator; scoring needs `profile`):

```json
{"row": "R1", "rate": 4000, "index": 3, "criterion": "rfc", "media": "none|rtp|srtp",
 "transport": "tls|udp", "hold_ms": 0, "dut_kind": "dut|ea-uas",
 "profile": {"stages": [{"name": "warmup", "cps": 2000, "seconds": 30},
                        {"name": "trial-1", "cps": 4000, "seconds": 180}],
             "transient_stages": ["warmup"]},
 "caller_hosts": {"caller-1": {"physical_cores": 48}}}
```

Without `plan.json` the stages are unnamed, no stage is a scored `trial-*`
stage and the trial is invalid. `dut_kind: "ea-uas"` (testbed baseline)
marks the DUT drain checks not applicable. `caller_hosts.<role>.physical_cores`
is the fallback when the caller's `resources.jsonl` lacks `physical_cores`.

### Assumptions the Rust ports must honour

Caller (`rvoip-loadgen`, unchanged from the E74 caller):

- `events.jsonl`: one JSON object per line, `{"timestamp": RFC 3339, "fields": {"event", "elapsed_ms", "call_id", ...}}`;
  `elapsed_ms` counts from the caller process start. Events read:
  `run_started` (once, before the first call), `call_scheduled` (`scheduler_lateness_ms`),
  `invite_sent` (first one per call is the dispatch time), `call_established`,
  `call_ended` (the observation: `sip_status`, `outcome` pass/fail/inconclusive, `reason`
  (`generator_capacity` / `generator_media_ports` for skipped calls, `audio_validation`,
  `setup_or_no_audio`), `failure_class` (`timeout`, `dialog_network`, `transport`,
  `caller_operation_timeout` mark DUT setup timeouts), `setup_ms`, `call_model`
  (`signaling-only` | `caller-hold`), `first_audio_ms`, `audio_match`, `max_audio_gap_ms`,
  `duration_ms`, and `rfc6076` with the `rfc.rs` fields `final_status`, `first_response_status`,
  `srd_ms`, `srd_kind`, `sdd_ms`, `sdt_ms`, `bye_final_status`, `completed`, `trace_complete`,
  `invite_transmissions`).
- Call ids are `<run_id>-<n>`; `caller.json` carries `run_id` and `stages: [{cps, seconds}]`
  (this host's share, same order and seconds as `plan.json`), `calls`.
- Every call is journalled (`journal_sample` 1). A sampled journal makes the caller's
  per-call evidence missing (the trial is invalid), never estimated.
- `report.json`: `planned`, `attempted`, `generator_skipped`, `interrupted`,
  `shutdown_outcome` ("ok"), `caller_watchdogs`, `caller_cleanup_failures`, `call_model`,
  `hold_ms`, `transport` (`tls_srtp` | `udp_rtp`), `rfc6076_trace`, `connections_per_source`,
  `endpoints`, and `sources: [{assigned, spilled_in, preferred_full, peak_active, failed, shutdown: {outcome}}]`.
  The per-second dispatch bins and the lateness distribution are derived from `events.jsonl`
  (as E74 did); report.json bins/histograms, if added, are not read.
- A media call passes only when the caller validated the DUT's echo of its tone for the
  hold (`outcome: pass`, `call_model: caller-hold`).

DUT (`rvoip-bench-uas`, RFC7502_BENCHMARK.md §3):

- `stats.jsonl` lines and `summary.json` carry the counters by their metric names without
  the `rvoip_bench_uas_` prefix; labelled counters are objects keyed by the label value:
  `"responses_sent_total": {"200": n, "503": n}`, `"byes_answered_total": {"200": n}`.
  `summary.json` may hold them at the top level or under `"counters"`. Each stats line has `"t"`.
- `summary.json`: `true_zero` and `clean_shutdown` as booleans (anything else reads as missing),
  `drain_seconds`, `started_at`, `ended_at`.
- `ready.json`: `pid`, `version`, `git_sha`, `config_sha256`, `transport`, `media`.
- `metrics-*.prom`: the snapshot with the most `invites_received_total` is used; it is the
  fallback for the 503 count (`responses_sent_total{code="503"}`).
- 503s during the drain (after `finish`) come after the load timeline and are not a
  backpressure marker.

`resources.jsonl` on every host: `host_sampler.py --pid <process under test>`.

## summary.json (`rvoip-bench-trial-summary/1`)

| Key | Content |
|---|---|
| `schema`, `trial_dir`, `generated_at` | identity |
| `plan` | `row, rate, index, criterion, media, transport, hold_ms, dut_kind, search` from plan.json (None when absent) |
| `evidence` | `callers` (roles), `callers_missing`, `missing` ({role: [files]}, `trial: ["plan.json"]`) |
| `stage_problems` | stage naming/consistency problems (plan vs callers' summed stages) |
| `run_start_epoch` | caller-1's `run_started` instant (the merged clock origin) |
| `caller_start_skew_seconds` | spread of the hosts' `run_started` instants |
| `trial_window` | scored `trial-*` window: `start_seconds, end_seconds, start_epoch, end_epoch` |
| `stage_summaries[]` | `name, index, requested_cps, cps, seconds, start_seconds, end_seconds, planned_offers, scheduled, transient, whole_stage, scored_window_seconds, scored` |
| `…whole_stage` / `…scored` | `offered, dispatched, terminal_observations, missing_terminal_observations, admitted, rejected_503, full, generator_skipped, admitted_incomplete, full_of_offered_pct, offered_cps, admitted_cps, setup_ms{p50,p95,p99,p99_9,max,count}, first_audio_ms{…}, rejection_503_duration_ms, rfc_session_ok, rfc_attempt_ok, rfc_disconnect_failures, rfc6076{…}` (calls keyed by INVITE send time; `scored` skips the first 15 s of stages >= 60 s) |
| `bins_1s[]` | merged 1 s bins: `t, stage, requested_cps, offered, dispatched, admitted, rejected_503, full, guard_skips, setup_p99_ms` |
| `backpressure` | `first_second, first_503_second, first_timeout_second, dut_first_503_second` (s from the run start) |
| `caller_validity` | combined predicate: `caller_valid, caller_valid_before_backpressure, checks{dispatch_within_1pct_per_bin_before_backpressure, schedule_to_write_p99_at_most_5ms, process_cpu_at_most_limit, no_thread_above_0_8_core, zero_guard_skips, clean_shutdown, source_pools_balanced}, dispatch_misses[{t, requested, dispatched, caller}], scheduler_lateness_p99_ms, process_peak_cores, process_core_limit, thread_peak_cores, hottest_thread, source_pools, missing_inputs, backpressure_second` |
| `caller_validity_by_caller` | the same per host (own clock) |
| `callers.<role>` | `start_offset_seconds, missing, report{verdict, planned, attempted, …, rfc6076_trace}` |
| `resource.<role>` (`dut`, `caller-<i>`) | `samples, scored_samples, logical_cpus, physical_cores, sampled_peak_cores, scored_mean_cores, scored_peak_cores, host_peak_cpu_pct, scored_host_mean_cpu_pct, peak_rss_bytes, peak_threads, scored_thread_groups{family: {mean_cores, peak_cores}}` |
| `generator` | `hosts, sources, connections_per_source, endpoints, transport, call_model, hold_ms` |
| `rfc6076` | `all_calls` and `scored` (trial-* stages) metric blocks, `caller_report_trace, call_model, hold_ms, transport` |
| `setup_ms`, `final_status_counts`, `disconnect_failures{all_calls, scored}` | scored-stage readouts |
| `media` | None (unknown), `{"applicable": false}`, or `established, validated_audio, audio_failures, validated_pct, audio_match{…}, first_audio_ms{…}, max_audio_gap_ms{…}, dut_rtp_packets_received, dut_rtp_packets_sent, dut_srtp_unprotect_failures` |
| `dut` | `present, missing, ready, version, git_sha, config_sha256, transport, media, true_zero, clean_shutdown, drain_seconds, started_at, ended_at, counters{…}, responses_503, responses_503_source, admission_rejected, first_503_epoch, first_503_second, peak_active_calls, peak_cpu_cores, peak_cpu_cores_source, peak_host_cpu_pct, peak_rss_bytes, logical_cpus, physical_cores, metrics_snapshot` |
| `checks` | `caller_evidence_complete, caller_all_planned_attempts, caller_clean_shutdown, caller_start_skew, caller_valid_before_backpressure, scored_stage_present, terminal_evidence_complete, rfc6076_trace_coverage, dut_ready, dut_true_zero, dut_clean_shutdown`, media rows: `media_echo_observed, media_all_audio_validated`, SRTP rows: `dut_srtp_unprotect_failures_zero`. Each is true, false, or None (input missing) |
| `checks_missing` | {check: why its input is missing} |
| `checks_not_applicable` | {check: why} (e.g. DUT drain checks for an ea-uas baseline) |
| `verdict` | `pass` (all checks true), `fail` (any false), `incomplete` (none false, some missing): evidence quality, recorded as data |
| `evaluation` | `rfc7502.trial_verdict` when a criterion was given |

## How a trial maps to pass / fail / invalid

`rfc7502.evaluate_trial(summary, criterion)` scores the `trial-*` stages
(whole stage, by INVITE send time):

- **attempted** = calls scheduled in the scored stages; **succeeded** = calls
  answered 200 (criterion `rfc`, RFC 7501 §3.1.8) or passed by the call model
  (criterion `full`: media rows need the validated echo).
- **invalid** (tester evidence; repeated at the same rate, 3 in a row stop the
  search): caller evidence missing (`caller_evidence_complete` false), calls
  without a terminal observation, scheduled calls not dispatched, or a caller
  validity check failing on the scored stage before the DUT's first
  backpressure (per-bin dispatch misses are judged per host, ignoring the
  warm-up, the stage-switch bins and bin-edge shortfalls the neighbouring bin
  makes up; stages under 10 s are judged on their total), no `trial-*` stage.
  Exception: when the attempts sent include Session Attempt Failures and every
  reason is about dispatch, shutdown or pool balance (or, with DUT 503s among
  fully dispatched attempts, the schedule-to-write check), the shortfall is the
  DUT's and the trial **fails**.
- **pass**: valid and succeeded == attempted (> 0).
- **fail**: valid and at least one Session Attempt Failure (a 3xx-6xx final,
  a 503 included, or no final within 10 s). A non-2xx to the caller's BYE is a
  disconnect failure: counted (`detail.disconnect_failures`), never a failure.
- Failed or missing checks other than the above (media, trace coverage, DUT
  checks) are recorded in the detail and never decide the trial.

`search.run_search` additionally:

- **void** (not recorded): no summary or no attempt in the scored stage; a
  second void in a row pauses the search (`paused`, `halted`).
- **halt** after recording: `checks.dut_true_zero` or `checks.dut_clean_shutdown`
  false, or missing unless `halt_on_missing_dut_evidence` is false. The search
  ends without a result.
