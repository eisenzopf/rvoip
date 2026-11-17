"""Caller (rvoip-loadgen) thread policy and the caller-side validity predicate.

Ported from the E74 harness (caller_policy.py). The caller is judged on
caller-side criteria only, on the scored stage and before the DUT's first
backpressure (first 503 or first setup timeout): a dispatch shortfall that
starts after that is a DUT result, not an invalid caller.

Thresholds (unchanged from E74, RFC7502_BENCHMARK.md §2):
* dispatch within max(1, 1 %) of the requested calls in every 1 s bin
  (rfc7502.caller_invalid_in_trial lets the neighbouring bin make up a
  bin-edge shortfall and ignores the unscored warm-up);
* schedule-to-write p99 <= 5 ms (`scheduler_lateness_ms`: timer due to the
  call being scheduled);
* caller process CPU <= 0.75 x the host's physical cores, during generation
  only (the scheduled seconds plus the hold; the caller's end-of-run report
  building is not load generation);
* no caller thread above 0.8 core over any sampler interval.

Thread policy: `RVOIP_LOADGEN_SIGNALING_THREADS` + `RVOIP_LOADGEN_LANES` x
`RVOIP_LOADGEN_WORKERS_PER_LANE` + `RVOIP_MEDIA_WORKER_THREADS` <= logical
CPUs, with `RVOIP_MEDIA_RUNTIME_LANES` media runtime lanes. Thread names:
`loadgen-signaling`, `loadgen-call-<n>`, `loadgen-validation`, `rvoip-media`.

Resource samples are host_sampler.py rows: `at` (epoch s), `cpu_cores_pct`
(process CPU over the interval, 100 = one core), `thread_cpu_times`
([{id, user_time, system_time}] seconds) and `thread_names` ({tid: name}).
Standard library only.
"""
import math

POLICY_KEYS = ('RVOIP_LOADGEN_SIGNALING_THREADS', 'RVOIP_LOADGEN_LANES',
               'RVOIP_LOADGEN_WORKERS_PER_LANE', 'RVOIP_MEDIA_WORKER_THREADS')
MEDIA_LANES_KEY = 'RVOIP_MEDIA_RUNTIME_LANES'
# 8 signaling + 4 lanes x 6 + 16 media = 48 logical CPUs (c8in.12xlarge).
POLICY_48 = {'RVOIP_LOADGEN_SIGNALING_THREADS': '8', 'RVOIP_LOADGEN_LANES': '4',
             'RVOIP_LOADGEN_WORKERS_PER_LANE': '6', 'RVOIP_MEDIA_WORKER_THREADS': '16',
             'RVOIP_MEDIA_RUNTIME_LANES': '4'}
# c8in.24xlarge (96 vCPU, 48 physical cores), the default caller host. Measured on the 48-vCPU
# caller at ~13k concurrent calls: signaling 6% busy, call workers 63%, media 70%; so the 96-vCPU
# budget keeps 8 signaling threads, doubles call workers (8 lanes x 6) and grows media to 40
# threads in 8 media runtime lanes: 8 + 48 + 40 = 96.
POLICY_96 = {'RVOIP_LOADGEN_SIGNALING_THREADS': '8', 'RVOIP_LOADGEN_LANES': '8',
             'RVOIP_LOADGEN_WORKERS_PER_LANE': '6', 'RVOIP_MEDIA_WORKER_THREADS': '40',
             'RVOIP_MEDIA_RUNTIME_LANES': '8'}
# c8in.48xlarge (192 vCPU). The caller bounds signaling <= 16, lanes <= 8, workers per lane <= 16,
# media <= 64 (media runtime lanes <= 8): 16 signaling + 8 lanes x 14 + 64 media = 192.
POLICY_192 = {'RVOIP_LOADGEN_SIGNALING_THREADS': '16', 'RVOIP_LOADGEN_LANES': '8',
              'RVOIP_LOADGEN_WORKERS_PER_LANE': '14', 'RVOIP_MEDIA_WORKER_THREADS': '64',
              'RVOIP_MEDIA_RUNTIME_LANES': '8'}
POLICY_BY_CPUS = {48: POLICY_48, 96: POLICY_96, 192: POLICY_192}
LINUX_COMM_LEN = 15   # TASK_COMM_LEN - 1: the longest thread name /proc shows
THREAD_GROUPS = ('loadgen-signaling', 'loadgen-call', 'loadgen-validation', 'rvoip-media')
DISPATCH_TOLERANCE = 0.01
SCHEDULE_P99_LIMIT_MS = 5.0
PROCESS_CORE_FRACTION = 0.75
THREAD_CORE_LIMIT = 0.8
VERSION = 'caller-validity-v1'


def policy_for(logical_cpus):
    """The caller environment for a host with `logical_cpus` (48, 96 or 192)."""
    if logical_cpus not in POLICY_BY_CPUS:
        raise ValueError('no caller thread policy for %s logical CPUs (have %s)' % (logical_cpus, sorted(POLICY_BY_CPUS)))
    return dict(POLICY_BY_CPUS[logical_cpus])


def budget(env):
    signaling, lanes, workers, media = (int(env[key]) for key in POLICY_KEYS)
    return signaling + lanes * workers + media


def validate(env, logical_cpus):
    """The caller's own rule: signaling + lanes*per_lane + media <= CPUs. Raises ValueError."""
    missing = [key for key in POLICY_KEYS if key not in env]
    if missing:
        raise ValueError('explicit complete caller policy required (missing %s)' % ', '.join(missing))
    signaling, lanes, workers, media = (int(env[key]) for key in POLICY_KEYS)
    if not (1 <= signaling <= 16 and 1 <= lanes <= 8 and 1 <= workers <= 16 and 1 <= media <= 64):
        raise ValueError('caller policy out of bounds')
    if budget(env) > logical_cpus:
        raise ValueError('caller executor budget %d exceeds %d host CPUs' % (budget(env), logical_cpus))
    lanes_media = int(env.get(MEDIA_LANES_KEY, '2'))
    if not 1 <= lanes_media <= min(8, media):
        raise ValueError('%s out of bounds' % MEDIA_LANES_KEY)
    return budget(env)


def expected_thread_groups(env):
    """Thread-name group -> expected thread count under `env` (validation threads are not bounded here)."""
    return {'loadgen-signaling': int(env['RVOIP_LOADGEN_SIGNALING_THREADS']),
            'loadgen-call': int(env['RVOIP_LOADGEN_LANES']) * int(env['RVOIP_LOADGEN_WORKERS_PER_LANE']),
            'rvoip-media': int(env['RVOIP_MEDIA_WORKER_THREADS'])}


def thread_group(name):
    """The policy group of a thread name (loadgen-call-3 -> loadgen-call), else the name itself.

    Linux keeps the first 15 bytes of a thread name (/proc comm), so a sampled
    `loadgen-signali` is `loadgen-signaling` and `loadgen-validat` is
    `loadgen-validation`."""
    name = name or 'unknown'
    for group in THREAD_GROUPS:
        if name == group or name.startswith(group + '-'):
            return group
        if len(name) == LINUX_COMM_LEN and (group + '-').startswith(name):
            return group
    return name


def observed_thread_groups(samples):
    """Group -> the most threads of that group seen in one sample."""
    out = {}
    for row in samples:
        counts = {}
        for name in (row.get('thread_names') or {}).values():
            group = thread_group(name)
            counts[group] = counts.get(group, 0) + 1
        for group, count in counts.items():
            out[group] = max(out.get(group, 0), count)
    return out


def _quantile(values, q):
    ordered = sorted(values)
    if not ordered:
        return None
    return ordered[max(0, min(len(ordered) - 1, math.ceil(q * len(ordered)) - 1))]


def thread_peaks(samples):
    """Max per-thread CPU (cores) over consecutive sampler intervals, and the hottest thread."""
    peak = 0.0
    hottest = None
    for first, last in zip(samples, samples[1:]):
        elapsed = last['at'] - first['at']
        if elapsed <= 0:
            continue
        before = {str(t['id']): t['user_time'] + t['system_time'] for t in first.get('thread_cpu_times') or []}
        for thread in last.get('thread_cpu_times') or []:
            tid = str(thread['id'])
            if tid not in before:
                continue
            cores = (thread['user_time'] + thread['system_time'] - before[tid]) / elapsed
            if cores > peak:
                peak = cores
                hottest = {'tid': tid, 'name': (last.get('thread_names') or {}).get(tid), 'at': last['at'], 'cores': cores}
    return peak, hottest


def generation_samples(samples, run_start_epoch, bins, hold_ms, backpressure_second=None):
    """Samples taken while the generator loads the DUT: from the run start to the end of the scheduled seconds
    plus the hold (or to the first backpressure). Without a run start every sample is kept."""
    if run_start_epoch is None:
        return list(samples)
    if backpressure_second is not None:
        end = run_start_epoch + backpressure_second
    elif bins:
        end = run_start_epoch + max(row['t'] for row in bins) + 1 + (hold_ms or 0) / 1000.0
    else:
        end = float('inf')
    return [row for row in samples if run_start_epoch <= row['at'] <= end]


def predicate(bins, scheduler_lateness_ms, samples, report, backpressure_second=None,
              physical_cores=None, process_core_limit=None, run_start_epoch=None, judged_from_second=0):
    """Caller validity: dispatch, schedule lag, CPU, per-thread CPU, skips, shutdown.

    `bins` are this caller's 1 s bins ({t, requested_cps, dispatched, guard_skips}); time-bounded checks are
    judged from `judged_from_second` and before the DUT's first backpressure second. `scheduler_lateness_ms`
    must already be restricted to calls scheduled before that marker. A check whose input is missing is
    False (and named under `missing_inputs`), never assumed.
    """
    missing = []
    if process_core_limit is None:
        if physical_cores is None:
            missing.append('physical_cores')
        else:
            process_core_limit = PROCESS_CORE_FRACTION * physical_cores

    def in_window(second):
        return second >= judged_from_second and (backpressure_second is None or second < backpressure_second)

    judged = [row for row in bins if in_window(row['t'])]
    misses = [row for row in judged
              if abs(row['dispatched'] - row['requested_cps']) > max(1, DISPATCH_TOLERANCE * row['requested_cps'])]
    lag_p99 = _quantile([value for value in scheduler_lateness_ms if value is not None], .99)
    window = generation_samples(samples, run_start_epoch, bins, report.get('hold_ms'), backpressure_second)
    cores = [row['cpu_cores_pct'] for row in window if row.get('cpu_cores_pct') is not None]
    process_peak = max(cores) / 100 if cores else None
    thread_peak, hottest = thread_peaks(window)
    if not samples:
        missing.append('resources.jsonl')
    if all('guard_skips' in row for row in bins) and bins:
        guard_skips = sum(row['guard_skips'] for row in judged)
    else:
        guard_skips = report.get('generator_skipped')
    checks = {
        'dispatch_within_1pct_per_bin_before_backpressure': bool(judged) and not misses,
        'schedule_to_write_p99_at_most_5ms': lag_p99 is not None and lag_p99 <= SCHEDULE_P99_LIMIT_MS,
        'process_cpu_at_most_limit': process_peak is not None and process_core_limit is not None
                                     and process_peak <= process_core_limit,
        'no_thread_above_0_8_core': len(window) >= 2 and thread_peak <= THREAD_CORE_LIMIT,
        'zero_guard_skips': guard_skips == 0,
        'clean_shutdown': report.get('shutdown_outcome') == 'ok' and not report.get('interrupted')
                          and not report.get('caller_watchdogs'),
    }
    sources = source_pools(report)
    if sources is not None:
        # Multi-source caller only: no source pool ran out and calls were spread round robin.
        checks['source_pools_balanced'] = sources['balanced']
    before_backpressure = all(value for key, value in checks.items() if key not in ('clean_shutdown', 'source_pools_balanced'))
    result = {'version': VERSION, 'caller_valid': all(checks.values()),
              'caller_valid_before_backpressure': before_backpressure and (checks['clean_shutdown'] or backpressure_second is not None),
              'checks': checks, 'missing_inputs': missing, 'backpressure_second': backpressure_second,
              'judged_from_second': judged_from_second, 'judged_bins': len(judged),
              'dispatch_misses': [{'t': row['t'], 'requested': row['requested_cps'], 'dispatched': row['dispatched']}
                                  for row in misses[:50]],
              'dispatch_miss_count': len(misses), 'guard_skips_before_backpressure': guard_skips,
              'scheduler_lateness_p99_ms': lag_p99,
              'process_peak_cores': process_peak, 'process_core_limit': process_core_limit,
              'physical_cores': physical_cores, 'generation_samples': len(window),
              'thread_peak_cores': thread_peak if len(window) >= 2 else None, 'hottest_thread': hottest,
              'scope': 'Schedule-to-write uses scheduler_lateness_ms (timer due to scheduling), which excludes '
                       'peer-blocked TLS writes. CPU is judged on samples taken during generation (run start to the '
                       'last scheduled second plus the hold). A shortfall or unclean shutdown after the DUT '
                       'backpressure marker is a DUT result.'}
    if sources is not None:
        result['source_pools'] = sources
    return result


def source_pools(report):
    """Per-source pool readout of a multi-source caller report, else None.

    Balanced: every source took calls, none was preferred while full (no pool
    exhaustion), assignments differ by at most one beyond spills and calls the
    global max_concurrent bound skipped, and every source endpoint shut down
    cleanly.
    """
    rows = report.get('sources')
    if not rows:
        return None
    assigned = [row.get('assigned', 0) for row in rows]
    spilled = sum(row.get('spilled_in', 0) for row in rows)
    full = sum(row.get('preferred_full', 0) for row in rows)
    shutdown_ok = all((row.get('shutdown') or {}).get('outcome') == 'ok' for row in rows)
    skipped = report.get('generator_skipped') or 0
    balanced = (min(assigned) > 0 and full == 0 and max(assigned) - min(assigned) <= 1 + spilled + skipped and shutdown_ok)
    return {'balanced': balanced, 'sources': len(rows), 'assigned': assigned, 'spilled': spilled,
            'preferred_full': full, 'generator_skipped': skipped, 'peak_active': [row.get('peak_active') for row in rows],
            'failed': [row.get('failed') for row in rows], 'shutdown_ok': shutdown_ok}


def combine(by_caller, backpressure_second=None):
    """One trial-wide validity from every caller host's predicate (each host valid on its own).

    Checks are AND-ed; dispatch misses keep their host (`caller`) so rfc7502 judges neighbour bins per host;
    lateness, process cores and hottest thread are the worst host's."""
    rows = list(by_caller.items())
    if not rows:
        return {'version': VERSION, 'caller_valid': False, 'caller_valid_before_backpressure': False, 'checks': {},
                'missing_inputs': ['caller evidence'], 'dispatch_misses': [], 'backpressure_second': backpressure_second}
    names = sorted(set().union(*[(v.get('checks') or {}).keys() for _, v in rows]))
    checks = {name: all((v.get('checks') or {}).get(name) for _, v in rows) for name in names}
    misses = [dict(miss, caller=role) for role, v in rows for miss in v.get('dispatch_misses') or []]

    def worst(key):
        values = [v.get(key) for _, v in rows if v.get(key) is not None]
        return max(values) if values else None
    hottest = max((v.get('hottest_thread') for _, v in rows if v.get('hottest_thread')), key=lambda h: h['cores'], default=None)
    hottest_role = next((role for role, v in rows if v.get('hottest_thread') is hottest), None) if hottest else None
    pools = [v.get('source_pools') for _, v in rows if v.get('source_pools')]
    out = {'version': VERSION, 'caller_valid': all(v.get('caller_valid') for _, v in rows),
           'caller_valid_before_backpressure': all(v.get('caller_valid_before_backpressure') for _, v in rows),
           'checks': checks, 'missing_inputs': sorted({m for _, v in rows for m in v.get('missing_inputs') or []}),
           'backpressure_second': backpressure_second,
           'dispatch_misses': misses, 'dispatch_miss_count': sum(v.get('dispatch_miss_count') or 0 for _, v in rows),
           'scheduler_lateness_p99_ms': worst('scheduler_lateness_p99_ms'),
           'process_peak_cores': worst('process_peak_cores'),
           'process_core_limit': min((v['process_core_limit'] for _, v in rows if v.get('process_core_limit') is not None), default=None),
           'thread_peak_cores': worst('thread_peak_cores'),
           'hottest_thread': dict(hottest, caller=hottest_role) if hottest else None,
           'scope': 'Every caller host valid on its own predicate (caller_validity_by_caller); dispatch misses '
                    'are tagged with their host.'}
    if pools:
        out['source_pools'] = {'balanced': all(p['balanced'] for p in pools), 'sources': sum(p['sources'] for p in pools),
                               'assigned': [a for p in pools for a in p['assigned']]}
    return out
