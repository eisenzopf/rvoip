"""Drive one RFC 7502 §4.10 search as consecutive trials (library).

Ported from the E74 harness (rfc7502_search.py). The E74
script ran each trial as a subprocess of its run planner; here the
orchestrator passes `run_trial`, a callable that runs one complete trial
(fresh DUT process, start barrier, load, drain, evidence extraction,
summarize_trial) and returns a TrialResult.

    result = run_search(spec, state_path, trials_path, run_trial)

`spec` (one row's search; unknown keys are ignored):

    row                   "R1"                    recorded with the result
    criterion             "rfc" | "full"          (rfc7502.evaluate_trial)
    r0                    initial rate (sps); default r_max
    n                     50000                   RFC 7502 N
    w, d                  0.10, max(0.10, w/2)    increase / decrease weights
    confirmations         10                      successes at or below old_r to converge
    algorithm             "rfc7502" | "bisect"
    r_max | baseline_sps  tester cap, or the §6.1 baseline it derives from (r_max = 0.67 x baseline)
    r_min, quantum        10, 4
    max_trials            60                      valid trials before the search stops
    max_hours             8                       wall-clock budget of this invocation
    warmup_seconds        30 (120 with warmup_ramp)
    warmup_ramp           false                   true: 25/50/75/100 % ramp (admission-on rows)
    minimum_trial_seconds 180                     the sustained rule
    hosts                 1                       caller hosts (whole per-host shares)
    expected              optional sanity-gate expectation (sps)
    tester_cap_ok         false                   a tester-limited result does not hold the lane
    halt_on_missing_dut_evidence  true            a trial without DUT summary.json halts like a failed check
    trial_overhead_seconds 900                    start/drain/upload time added to a trial's need

State: `state_path` (rfc7502.Search JSON, resumed when it exists); one JSON
line per recorded trial appended to `trials_path`; `verdict.json` in each
TrialResult.trial_dir.

Halt rules (the search ends without a result): a trial whose DUT evidence
shows `dut_true_zero` or `dut_clean_shutdown` false (or missing, unless
halt_on_missing_dut_evidence is false); three invalid trials in a row
(rfc7502.INVALID_STREAK_STOP); max_trials; max_hours. A trial that offered
no session (no summary, or n = 0) is void: not recorded; a second void trial
in a row pauses the search (state kept, resumable) with `halted` set. An
absolute `deadline` (epoch s, e.g. the lane's time cap) pauses the search
before a trial that would not fit. Standard library only.
"""
from dataclasses import dataclass, field
import json
from pathlib import Path
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
import rfc7502  # noqa: E402

HALT_CHECKS = ('dut_true_zero', 'dut_clean_shutdown')
VOID_HALT = 2
DEFAULT_MAX_HOURS = 8
DEFAULT_TRIAL_OVERHEAD_SECONDS = 900
SANITY_BAND = (0.5, 1.2)


@dataclass
class TrialResult:
    """What run_trial returns for one trial."""
    summary: dict | None = None          # merged/summary.json (summarize_trial.summarize), None if none was produced
    run: str | None = None               # trial evidence path relative to the lane root, e.g. "R1/t03-r4000"
    trial_dir: str | Path | None = None  # local trial directory; verdict.json is written there when set
    rc: int = 0                          # the orchestrator's own exit status for the trial
    wall_seconds: float | None = None
    note: str | None = None
    extra: dict = field(default_factory=dict)   # copied into the trial's recorded detail


def _log(*args):
    print(time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()), *args, file=sys.stderr, flush=True)


def halt_reasons(summary, spec=None):
    """DUT evidence that stops a search: the DUT, not the offered rate, is broken."""
    spec = spec or {}
    checks = (summary or {}).get('checks') or {}
    reasons = []
    for key in HALT_CHECKS:
        if key not in checks:
            continue   # not applicable (testbed baseline against ea-uas)
        if checks[key] is False:
            reasons.append('%s is false' % key)
        elif checks[key] is None and spec.get('halt_on_missing_dut_evidence', True):
            reasons.append('%s missing (%s)' % (key, ((summary or {}).get('checks_missing') or {}).get(key, 'no evidence')))
    return reasons


def sanity_gate(result, expected, tester_cap_ok=False):
    """R outside [0.5, 1.2] x expected holds the lane; a tester-cap result holds unless tester_cap_ok."""
    if expected is None:
        return None
    value = result.get('result_sps')
    r_max = (result.get('parameters') or {}).get('r_max')
    if result.get('tester_limited'):
        return {'expected': expected, 'binding': 'tester cap', 'hold': not tester_cap_ok,
                'reason': 'R >= tester cap %s sps; expected %.0f' % (r_max, expected)}
    low, high = SANITY_BAND
    if value is None or not low * expected <= value <= high * expected:
        return {'expected': expected, 'binding': 'unknown', 'hold': True,
                'reason': 'R %s outside [%.1f, %.1f] x expected %.0f' % (value, low, high, expected)}
    return {'expected': expected, 'binding': 'DUT', 'hold': False,
            'reason': 'R %s within [%.1f, %.1f] x expected %.0f' % (value, low, high, expected)}


def trial_need_seconds(spec, profile):
    """Wall seconds one trial needs: its stages, the hold and the orchestration overhead."""
    hold = (spec.get('hold_ms') or 0) / 1000.0
    return sum(stage['seconds'] for stage in profile['stages']) + hold + float(
        spec.get('trial_overhead_seconds', DEFAULT_TRIAL_OVERHEAD_SECONDS))


def _attempted(summary):
    if not summary:
        return 0
    return sum((row.get('whole_stage') or {}).get('offered') or 0 for row in rfc7502.trial_stage_rows(summary))


def _write_json(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + '.tmp')
    tmp.write_text(json.dumps(value, indent=1) + '\n')
    tmp.replace(path)


def run_search(spec, state_path, trials_path, run_trial, clock=time, sanity=None, log=_log, deadline=None):
    """Run (or resume) one search; returns the result dict:
    rfc7502.Search.summary() plus row, criterion, runs, halted, paused and sanity_gate."""
    state_path, trials_path = Path(state_path), Path(trials_path)
    criterion = spec.get('criterion', 'rfc')
    row = spec.get('row')
    resumed = state_path.exists()
    search = rfc7502.Search.load(state_path) if resumed else rfc7502.new_search(spec)
    state_path.parent.mkdir(parents=True, exist_ok=True)
    search.save(state_path)
    budget_end = clock.time() + 3600 * float(spec.get('max_hours', DEFAULT_MAX_HOURS))
    log('RFC7502_START', row, json.dumps(search.summary()['parameters']), 'criterion', criterion,
        'resumed' if resumed else 'new')
    halted, paused, voids = None, False, 0
    while not search.done:
        now = clock.time()
        if now > budget_end:
            search.stop('max_hours (%s) reached' % spec.get('max_hours', DEFAULT_MAX_HOURS))
            break
        rate = search.next_rate()
        index = len(search.s['trials'])
        profile = rfc7502.profile_for_spec(spec, rate)
        need = trial_need_seconds(spec, profile)
        if deadline is not None and deadline - now < need:
            paused = True
            log('RFC7502_PAUSED', row, 'deadline in %.0f s < %.0f s for the next trial' % (deadline - now, need))
            break
        log('RFC7502_TRIAL_START', row, 'index', index, 'rate', rate, 'stages', json.dumps(profile['stages']))
        started = clock.time()
        result = run_trial(rate, index, profile)
        if not isinstance(result, TrialResult):
            raise TypeError('run_trial must return a search.TrialResult')
        summary = result.summary
        if not _attempted(summary):
            # A trial that offered no session (a guard refusal, a host down) is not a trial: nothing is recorded;
            # a second one in a row pauses the search instead of burning trials.
            voids += 1
            log('RFC7502_VOID_TRIAL', row, 'index', index, 'rate', rate, 'rc', result.rc, 'consecutive', voids,
                result.note or '')
            if voids >= VOID_HALT:
                halted = '%d consecutive trials offered no session (n = 0); last: %s' % (voids, result.note or 'rc=%s' % result.rc)
                paused = True
                break
            continue
        voids = 0
        attempted, succeeded, valid, detail = rfc7502.evaluate_trial(summary, criterion)
        halts = halt_reasons(summary, spec)
        warmups = [stage for stage in profile['stages'] if not stage['name'].startswith('trial-')]
        detail.update(run=result.run, rc=result.rc, note=result.note,
                      wall_seconds=round(result.wall_seconds if result.wall_seconds is not None else clock.time() - started),
                      warmup_seconds=sum(stage['seconds'] for stage in warmups), warmup_ramp=bool(spec.get('warmup_ramp')),
                      scored_seconds=sum(stage['seconds'] for stage in profile['stages'] if stage['name'].startswith('trial-')),
                      halt_reasons=halts or None, **(result.extra or {}))
        outcome = search.record(rate, attempted, succeeded, valid, detail)
        search.save(state_path)
        record = dict(search.s['trials'][-1], row=row)
        trials_path.parent.mkdir(parents=True, exist_ok=True)
        with open(trials_path, 'a') as stream:
            stream.write(json.dumps(record) + '\n')
        if result.trial_dir:
            _write_json(Path(result.trial_dir) / 'verdict.json', {
                'outcome': outcome, 'row': row, 'index': index, 'rate': rate, 'criterion': criterion,
                'attempted': attempted, 'succeeded': succeeded, 'failed': attempted - succeeded if outcome == 'fail' else 0,
                'reasons': (detail.get('invalid_reasons') if outcome == 'invalid' else
                            ['%d Session Attempt Failures: %s' % (attempted - succeeded,
                                                                 json.dumps(detail.get('final_status_counts') or {}, sort_keys=True))]
                            if outcome == 'fail' else []),
                'halt_reasons': halts, 'next_rate': search.next_rate(), 'search_done': search.done})
        log('RFC7502_TRIAL', row, 'index', index, outcome, 'n=%d s=%d' % (attempted, succeeded), 'run', result.run or '-',
            'next', search.next_rate())
        if halts:
            halted = '; '.join(halts)
            search.stop('halt: ' + halted)
            search.save(state_path)
            break
    search.save(state_path)
    result = dict(search.summary(), row=row, criterion=criterion, halted=halted, paused=paused and not search.done,
                  runs=[(t.get('detail') or {}).get('run') for t in search.s['trials']])
    if search.done:
        gate = sanity(result) if sanity else sanity_gate(result, spec.get('expected'), spec.get('tester_cap_ok', False))
        if gate:
            result['sanity_gate'] = gate
            log('RFC7502_SANITY', row, gate['reason'], '(hold)' if gate.get('hold') else '')
    log('RFC7502_RESULT', row, json.dumps({k: result[k] for k in ('result_sps', 'tester_limited', 'stop_reason', 'trials',
                                                                   'halted', 'paused')}))
    return result
