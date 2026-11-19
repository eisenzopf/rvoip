"""RFC 7502 §4.10 benchmarking algorithm, trial plans and trial scoring.

Ported from the E74 harness (rfc7502.py). The goal (RFC 7502 §4.10) is the
largest Session Attempt Rate R (sessions/s) that the DUT handles with zero
failures while N sessions are attempted at R. The algorithm is iterative, not a
bisection:

    success (s == n):  if r > old_r: old_r = r  else: count += 1 (stop at 10)
                       r = floor(r + w*r)
    failure:           r = floor(r - d*r); d = max(0.10, d/2); w = max(0.10, w/2)
    result:            R = max(r, old_r) once count reaches 10

`Search(algorithm='rfc7502')` is that procedure exactly (defaults N = 50000,
w = 0.10, d = max(0.10, w/2); RFC 7502 Appendix A converges to 458 sps for a
DUT whose limit is 460). `algorithm='bisect'` is an accelerator and a declared
deviation: bracket with the same w/d steps, bisect to a resolution, then
confirm the lower bound with `confirmations` zero-failure trials.

Rules layered on the RFC (each one is a declared deviation in the report;
RFC 7502 specifies none of them):
* every trial is its own run (fresh DUT process, full drain between trials);
* a warm-up precedes the scored trial stage: 30 s at r/2, or 120 s ramped
  25/50/75/100 % when admission is on (not scored);
* the scored stage lasts max(N / r, 180 s) (the sustained rule; N actually
  attempted is reported per trial);
* rates are floored to a multiple of `quantum` (default 4) and every caller
  host's share of every stage times its seconds is a whole number of calls;
* the tester cap r_max (0.67 x the §6.1 testbed baseline of the same row)
  bounds r; convergence at the cap is reported as R >= r_max
  (tester-limited), never as R;
* a trial whose tester evidence is invalid (missing terminal observations,
  undispatched or generator-skipped calls, an invalid caller during the scored
  stage) is repeated at the same rate, never counted as a DUT failure; a third
  consecutive invalid trial stops the search;
* the caller's validity is judged on the scored stage only, and a one-bin
  dispatch shortfall that the neighbouring bin makes up (a 1 s bin-edge
  artefact) is not a dispatch miss;
* the trial's failed checks (media and the rest) are recorded with every
  trial as data; under criterion `rfc` they never decide the trial.

Trial success criteria (`criterion`):
  rfc   RFC 7501 §3.1.8: no Session Attempt Failure (every INVITE got a 200
        within the establishment threshold). A non-2xx to the caller's BYE is
        a disconnect failure: counted and reported, never a failed attempt.
  full  rfc plus the call model's own pass (media rows: the caller validated
        the DUT's echo of its tone for the whole hold).

Standard library only (Python 3.10+).
"""
import json
import math
from pathlib import Path

RFC_DEFAULTS = {'n': 50000, 'w': 0.10, 'confirmations': 10, 'floor_weight': 0.10}
QUANTUM = 4
MINIMUM_TRIAL_SECONDS = 180
WARMUP_SECONDS = 30
RAMPED_WARMUP_SECONDS = 120
MAX_STAGE_SECONDS = 600
INVALID_STREAK_STOP = 3
TESTER_CAP_FACTOR = 0.67
WARMUP_RAMP_STEPS = 4
RAMP_CPS_QUANTUM = 4
SHORT_TRIAL_SECONDS = 10


class Search:
    """Resumable RFC 7502 §4.10 search state (JSON-serializable)."""

    def __init__(self, r, n=50000, w=0.10, d=None, confirmations=10, algorithm='rfc7502', max_trials=60,
                 r_max=None, r_min=10, quantum=1, resolution=0.02, floor_weight=0.10):
        if algorithm not in ('rfc7502', 'bisect'):
            raise ValueError('algorithm must be rfc7502 or bisect')
        if not (int(r) >= 1 and n >= 1 and 0 < w <= 1.0 and confirmations >= 1 and quantum >= 1):
            raise ValueError('invalid search parameters')
        d = max(floor_weight, w / 2) if d is None else d
        self.s = {'algorithm': algorithm, 'n': int(n), 'r': self._q(int(r), quantum), 'w': w, 'd': d,
                  'w0': w, 'd0': d, 'old_r': 0, 'count': 0, 'confirmations': int(confirmations),
                  'floor_weight': floor_weight, 'max_trials': int(max_trials), 'r_max': r_max, 'r_min': int(r_min),
                  'quantum': int(quantum), 'resolution': resolution, 'lo': None, 'hi': None, 'confirm_left': None,
                  'invalid_streak': 0, 'trials': [], 'done': False, 'result': None, 'stop_reason': None,
                  'tester_limited': False}
        if r_max is not None:
            self.s['r'] = min(self.s['r'], self._q(int(r_max), quantum))

    @staticmethod
    def _q(rate, quantum):
        return max(quantum, (int(rate) // quantum) * quantum)

    # ---- persistence
    def to_json(self):
        return json.dumps(self.s, indent=1)

    @classmethod
    def from_json(cls, text):
        search = cls.__new__(cls)
        search.s = json.loads(text)
        return search

    def save(self, path):
        path = Path(path)
        tmp = path.with_suffix(path.suffix + '.tmp')
        tmp.write_text(self.to_json() + '\n')
        tmp.replace(path)

    @classmethod
    def load(cls, path):
        return cls.from_json(Path(path).read_text())

    # ---- algorithm
    @property
    def done(self):
        return self.s['done']

    @property
    def result(self):
        return self.s['result']

    def next_rate(self):
        """The rate of the next trial, or None when the search has ended."""
        return None if self.s['done'] else self.s['r']

    def _stop(self, reason, result=None):
        self.s.update(done=True, stop_reason=reason, result=result)

    def stop(self, reason):
        """End the search without a result (halt rule, time budget)."""
        if not self.s['done']:
            self._stop(reason, None)

    def _set_rate(self, rate):
        s = self.s
        rate = self._q(rate, s['quantum'])
        if s['r_max'] is not None:
            rate = min(rate, self._q(s['r_max'], s['quantum']))
        s['r'] = rate
        if rate < s['r_min']:
            self._stop('rate below r_min (%d)' % s['r_min'], None)

    def record(self, rate, attempted, succeeded, valid=True, detail=None):
        """Record one trial at `rate`: `attempted` sessions (n), `succeeded`
        (s); success is s == n with n > 0. Returns 'pass'|'fail'|'invalid'."""
        s = self.s
        if s['done']:
            raise RuntimeError('search already ended')
        if rate != s['r']:
            raise ValueError('trial rate %s is not the requested %s' % (rate, s['r']))
        if not valid or not attempted:
            s['invalid_streak'] += 1
            s['trials'].append({'index': len(s['trials']), 'rate': rate, 'attempted': attempted, 'succeeded': succeeded,
                                'outcome': 'invalid', 'detail': detail})
            if s['invalid_streak'] >= INVALID_STREAK_STOP:
                self._stop('%d consecutive invalid trials at %d sps (tester evidence)' % (INVALID_STREAK_STOP, rate), None)
            return 'invalid'
        s['invalid_streak'] = 0
        ok = succeeded == attempted
        s['trials'].append({'index': len(s['trials']), 'rate': rate, 'attempted': attempted, 'succeeded': succeeded,
                            'failed': attempted - succeeded, 'outcome': 'pass' if ok else 'fail', 'detail': detail,
                            'w': s['w'], 'd': s['d'], 'old_r': s['old_r'], 'count': s['count']})
        if s['algorithm'] == 'rfc7502':
            self._rfc(rate, ok)
        else:
            self._bisect(rate, ok)
        if not s['done'] and len([t for t in s['trials'] if t['outcome'] != 'invalid']) >= s['max_trials']:
            self._stop('max_trials (%d) without convergence; highest passing rate %s' % (s['max_trials'], s['old_r'] or None),
                       None)
        return 'pass' if ok else 'fail'

    def _rfc(self, rate, ok):
        """RFC 7502 §4.10 max_sps(), one send_traffic() result at a time."""
        s = self.s
        if ok:
            if rate > s['old_r']:
                s['old_r'] = rate
            else:
                s['count'] += 1
                if s['count'] >= s['confirmations']:
                    self._finish(max(rate, s['old_r']), 'converged: %d successes at or below old_r' % s['count'])
                    return
            # At the tester cap r stays at r_max: later successes there count
            # toward convergence (r == old_r) and _finish reports R >= r_max.
            self._set_rate(math.floor(rate + s['w'] * rate))
        else:
            self._set_rate(math.floor(rate - s['d'] * rate))
            s['d'] = max(s['floor_weight'], s['d'] / 2)
            s['w'] = max(s['floor_weight'], s['w'] / 2)

    def _bisect(self, rate, ok):
        """Accelerator (declared deviation): bracket, bisect, confirm."""
        s = self.s
        if s['confirm_left'] is not None:
            if ok:
                s['confirm_left'] -= 1
                if s['confirm_left'] <= 0:
                    self._finish(rate, 'bisect: %d confirmation trials passed' % s['confirmations'])
                return
            # A confirmation failed: this rate is not sustainable.
            s['confirm_left'] = None
            s['hi'] = rate
            s['lo'] = max([t['rate'] for t in s['trials'] if t['outcome'] == 'pass' and t['rate'] < rate], default=None)
        elif ok:
            s['lo'] = rate if s['lo'] is None else max(s['lo'], rate)
            s['old_r'] = max(s['old_r'], rate)
        else:
            s['hi'] = rate if s['hi'] is None else min(s['hi'], rate)
        lo, hi = s['lo'], s['hi']
        if lo is None:
            self._set_rate(math.floor(rate - s['d'] * rate))
            return
        cap = self._q(s['r_max'], s['quantum']) if s['r_max'] is not None else None
        if hi is None:
            if cap is not None and lo >= cap:
                s['tester_limited'] = True
                s['confirm_left'] = s['confirmations']
                self._set_rate(lo)
                return
            self._set_rate(math.floor(lo + s['w'] * lo))
            return
        if hi - lo <= max(s['quantum'], math.ceil(s['resolution'] * lo)):
            s['confirm_left'] = s['confirmations']
            self._set_rate(lo)
            return
        self._set_rate((lo + hi) // 2)

    def _finish(self, rate, reason):
        s = self.s
        cap = self._q(s['r_max'], s['quantum']) if s['r_max'] is not None else None
        limited = cap is not None and rate >= cap
        s['tester_limited'] = limited
        self._stop(reason + (' (tester-limited at r_max: R >= %d)' % cap if limited else ''), rate)

    def summary(self):
        s = self.s
        valid = [t for t in s['trials'] if t['outcome'] != 'invalid']
        return {'algorithm': s['algorithm'], 'result_sps': s['result'], 'tester_limited': s['tester_limited'],
                'stop_reason': s['stop_reason'], 'done': s['done'], 'trials': len(s['trials']), 'valid_trials': len(valid),
                'passing_trials': sum(t['outcome'] == 'pass' for t in valid),
                'highest_passing_sps': max((t['rate'] for t in valid if t['outcome'] == 'pass'), default=None),
                'lowest_failing_sps': min((t['rate'] for t in valid if t['outcome'] == 'fail'), default=None),
                'parameters': {'N': s['n'], 'r0': s['trials'][0]['rate'] if s['trials'] else s['r'], 'w': s['w0'], 'd': s['d0'],
                               'confirmations': s['confirmations'], 'quantum': s['quantum'], 'r_max': s['r_max']}}


def tester_cap(baseline_sps, factor=TESTER_CAP_FACTOR, quantum=QUANTUM, cap=None):
    """r_max = factor x the §6.1 testbed baseline of the same row, floored to the quantum (and an optional hard cap)."""
    if baseline_sps is None:
        return None
    r_max = int(float(factor) * float(baseline_sps)) // quantum * quantum
    if cap is not None:
        r_max = min(r_max, int(cap) // quantum * quantum)
    return r_max


# ---- trial plans

def trial_seconds(rate, n, minimum=0):
    """Seconds needed to attempt n sessions at `rate` (RFC: until n are
    attempted), at least `minimum`, rounded up to whole seconds."""
    return max(int(math.ceil(n / float(rate))), int(minimum))


def host_shares(cps, seconds, hosts):
    """Each caller host's share (cps, seconds, calls) of one stage: a whole cps per host, the first hosts taking
    the remainder (6440 cps over 3 hosts is 2147 + 2147 + 2146), so the stage total is exact and every host's
    cps x seconds is an exact whole number in floating point too. rvoip-loadgen refuses a stage whose
    cps x seconds has a fractional part, and an even fractional share does not survive that: 6440 / 3 cps x
    30 s is 64399.99999999999 (baseline-0.5 R1 at 12880 sps: two void trials). ValueError unless `cps` is a
    whole number of at least `hosts`."""
    whole, seconds = int(cps), int(seconds)
    if whole != cps or whole < hosts or hosts < 1 or seconds < 1:
        raise ValueError('stage %s cps x %s s does not split into %d whole-cps host shares' % (cps, seconds, hosts))
    base, extra = divmod(whole, hosts)
    shares = [base + (1 if i < extra else 0) for i in range(hosts)]
    return [{'cps': share, 'seconds': seconds, 'calls': share * seconds} for share in shares]


def validate_shares(profile, hosts):
    """Every stage of `profile` splits into `hosts` whole-cps shares (raises ValueError otherwise)."""
    for stage in profile['stages']:
        host_shares(stage['cps'], stage['seconds'], hosts)
    return True


def _whole_seconds(cps, seconds, hosts):
    """The smallest seconds >= `seconds` with cps x seconds divisible by hosts."""
    while (int(cps) * int(seconds)) % hosts:
        seconds += 1
    return seconds


def trial_profile(rate, n=50000, warmup_seconds=WARMUP_SECONDS, minimum_seconds=MINIMUM_TRIAL_SECONDS,
                  max_stage_seconds=MAX_STAGE_SECONDS, warmup_ramp=False, hosts=1):
    """Load profile for one trial: a warm-up (not scored), then the scored
    trial stage(s) at `rate`. Stages longer than the caller's 600 s bound are
    split into consecutive `trial-<k>` stages.

    The default warm-up is `warmup_seconds` at half the rate. `warmup_ramp`
    (admission-on rows) ramps to the trial rate in WARMUP_RAMP_STEPS equal
    steps (25/50/75/100 % of `rate`, stages warmup-1..warmup-4, all transient)
    so a freshly started DUT's admission has learned the call cost before the
    scored stage; each step is a multiple of RAMP_CPS_QUANTUM.

    `hosts` caller hosts split every stage evenly: every stage's cps x seconds
    is made divisible by `hosts` (seconds are extended when a quantum-4 rate
    alone cannot guarantee it, e.g. 3 hosts)."""
    rate = int(rate)
    if rate < 1 or hosts < 1:
        raise ValueError('rate and hosts must be positive')
    total = trial_seconds(rate, n, minimum_seconds)
    if total < SHORT_TRIAL_SECONDS:
        # Below 10 s, +4 % so the scored window still holds >= n attempts at high rates
        # (short stages lose a few percent of their attempts to the 1 s bin edges).
        total = trial_seconds(rate, int(math.ceil(n * 1.04)), minimum_seconds)
    stages = []
    if warmup_seconds and warmup_ramp:
        steps = WARMUP_RAMP_STEPS
        if int(warmup_seconds) % steps or int(warmup_seconds) // steps < 1:
            raise ValueError('ramped warm-up: seconds divisible by %d' % steps)
        for k in range(1, steps + 1):
            cps = int(math.ceil(rate * k / float(steps) / RAMP_CPS_QUANTUM)) * RAMP_CPS_QUANTUM
            cps = max(RAMP_CPS_QUANTUM, min(cps, rate))
            stages.append({'name': 'warmup-%d' % k, 'cps': cps,
                           'seconds': _whole_seconds(cps, int(warmup_seconds) // steps, hosts)})
    elif warmup_seconds:
        cps = max(1, rate // 2)
        stages.append({'name': 'warmup', 'cps': cps, 'seconds': _whole_seconds(cps, int(warmup_seconds), hosts)})
    warmups = [stage['name'] for stage in stages]
    left, index = total, 1
    while left > 0:
        chunk = min(left, max_stage_seconds)
        left -= chunk
        if left <= 0:
            chunk = _whole_seconds(rate, chunk, hosts)
        stages.append({'name': 'trial-%d' % index, 'cps': rate, 'seconds': chunk})
        index += 1
    profile = {'name': 'rfc7502', 'stages': stages, 'transient_stages': warmups, 'hosts': hosts,
               'basis': 'RFC 7502 §4.10 trial at %d sps, N >= %d, scored stage >= %d s' % (rate, n, minimum_seconds) +
                        (' (warm-up ramped to the trial rate over %d s)' % int(warmup_seconds) if warmup_seconds and warmup_ramp
                         else ' (warm-up %d s at r/2)' % int(warmup_seconds) if warmup_seconds else '')}
    validate_shares(profile, hosts)
    return profile


def profile_for_spec(search_spec, rate, hosts=None):
    """trial_profile() with a search spec's options (see new_search)."""
    ramp = bool(search_spec.get('warmup_ramp'))
    warmup = search_spec.get('warmup_seconds', RAMPED_WARMUP_SECONDS if ramp else WARMUP_SECONDS)
    return trial_profile(rate, search_spec.get('n', RFC_DEFAULTS['n']), warmup,
                         search_spec.get('minimum_trial_seconds', MINIMUM_TRIAL_SECONDS),
                         search_spec.get('max_stage_seconds', MAX_STAGE_SECONDS), warmup_ramp=ramp,
                         hosts=int(hosts or search_spec.get('hosts', 1)))


def new_search(search_spec):
    """A Search from a spec dict: r0, n, w, d, confirmations, algorithm, max_trials, r_max | baseline_sps
    (r_max = 0.67 x baseline, floored to the quantum; optional r_max_cap), r_min, quantum (default 4),
    resolution. r0 is kept at or below r_max."""
    spec = dict(search_spec)
    quantum = int(spec.get('quantum', QUANTUM))
    r_max = spec.get('r_max')
    if r_max is None and spec.get('baseline_sps') is not None:
        r_max = tester_cap(spec['baseline_sps'], spec.get('tester_cap_factor', TESTER_CAP_FACTOR), quantum, spec.get('r_max_cap'))
    r0 = spec.get('r0', r_max)
    if r0 is None:
        raise ValueError('search spec needs r0 (or r_max / baseline_sps)')
    return Search(r0, n=spec.get('n', RFC_DEFAULTS['n']), w=spec.get('w', RFC_DEFAULTS['w']),
                  d=spec.get('d'), confirmations=spec.get('confirmations', RFC_DEFAULTS['confirmations']),
                  algorithm=spec.get('algorithm', 'rfc7502'), max_trials=spec.get('max_trials', 60),
                  r_max=r_max, r_min=spec.get('r_min', 10), quantum=quantum,
                  resolution=spec.get('resolution', 0.02))


# ---- trial evidence

def trial_windows(summary):
    """[start, end) seconds of each trial-* stage from the run start (stage_summaries in run order)."""
    windows, at = [], 0
    for row in summary.get('stage_summaries') or []:
        seconds = row.get('seconds') or 0
        if str(row.get('name', '')).startswith('trial-'):
            windows.append((at, at + seconds))
        at += seconds
    return windows


def _real_misses(summary, misses, windows):
    """Dispatch misses of one caller host that are tester evidence (see caller_invalid_in_trial)."""
    misses = sorted(misses, key=lambda row: row['t'])
    by_t = {row['t']: row for row in misses}
    real = []
    short = [(a, b) for a, b in windows if b - a < SHORT_TRIAL_SECONDS]
    for a, b in short:
        # A trial stage under 10 s is judged on its total, not per bin: the stage switch shifts the schedulers by a
        # fraction of a second, so one edge bin is a large share of a short stage. Bins outside the miss list were
        # within 1% of their request and count as requested.
        inside = [row for row in misses if a <= row['t'] < b]
        if not inside:
            continue
        # Every bin of a stage requests the same share of this caller host (the stage rate for one host).
        requested = inside[0]['requested'] * (b - a)
        deviation = sum(row['dispatched'] - row['requested'] for row in inside)
        if requested and abs(deviation) > 0.01 * requested:
            real.extend(inside)
    for row in misses:
        if any(a <= row['t'] < b for a, b in short):
            continue   # judged on the stage total above
        if windows and not any(a <= row['t'] < b for a, b in windows):
            continue   # warm-up (not scored)
        if windows and row['dispatched'] < row['requested'] and any(row['t'] == b - 1 for a, b in windows) \
                and not by_t.get(row['t'] + 1):
            continue   # the trial's last, partial second (its tail lands in the next bin, outside the trial)
        if windows and row['dispatched'] > row['requested'] and any(row['t'] == a for a, b in windows):
            continue   # the stage switch: the schedulers' catch-up burst lands in the trial's first second
        pair = by_t.get(row['t'] + 1) or by_t.get(row['t'] - 1)
        if pair and abs((row['dispatched'] + pair['dispatched']) - (row['requested'] + pair['requested'])) \
                <= 0.01 * (row['requested'] + pair['requested']):
            continue   # bin-edge: the neighbouring bin made it up
        real.append(row)
    return real


def caller_invalid_in_trial(summary):
    """Caller-validity reasons that concern the scored trial stage: every failed validity check except the
    per-bin dispatch check, plus dispatch misses inside a trial window that the neighbouring bin does not make
    up (sum of the two bins within 1% of the sum requested). Misses are judged per caller host (`caller`)."""
    validity = summary.get('caller_validity') or {}
    if not validity or validity.get('caller_valid_before_backpressure') is not False:
        return []
    checks = validity.get('checks') or {}
    vacuous = set()
    lateness = validity.get('scheduler_lateness_p99_ms')
    if validity.get('backpressure_second') is not None and lateness is not None and lateness <= 5.0:
        # A 503 in the warm-up's first second makes the "before backpressure" window empty, so the
        # schedule-to-write check has no calls to judge; the run's own schedule lateness p99 decides.
        vacuous.add('schedule_to_write_p99_at_most_5ms')
    reasons = ['caller check %s failed' % name for name, ok in sorted(checks.items())
               if ok is False and name != 'dispatch_within_1pct_per_bin_before_backpressure' and name not in vacuous]
    softirq_check = 'no_irq_cpu_softirq_saturated'
    if checks.get(softirq_check) is False:
        saturated = host_saturation(summary)
        named = [text for role, text in saturated['parts'] if role != 'dut']
        if named:
            reasons = [reason + ' (%s)' % '; '.join(named) if reason == 'caller check %s failed' % softirq_check else reason
                       for reason in reasons]
    if checks.get('dispatch_within_1pct_per_bin_before_backpressure') is False:
        windows = trial_windows(summary)
        misses = validity.get('dispatch_misses') or []
        groups = {}
        for row in misses:
            groups.setdefault(row.get('caller'), []).append(row)
        real = []
        for rows in groups.values():
            real.extend(_real_misses(summary, rows, windows))
        if real or not windows and not misses:
            reasons.append('caller dispatch outside 1%% in %d trial bins' % len(real) if real else
                           'caller dispatch check failed without per-bin detail')
    return reasons


def _pct_text(share):
    return '%.0f %%' % (100.0 * share) if share is not None else 'n/a'


def _cpu_ranges(cpus):
    cpus = sorted(set(cpus or ()))
    parts, start = [], None
    for i, cpu in enumerate(cpus):
        start = cpu if start is None else start
        if i + 1 == len(cpus) or cpus[i + 1] != cpu + 1:
            parts.append(str(start) if start == cpu else '%d-%d' % (start, cpu))
            start = None
    return ','.join(parts)


def host_saturation(summary):
    """Which host's kernel receive path saturated in one trial (summarize_trial resource.<role>.softirq).

    {'side': 'caller' | 'dut' | 'both' | None (none saturated) | 'unknown' (no softirq record),
     'parts': [(role, text)], 'text': one line for a report, 'net_flags': ['caller-1 pps_allowance_exceeded +12']}.
    A caller host counts when its check failed (saturation that began before the DUT's first backpressure) or
    it saturated at any point of the scored stage; the DUT side is recorded only."""
    summary = summary or {}
    resource = summary.get('resource') or {}
    by_caller = summary.get('caller_validity_by_caller') or {}
    far_end = 'ea-uas host' if ((summary.get('plan') or {}).get('dut_kind') == 'ea-uas') else 'DUT host'
    parts, peaks, sides, known = [], [], set(), False
    roles = sorted(role for role in set(resource) | set(by_caller) if str(role).startswith('caller')) + ['dut']
    for role in roles:
        softirq = (resource.get(role) or {}).get('softirq') or (by_caller.get(role) or {}).get('softirq')
        if not softirq or softirq.get('peak_share') is None:
            continue
        known = True
        label = far_end if role == 'dut' else role
        peaks.append('%s %s' % (label, _pct_text(softirq['peak_share'])))
        cpus = softirq.get('saturated_cpus') or []
        if cpus:
            sides.add('dut' if role == 'dut' else 'caller')
            when = softirq.get('first_saturated_second')
            parts.append((role, '%s receive path (softirq %s on CPUs %s%s%s)' % (
                label, _pct_text(softirq['peak_share']), _cpu_ranges(cpus),
                ', from second %d' % when if when is not None else '',
                ', after the DUT backpressure marker' if role != 'dut' and softirq.get('saturated_before_cutoff') is False
                and softirq.get('cutoff_epoch') is not None else '')))
    flags = []
    for role in sorted(resource):
        net = (resource.get(role) or {}).get('net') or {}
        for key, value in sorted((net.get('flagged') or {}).items()):
            flags.append('%s %s +%d' % (far_end if role == 'dut' else role, key, value))
    if not known:
        side, text = 'unknown', 'softirq not recorded'
    elif not sides:
        side, text = None, 'no host receive path saturated (peak softirq %s)' % ', '.join(peaks)
    else:
        side = 'both' if len(sides) == 2 else sides.pop()
        text = '; '.join(t for _, t in parts)
    return {'side': side, 'parts': parts, 'text': text, 'net_flags': flags}


def caller_load(summary):
    """Generator load of one trial: sources and flows, the caller processes' peak cores and hottest thread,
    and the caller hosts' peak CPU."""
    validity = summary.get('caller_validity') or {}
    pools = validity.get('source_pools') or {}
    resource = summary.get('resource') or {}
    hosts = {role: (row or {}).get('host_peak_cpu_pct') for role, row in resource.items() if str(role).startswith('caller')}
    sources = pools.get('sources')
    per_source = (summary.get('generator') or {}).get('connections_per_source') or 1
    flows = sources * per_source if sources else sources
    transport = str((summary.get('rfc6076') or {}).get('transport') or '').replace('_', '-')
    return {'sources': sources, 'connections': flows if transport != 'udp-rtp' else 0, 'flows': flows,
            'process_peak_cores': validity.get('process_peak_cores'), 'process_core_limit': validity.get('process_core_limit'),
            'hottest_thread_cores': validity.get('thread_peak_cores'),
            'hottest_thread': (validity.get('hottest_thread') or {}).get('name'),
            'host_peak_cpu_pct': hosts}


def trial_stage_rows(summary):
    return [row for row in summary.get('stage_summaries') or [] if str(row.get('name', '')).startswith('trial-')]


def evaluate_trial(summary, criterion='rfc'):
    """(attempted, succeeded, valid, detail) for the trial-* stages of one trial's merged summary.json.
    Attempted = every call scheduled in the trial stages (RFC 7501 §3.1.6: the EA counts the attempts it
    actually sends)."""
    if criterion not in ('rfc', 'full'):
        raise ValueError('criterion must be rfc or full')
    rows = trial_stage_rows(summary)
    if not rows:
        return 0, 0, False, {'reason': 'no trial-* stage in summary.json', 'invalid_reasons': ['no trial-* stage']}
    whole = [row['whole_stage'] for row in rows]
    offered = sum(w['offered'] for w in whole)
    dispatched = sum(w['dispatched'] for w in whole)
    missing = sum(w['missing_terminal_observations'] for w in whole)
    key = ('rfc_attempt_ok' if all('rfc_attempt_ok' in w for w in whole) else 'rfc_session_ok') if criterion == 'rfc' else 'full'
    succeeded = sum(w.get(key) or 0 for w in whole)
    validity = summary.get('caller_validity') or {}
    checks = summary.get('checks') or {}
    reasons = []
    if checks.get('caller_evidence_complete') is False:
        reasons.append('caller evidence missing: %s' % ', '.join((summary.get('evidence') or {}).get('callers_missing') or []))
    if missing:
        reasons.append('%d calls without terminal evidence' % missing)
    if dispatched < offered:
        reasons.append('%d scheduled calls not dispatched' % (offered - dispatched))
    reasons += caller_invalid_in_trial(summary)
    # A DUT that stops answering makes the caller's calls wait until their setup deadline, so the caller's
    # concurrency bound fills and it skips scheduled calls. When the sent attempts include Session Attempt
    # Failures, the caller's own CPU checks are clean and every other reason is about dispatch/shutdown/pool
    # balance, the shortfall is the DUT's (RFC 7501 §3.1.8 failures among the attempts sent): fail, not invalid.
    # A saturated caller receive path (no_irq_cpu_softirq_saturated) is never in this list: the tester's own
    # kernel dropped the responses, so those failures are tester evidence and the trial stays invalid.
    dispatch_kinds = ('scheduled calls not dispatched', 'caller dispatch outside', 'caller dispatch check failed',
                      'caller check clean_shutdown failed', 'caller check source_pools_balanced failed',
                      'caller check zero_guard_skips failed')
    sent_failures = dispatched - succeeded
    rejected_503 = sum(w.get('rejected_503') or 0 for w in whole)
    if rejected_503 > 0 and dispatched >= 0.99 * offered:
        # A DUT answering many INVITEs with 503 slows the caller's write path (schedule-to-write p99 > 5 ms)
        # while it still sent every scheduled attempt: those 503s are Session Attempt Failures, not tester evidence.
        dispatch_kinds += ('caller check schedule_to_write_p99_at_most_5ms failed',)
    if reasons and sent_failures > 0 and all(any(kind in reason for kind in dispatch_kinds) for reason in reasons):
        dut_shortfall, reasons = reasons, []
    else:
        dut_shortfall = None
    # Incomplete wire records (a dropped trace event) are reported, never a reason to repeat a trial:
    # completion then falls back to the caller's outcome.
    incomplete = sum((w.get('rfc6076') or {}).get('trace', {}).get('incomplete_records') or 0 for w in whole)
    without = sum((w.get('rfc6076') or {}).get('trace', {}).get('calls_without_record') or 0 for w in whole)
    detail = {'offered': offered, 'dispatched': dispatched, 'succeeded': succeeded, 'criterion': criterion,
              'rejected_503': rejected_503,
              'full': sum(w.get('full') or 0 for w in whole),
              'rfc_session_ok': sum(w.get('rfc_session_ok') or 0 for w in whole),
              'scored_by': key,
              'disconnect_failures': sum(w.get('rfc_disconnect_failures') or 0 for w in whole),
              'final_status_counts': _sum_counts((w.get('rfc6076') or {}).get('final_status_counts') for w in whole),
              'rfc6076_incomplete_records': incomplete, 'rfc6076_calls_without_record': without,
              'rfc6076_trace_coverage_low': bool(offered) and (incomplete + without) > 0.01 * offered,
              'invalid_reasons': reasons, 'stages': [row['name'] for row in rows],
              'dut_shortfall_reasons': dut_shortfall,
              # recorded as data; under criterion rfc they never decide the trial
              'run_verdict': summary.get('verdict'),
              'run_failed_checks': sorted(k for k, v in checks.items() if v is False),
              'run_missing_checks': sorted(k for k, v in checks.items() if v is None),
              'caller_validity_checks_failed': sorted(k for k, v in ((validity.get('checks') or {}).items()) if v is False),
              'dut': {key: (summary.get('dut') or {}).get(key) for key in
                      ('true_zero', 'clean_shutdown', 'peak_cpu_cores', 'peak_rss_bytes', 'responses_503')},
              # the tester's own load, so a result can show the generator was not the limit
              'caller': caller_load(summary),
              # which host's kernel receive path saturated (tester evidence on a caller, recorded on the DUT)
              'host_saturation': host_saturation(summary)}
    return offered, succeeded, not reasons, detail


def _sum_counts(blocks):
    out = {}
    for block in blocks:
        for key, value in (block or {}).items():
            out[key] = out.get(key, 0) + value
    return dict(sorted(out.items()))


def trial_verdict(summary, criterion='rfc'):
    """verdict.json content for one trial: outcome pass | fail | invalid with its reasons (the search records
    the same outcome; a search only differs when it has already ended)."""
    if summary is None:
        return {'outcome': 'invalid', 'criterion': criterion, 'attempted': 0, 'succeeded': 0,
                'reasons': ['no merged summary.json']}
    attempted, succeeded, valid, detail = evaluate_trial(summary, criterion)
    if not valid or not attempted:
        outcome, reasons = 'invalid', detail.get('invalid_reasons') or ['no session attempted']
    elif succeeded == attempted:
        outcome, reasons = 'pass', []
    else:
        outcome = 'fail'
        reasons = ['%d of %d attempts were Session Attempt Failures (RFC 7501 §3.1.8): %s' % (
            attempted - succeeded, attempted, json.dumps(detail.get('final_status_counts') or {}, sort_keys=True))]
        reasons += detail.get('dut_shortfall_reasons') or []
    return {'outcome': outcome, 'criterion': criterion, 'attempted': attempted, 'succeeded': succeeded,
            'failed': attempted - succeeded if outcome == 'fail' else 0, 'reasons': reasons, 'detail': detail}
