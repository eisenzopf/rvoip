"""Synthetic trial evidence in the RFC7502_BENCHMARK.md §5 layout (dut/ + caller-<i>/), for the tests.

The caller files follow the rvoip-loadgen (E74 caller) output: events.jsonl lines
{"timestamp", "level", "target", "fields": {"event", "run_id", "elapsed_ms", "call_id", ...}},
report.json counters and caller.json stages; the DUT files follow §3 (ready.json, stats.jsonl,
summary.json, metrics-*.prom); resources.jsonl follows host_sampler.py.
"""
import datetime
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import rfc7502  # noqa: E402

T0 = 1_800_000_000.0   # a fixed epoch for the run start


def stamp(epoch):
    return datetime.datetime.fromtimestamp(epoch, tz=datetime.timezone.utc).isoformat()


def event(epoch, elapsed_ms, run_id, kind, call_id=None, **fields):
    row = dict(fields, event=kind, run_id=run_id, scenario='rfc7502', elapsed_ms=elapsed_ms)
    if call_id is not None:
        row['call_id'] = call_id
    return json.dumps({'timestamp': stamp(epoch), 'level': 'INFO', 'target': 'rvoip_loadgen', 'fields': row})


def default_outcome(stage, index_in_stage, rate, limit, reject_status=503):
    """Status of one call: `reject_status` (503; None = no final response, a timeout) for the share of the
    trial stage above `limit` (a DUT capacity), else 200."""
    if limit is not None and stage.startswith('trial-') and rate > limit and (index_in_stage % rate) >= limit:
        return reject_status
    return 200


def write_caller(directory, profile, host_index, hosts, run_start_epoch, rate, *, limit=None, media=False,
                 hold_ms=0, process_offset_ms=400, lateness_ms=0.4, audio_fail_every=None, thread_cores=0.3,
                 process_cores=3.0, transport='tls_srtp', skip_events=False, bye_status=200, drop_bins=(),
                 reject_status=503, setup_offset_ms=0.0, saturate=None, irq_cpus=None, net=None):
    """One caller host's share of `profile`. Returns the number of calls scheduled. `saturate` = (cpu, first, last)
    sampler intervals (sample i is taken at run start - 2 + i) at 98 % softirq; `net` = (before, after) texts."""
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    run_id = 'trial'
    epoch0 = run_start_epoch - process_offset_ms / 1000.0
    run_start_ms = process_offset_ms
    lines = [event(run_start_epoch, run_start_ms, run_id, 'run_started', offered_cps=rate, stages=[])]
    stages = []
    calls = 0
    at = 0
    counters = {'passed': 0, 'failed': 0}
    for stage in profile['stages']:
        share = stage['cps'] / hosts
        stages.append({'cps': share, 'seconds': stage['seconds']})
        count = int(round(share * stage['seconds']))
        for k in range(count):
            scheduled = run_start_ms + at * 1000 + int(k * 1000 / share)
            second = (scheduled - run_start_ms) // 1000
            if second in drop_bins and k % 2:
                continue
            call_id = '%s-%d' % (run_id, calls)
            calls += 1
            global_index = k * hosts + host_index
            status = default_outcome(stage['name'], global_index, stage['cps'], limit, reject_status)
            sched_epoch = epoch0 + scheduled / 1000.0
            lines.append(event(sched_epoch, scheduled, run_id, 'call_scheduled', call_id, scheduler_lateness_ms=lateness_ms))
            sent = scheduled + 1
            lines.append(event(sched_epoch, sent, run_id, 'invite_sent', call_id, session_id='s%d' % calls))
            invite_ms = int((epoch0 * 1000) + sent)
            if status == 200:
                setup = 4.0 + (k % 7) + setup_offset_ms
                final_at = invite_ms + int(setup)
                bye_at = final_at + hold_ms
                record = {'invite_at_ms': invite_ms, 'invite_transmissions': 1, 'first_response_status': 200,
                          'first_response_at_ms': final_at, 'final_status': 200, 'final_at_ms': final_at,
                          'bye_origin': 'caller', 'bye_at_ms': bye_at, 'bye_transmissions': 1,
                          'bye_final_status': bye_status, 'bye_final_at_ms': bye_at + 2, 'srd_ms': setup,
                          'srd_kind': 'successful', 'sdd_ms': 2.0 if 200 <= bye_status < 300 else None,
                          'sdt_ms': float(hold_ms), 'established': True,
                          'completed': 200 <= bye_status < 300, 'trace_complete': True}
                lines.append(event(sched_epoch, sent + int(setup), run_id, 'call_established', call_id, setup_ms=setup,
                                   sip_status=200))
                ended = {'sip_status': 200, 'setup_ms': setup, 'outcome': 'pass',
                         'reason': 'recording_verified' if media else 'signaling_verified',
                         'call_model': 'caller-hold' if media else 'signaling-only', 'held_ms': float(hold_ms),
                         'duration_ms': setup + hold_ms + 2, 'rfc6076': record}
                if media:
                    ended.update(first_audio_ms=42.0, audio_match=0.97, received_samples=8000 * max(1, hold_ms // 1000),
                                 max_audio_gap_ms=21.0)
                    if audio_fail_every and calls % audio_fail_every == 0:
                        ended.update(outcome='fail', reason='audio_validation', audio_match=0.2)
                if bye_status >= 300:
                    ended['outcome'] = 'fail'
            else:
                record = {'invite_at_ms': invite_ms, 'invite_transmissions': 1, 'first_response_status': status,
                          'first_response_at_ms': invite_ms + 1, 'final_status': status, 'final_at_ms': invite_ms + 1,
                          'bye_transmissions': 0, 'srd_ms': 1.0, 'srd_kind': 'failed', 'established': False,
                          'completed': False, 'trace_complete': True}
                ended = {'sip_status': status, 'setup_ms': 0.0, 'outcome': 'fail', 'reason': 'setup_or_no_audio',
                         'failure_class': 'sip_rejected' if status is not None else 'timeout',
                         'call_model': 'caller-hold' if media else 'signaling-only',
                         'duration_ms': 1.0, 'rfc6076': record}
            counters['passed' if ended['outcome'] == 'pass' else 'failed'] += 1
            end_ms = sent + int(ended['duration_ms']) + 1
            lines.append(event(epoch0 + end_ms / 1000.0, end_ms, run_id, 'call_ended', call_id, **ended))
        at += stage['seconds']
    total = sum(s['seconds'] for s in stages)
    lines.append(event(run_start_epoch + total + 1, run_start_ms + (total + 1) * 1000, run_id, 'run_verdict', outcome='pass'))
    if not skip_events:
        (directory / 'events.jsonl').write_text('\n'.join(lines) + '\n')
    config = {'run_id': run_id, 'scenario': 'rfc7502', 'stages': stages, 'calls': calls,
              'cps': max(s['cps'] for s in stages), 'max_concurrent': 20000, 'hold_ms': hold_ms,
              'signaling_only': not media, 'transport': transport.replace('_', '-')}
    (directory / 'caller.json').write_text(json.dumps(config))
    report = {'schema_version': 2, 'run_id': run_id, 'verdict': 'pass' if not counters['failed'] else 'fail',
              'planned': calls, 'attempted': calls, 'passed': counters['passed'], 'failed': counters['failed'],
              'generator_skipped': 0, 'interrupted': False, 'shutdown_outcome': 'ok', 'shutdown_class': 'none',
              'caller_watchdogs': 0, 'caller_cleanup_failures': 0, 'journal_sample': 1,
              'call_model': 'caller-hold' if media else 'signaling-only', 'hold_ms': hold_ms, 'transport': transport,
              'rfc6076_trace': {'records': calls, 'incomplete': 0, 'table_dropped': 0, 'wait_ms': 250},
              'connections_per_source': 8, 'endpoints': 16,
              'sources': [{'assigned': calls // 2 + (calls % 2), 'spilled_in': 0, 'preferred_full': 0, 'peak_active': 10,
                           'failed': 0, 'shutdown': {'outcome': 'ok'}},
                          {'assigned': calls // 2, 'spilled_in': 0, 'preferred_full': 0, 'peak_active': 10, 'failed': 0,
                           'shutdown': {'outcome': 'ok'}}],
              'calls': []}
    (directory / 'report.json').write_text(json.dumps(report))
    (directory / 'analysis.json').write_text(json.dumps({'attempted': calls}))
    write_resources(directory / 'resources.jsonl', run_start_epoch - 2, total + hold_ms / 1000.0 + 4,
                    process_cores, thread_cores, ['loadgen-signaling', 'loadgen-call-0', 'loadgen-call-1', 'rvoip-media'],
                    logical=96, physical=48, saturate=saturate, irq_cpus=irq_cpus)
    if net:
        (directory / 'net-before.txt').write_text(net[0])
        (directory / 'net-after.txt').write_text(net[1])
    return calls


SAMPLED_CPUS = 4   # per-CPU /proc/stat lines in synthetic samples (USER_HZ 100, 1 s apart)


def cpu_stat(i, saturate=None):
    """host.stat of synthetic sample `i`: cumulative per-CPU ticks [user nice system idle iowait irq softirq steal],
    2 % softirq per interval; `saturate` = (cpu, first_i, last_i) puts that CPU at 98 % softirq in those intervals."""
    stat = {}
    for cpu in range(SAMPLED_CPUS):
        hot = 0
        if saturate and cpu == saturate[0]:
            hot = max(0, min(i, saturate[2]) - saturate[1] + 1)   # intervals ending at first_i..last_i so far
        normal = i - hot
        stat['cpu%d' % cpu] = [10 * normal + 1 * hot, 0, 0, 88 * normal + 1 * hot, 0, 0, 2 * normal + 98 * hot, 0]
    stat['cpu'] = [sum(v[k] for v in stat.values()) for k in range(8)]
    return stat


def write_resources(path, start, seconds, process_cores, thread_cores, names, logical, physical, hot_at=None,
                    saturate=None, irq_cpus=None):
    rows = []
    for i in range(int(seconds) + 1):
        at = start + i
        threads = [{'id': 100 + j, 'user_time': thread_cores * i * 0.8, 'system_time': thread_cores * i * 0.2}
                   for j in range(len(names))]
        if hot_at is not None and i >= hot_at:
            threads[0]['user_time'] += (i - hot_at + 1) * 1.0
        row = {'at': at, 'pid': 4242, 'logical_cpus': logical, 'physical_cores': physical, 'rss_bytes': 300 * 2 ** 20,
               'threads': len(names), 'thread_cpu_times': threads,
               'thread_names': {str(100 + j): name for j, name in enumerate(names)},
               'host': {'stat': cpu_stat(i, saturate)}}
        if irq_cpus is not None:
            row['irq_iface'], row['irq_cpus'] = 'ens5', irq_cpus
        if i:
            row['cpu_cores_pct'] = 100.0 * process_cores
            row['host_cpu_pct'] = 12.5
        rows.append(json.dumps(row))
    Path(path).write_text('\n'.join(rows) + '\n')


def write_dut(directory, run_start_epoch, profile, *, limit=None, true_zero=True, clean=True, skip=(), media=None,
              srtp_failures=0, saturate=None):
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    total = sum(s['seconds'] for s in profile['stages'])
    invites = ok = rejected = 0
    stats, at = [], 0
    for stage in profile['stages']:
        for second in range(stage['seconds']):
            cps = stage['cps']
            bad = max(0, cps - limit) if limit is not None and stage['name'].startswith('trial-') else 0
            invites += cps
            ok += cps - bad
            rejected += bad
            stats.append({'t': run_start_epoch + at + second + 1, 'invites_received_total': invites,
                          'responses_sent_total': {'200': ok, '503': rejected}, 'calls_established_total': ok,
                          'active_calls': cps, 'pending_setups': 0, 'pending_cleanups': 0, 'open_dialogs': cps,
                          'process_cpu_seconds_total': 0.5 * (at + second + 1), 'process_resident_memory_bytes': 64 * 2 ** 20,
                          'rtp_packets_received_total': ok * 50 if media else 0, 'rtp_packets_sent_total': ok * 50 if media else 0,
                          'srtp_unprotect_failures_total': srtp_failures})
        at += stage['seconds']
    files = {
        'ready.json': {'pid': 4242, 'started_at': run_start_epoch - 5, 'transport': 'tls', 'listen': '10.0.0.10:5061',
                       'media': media or 'none', 'metrics_listen': '0.0.0.0:9464', 'version': '0.4.1',
                       'git_sha': 'abcdef1234567890', 'config_sha256': '0' * 64},
        'summary.json': {'counters': dict(stats[-1], t=None), 'true_zero': true_zero, 'clean_shutdown': clean,
                         'drain_seconds': 1.5, 'started_at': run_start_epoch - 5, 'ended_at': run_start_epoch + total + 3},
    }
    for name, value in files.items():
        if name not in skip:
            (directory / name).write_text(json.dumps(value))
    if 'stats.jsonl' not in skip:
        (directory / 'stats.jsonl').write_text('\n'.join(json.dumps(r) for r in stats) + '\n')
    if 'metrics' not in skip:
        (directory / 'metrics-final.prom').write_text(
            '# TYPE rvoip_bench_uas_invites_received_total counter\n'
            'rvoip_bench_uas_invites_received_total %d\n'
            'rvoip_bench_uas_responses_sent_total{code="200"} %d\n'
            'rvoip_bench_uas_responses_sent_total{code="503"} %d\n' % (invites, ok, rejected))
    if 'resources.jsonl' not in skip:
        write_resources(directory / 'resources.jsonl', run_start_epoch - 2, total + 4, 4.0, 0.5,
                        ['rvoip-bench-uas', 'tokio-rt-worker-0', 'tokio-rt-worker-1'], logical=16, physical=8,
                        saturate=saturate)


def make_trial(trial_dir, rate, *, hosts=2, n=None, warmup_seconds=4, minimum_seconds=6, limit=None, media=False,
               hold_ms=0, skew=0.0, dut_kwargs=None, caller_kwargs=None, skip_callers=(), row='R1', run_start=T0,
               plan=True):
    """A complete trial directory; returns its profile."""
    trial_dir = Path(trial_dir)
    profile = rfc7502.trial_profile(rate, n or rate * minimum_seconds, warmup_seconds, minimum_seconds, hosts=hosts)
    trial_dir.mkdir(parents=True, exist_ok=True)
    if plan:
        (trial_dir / 'plan.json').write_text(json.dumps({
            'row': row, 'rate': rate, 'index': 0, 'criterion': 'rfc', 'profile': profile,
            'media': ('srtp' if media else 'none'), 'transport': 'tls', 'hold_ms': hold_ms,
            'caller_hosts': {'caller-%d' % (i + 1): {'physical_cores': 48} for i in range(hosts)}}))
    for i in range(hosts):
        if i + 1 in skip_callers:
            (trial_dir / ('caller-%d' % (i + 1))).mkdir(parents=True, exist_ok=True)
            continue
        write_caller(trial_dir / ('caller-%d' % (i + 1)), profile, i, hosts, run_start + (skew if i else 0.0), rate,
                     limit=limit, media=media, hold_ms=hold_ms, process_offset_ms=400 + 100 * i, **(caller_kwargs or {}))
    write_dut(trial_dir / 'dut', run_start, profile, limit=limit, media=('srtp' if media else None), **(dut_kwargs or {}))
    return profile
