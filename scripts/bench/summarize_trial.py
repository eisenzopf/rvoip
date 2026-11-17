#!/usr/bin/env python3
"""Summarize one RFC 7502 trial from its evidence directory.

Replaces the E74 harness's summarize-network.py (with stage_summary.py,
cohort_metrics.py and stage_outcomes.py folded in); the stage, bin and
backpressure logic is ported from E74, everything specific to the E74 worker
(NIC/XDP counters, executor sizing, recording fixtures) is dropped.

usage: summarize_trial.py <trial-dir> [--out <path>] [--criterion rfc|full]
       (default out: <trial-dir>/merged/summary.json; prints the verdict)

Input: the RFC7502_BENCHMARK.md §5 layout with the archives already
extracted:

    <trial-dir>/plan.json        trial plan: profile (named stages), row, rate,
                                 media, transport, hosts, criterion, dut_kind
    <trial-dir>/dut/             ready.json stats.jsonl summary.json
                                 metrics-*.prom dut.log resources.jsonl
                                 [tuning.json net-before.txt net-after.txt]
    <trial-dir>/caller-<i>/      caller.json events.jsonl report.json
                                 analysis.json caller.log resources.jsonl
                                 [tuning.json net-before.txt net-after.txt]

Per host, `resource.<role>.softirq` is the kernel receive-path readout
(caller_policy.softirq_saturation: peak per-CPU softirq share, saturated NIC
IRQ CPUs, first saturated second); on a caller it backs the
`no_irq_cpu_softirq_saturated` validity check, on the DUT it is recorded only
(`resource.dut.softirq_peak_share`, `softirq_saturated_cpus`). When
bench-agent.sh left net-before.txt / net-after.txt, `resource.<role>.net`
holds the counter deltas (NIC drops and ENA allowance counters, softnet
drops and time squeeze, UDP buffer errors, TCP timeouts, conntrack).

Output: one dict (schema `rvoip-bench-trial-summary/1`, see README.md) with
exactly what rfc7502.evaluate_trial, caller_policy, search.py and the report
read. A check whose input is missing is None and named in
`checks_missing`; nothing is invented. Standard library only.
"""
import argparse
import datetime
import json
import math
from pathlib import Path
import re
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import caller_policy  # noqa: E402
import host_sampler  # noqa: E402
import merge_callers  # noqa: E402
import rfc6076  # noqa: E402
import rfc7502  # noqa: E402

SCHEMA = 'rvoip-bench-trial-summary/1'
STEADY_SKIP_SECONDS = 15
PERCENTILES = [('p50', .5), ('p95', .95), ('p99', .99), ('p99_9', .999), ('max', 1.0)]
TIMEOUT_CLASSES = ('timeout', 'dialog_network', 'transport', 'caller_operation_timeout')
METRIC_PREFIX = 'rvoip_bench_uas_'
TRACE_COVERAGE_LIMIT = 0.01


# ---------------------------------------------------------------- per-call helpers (ported from stage_summary.py)

def quantile(values, q):
    ordered = sorted(value for value in values if value is not None)
    if not ordered:
        return None
    return ordered[min(len(ordered) - 1, max(0, math.ceil(q * len(ordered)) - 1))]


def percentiles(values):
    values = [value for value in values if value is not None]
    out = {name: quantile(values, q) for name, q in PERCENTILES}
    out['count'] = len(values)
    return out


def complete_call(call):
    """The call model's own pass: signaling-only and caller-hold calls are final when the caller passed them
    (for media rows the caller validated the DUT's echo of its tone over the whole hold)."""
    return call.get('sip_status') == 200 and call.get('outcome') == 'pass'


def build_calls(scheduled, sent, ended, lateness=None, planned=None):
    """One row per scheduled call: elapsed-ms schedule/send times (wall clock, for the 1 s bins), the planned
    time on its own host's schedule (`plan_ms`, for stage membership; the schedule time without a plan)
    and terminal fields."""
    lateness = lateness or {}
    planned = planned or {}
    return [{'id': cid, 'scheduled_ms': at, 'send_ms': sent.get(cid, at), 'plan_ms': planned.get(cid, at),
             'dispatched': cid in sent, 'scheduler_lateness_ms': lateness.get(cid), 'end': ended.get(cid)}
            for cid, at in scheduled.items()]


def aggregate(rows, seconds):
    ends = [row['end'] for row in rows if row['end'] is not None]
    admitted = [end for end in ends if end.get('sip_status') == 200]
    rejected = [end for end in ends if end.get('sip_status') == 503]
    full = sum(complete_call(end) for end in ends)
    offered = len(rows)
    return {
        'offered': offered, 'dispatched': sum(row['dispatched'] for row in rows),
        'terminal_observations': len(ends), 'missing_terminal_observations': offered - len(ends),
        'admitted': len(admitted), 'rejected_503': len(rejected), 'full': full,
        'generator_skipped': sum(end.get('reason') in ('generator_capacity', 'generator_media_ports') for end in ends),
        'admitted_incomplete': sum(not complete_call(end) for end in admitted),
        'full_of_offered_pct': 100 * full / offered if offered else None,
        'offered_cps': offered / seconds if seconds else None,
        'admitted_cps': len(admitted) / seconds if seconds else None,
        'setup_ms': percentiles([end.get('setup_ms') for end in admitted]),
        'first_audio_ms': percentiles([end.get('first_audio_ms') for end in admitted]),
        'rejection_503_duration_ms': {'p50': quantile([end.get('duration_ms') for end in rejected], .5),
                                      'p99': quantile([end.get('duration_ms') for end in rejected], .99)},
        # RFC 7501 session outcome and RFC 6076 metrics over the same calls.
        'rfc_session_ok': sum(rfc6076.rfc_session_ok(end) for end in ends),
        'rfc_attempt_ok': sum(rfc6076.rfc_attempt_ok(end) for end in ends),
        'rfc_disconnect_failures': sum(rfc6076.disconnect_failure(end) for end in ends),
        'rfc6076': rfc6076.metrics(ends),
    }


def stage_summaries(stages, run_start_ms, calls, transient=()):
    """Per stage: whole-stage denominators plus a steady scored window. A call belongs to the stage its own
    caller host scheduled it in (`plan_ms`: its schedule index on that host's stages, from the host's own
    run start), so neither a start skew between hosts nor a late dispatch moves it to another stage."""
    transient = set(transient)
    result = []
    elapsed = 0
    for index, stage in enumerate(stages):
        name = stage.get('name') or 'stage%d' % index
        begin = run_start_ms + elapsed * 1000
        finish = begin + stage['seconds'] * 1000
        is_transient = name in transient or stage['seconds'] < 60
        skip = 0 if is_transient else STEADY_SKIP_SECONDS
        scored_begin = begin + skip * 1000
        whole = [row for row in calls if begin <= row['plan_ms'] < finish]
        scored = [row for row in calls if scored_begin <= row['plan_ms'] < finish]
        row = {'name': name, 'index': index, 'requested_cps': stage['cps'], 'cps': stage['cps'], 'seconds': stage['seconds'],
               'start_seconds': elapsed, 'end_seconds': elapsed + stage['seconds'],
               'planned_offers': int(round(stage['cps'] * stage['seconds'])),
               'scheduled': len(whole),
               'transient': is_transient,
               'scoring': "whole stage by each call's own host schedule; steady window skips the first %d s" % skip,
               'whole_stage': aggregate(whole, stage['seconds']),
               'scored_window_seconds': stage['seconds'] - skip,
               'scored': aggregate(scored, stage['seconds'] - skip)}
        result.append(row)
        elapsed += stage['seconds']
    return result


def bins(stages, run_start_ms, calls):
    """1 s bins keyed by caller send time; offered keyed by schedule time."""
    total = sum(stage['seconds'] for stage in stages)
    rates, names = [], []
    for index, stage in enumerate(stages):
        rates += [stage['cps']] * stage['seconds']
        names += [stage.get('name') or 'stage%d' % index] * stage['seconds']
    rows = [{'t': second, 'stage': names[second], 'requested_cps': rates[second], 'offered': 0, 'dispatched': 0,
             'admitted': 0, 'rejected_503': 0, 'full': 0, 'guard_skips': 0, '_setup': []}
            for second in range(total)]
    for call in calls:
        scheduled = int((call['scheduled_ms'] - run_start_ms) // 1000)
        if 0 <= scheduled < total:
            rows[scheduled]['offered'] += 1
            if call['end'] is not None and call['end'].get('reason') in ('generator_capacity', 'generator_media_ports'):
                rows[scheduled]['guard_skips'] += 1
        sent = int((call['send_ms'] - run_start_ms) // 1000)
        if not 0 <= sent < total:
            continue
        target = rows[sent]
        target['dispatched'] += int(call['dispatched'])
        end = call['end']
        if end is None:
            continue
        if end.get('sip_status') == 200:
            target['admitted'] += 1
            target['_setup'].append(end.get('setup_ms'))
        elif end.get('sip_status') == 503:
            target['rejected_503'] += 1
        target['full'] += int(complete_call(end))
    for row in rows:
        row['setup_p99_ms'] = quantile(row.pop('_setup'), .99)
    return rows


def first_backpressure(bin_rows, calls=(), run_start_ms=0, dut_markers=()):
    """Earliest DUT backpressure marker (s since run start), or None, with its parts: the first on-wire 503,
    the first setup failure that is a DUT timeout, and DUT-side markers (first 503 in stats.jsonl)."""
    first_503 = min((row['t'] for row in bin_rows if row['rejected_503']), default=None)
    first_timeout = None
    for call in calls:
        end = call['end']
        if end and end.get('outcome') != 'pass' and end.get('sip_status') != 200 \
                and end.get('failure_class') in TIMEOUT_CLASSES:
            second = int((call['send_ms'] - run_start_ms) // 1000)
            first_timeout = second if first_timeout is None else min(first_timeout, second)
    dut = min((int(s) for s in dut_markers if s is not None), default=None)
    candidates = [s for s in (first_503, first_timeout, dut) if s is not None]
    return {'first_second': min(candidates) if candidates else None, 'first_503_second': first_503,
            'first_timeout_second': first_timeout, 'dut_first_503_second': dut}


# ---------------------------------------------------------------- evidence readers

def _json(path):
    try:
        return json.loads(Path(path).read_text())
    except (OSError, ValueError):
        return None


def _jsonl(path):
    rows = []
    try:
        with open(path) as stream:
            for line in stream:
                if line.strip():
                    try:
                        rows.append(json.loads(line))
                    except ValueError:
                        continue
    except OSError:
        return None
    return rows


def parse_prometheus(text):
    """[(name, labels dict, value)] of a Prometheus text exposition."""
    out = []
    for line in (text or '').splitlines():
        if not line or line.startswith('#'):
            continue
        match = re.match(r'^([a-zA-Z_:][a-zA-Z0-9_:]*)(\{([^}]*)\})?\s+(\S+)', line)
        if not match:
            continue
        labels = dict(re.findall(r'(\w+)="((?:[^"\\]|\\.)*)"', match.group(3) or ''))
        try:
            out.append((match.group(1), labels, float(match.group(4))))
        except ValueError:
            continue
    return out


def prom_value(series, name, **labels):
    values = [value for n, l, value in series if n == name and all(l.get(k) == v for k, v in labels.items())]
    return sum(values) if values else None


def counter(row, key, code=None):
    """A DUT counter from a stats.jsonl line or summary.json (top level or under `counters`);
    `code` reads one entry of a per-code dict ({"503": n})."""
    if row is None:
        return None
    for source in (row, row.get('counters') or {}):
        if key in source:
            value = source[key]
            if code is None:
                return value
            if isinstance(value, dict):
                return value.get(str(code), 0)
    return None


def thread_family(name):
    """Thread name without its trailing index (tokio-rt-worker-3 -> tokio-rt-worker)."""
    group = caller_policy.thread_group(name)
    return re.sub(r'[-_#]?\d+$', '', group) or group


def resource_summary(samples, window=None):
    """host_sampler.py rows -> peak and window figures. `window` = (start_epoch, end_epoch) of the scored stage."""
    if not samples:
        return None
    cpus = {row.get('logical_cpus') for row in samples if row.get('logical_cpus')}
    cores = {row.get('physical_cores') for row in samples if row.get('physical_cores')}
    inside = [row for row in samples if window and window[0] <= row['at'] <= window[1]]
    process = [row['cpu_cores_pct'] / 100 for row in samples if row.get('cpu_cores_pct') is not None]
    scored = [row['cpu_cores_pct'] / 100 for row in inside if row.get('cpu_cores_pct') is not None]
    groups = {}
    for first, last in zip(inside, inside[1:]):
        elapsed = last['at'] - first['at']
        if elapsed <= 0:
            continue
        before = {str(t['id']): t['user_time'] + t['system_time'] for t in first.get('thread_cpu_times') or []}
        interval = {}
        for thread in last.get('thread_cpu_times') or []:
            tid = str(thread['id'])
            if tid in before:
                family = thread_family((last.get('thread_names') or {}).get(tid))
                interval[family] = interval.get(family, 0.0) + max(0.0, thread['user_time'] + thread['system_time'] - before[tid])
        for family, seconds in interval.items():
            row = groups.setdefault(family, {'cpu_seconds': 0.0, 'wall_seconds': 0.0, 'peak_cores': 0.0})
            row['cpu_seconds'] += seconds
            row['wall_seconds'] += elapsed
            row['peak_cores'] = max(row['peak_cores'], seconds / elapsed)
    return {'samples': len(samples), 'scored_samples': len(inside),
            'logical_cpus': cpus.pop() if len(cpus) == 1 else (sorted(cpus) or None),
            'physical_cores': cores.pop() if len(cores) == 1 else (sorted(cores) or None),
            'sampled_peak_cores': max(process) if process else None,
            'scored_mean_cores': sum(scored) / len(scored) if scored else None,
            'scored_peak_cores': max(scored) if scored else None,
            'host_peak_cpu_pct': max((row['host_cpu_pct'] for row in samples if row.get('host_cpu_pct') is not None), default=None),
            'scored_host_mean_cpu_pct': (lambda v: sum(v) / len(v) if v else None)(
                [row['host_cpu_pct'] for row in inside if row.get('host_cpu_pct') is not None]),
            'peak_rss_bytes': max((row['rss_bytes'] for row in samples if row.get('rss_bytes') is not None), default=None),
            'peak_threads': max((row['threads'] for row in samples if row.get('threads') is not None), default=None),
            'scored_thread_groups': {family: {'mean_cores': row['cpu_seconds'] / row['wall_seconds'], 'peak_cores': row['peak_cores']}
                                     for family, row in sorted(groups.items()) if row['wall_seconds'] > 0}}


# ---------------------------------------------------------------- kernel receive path and network counters

def irq_cpus_from_tuning(host_dir):
    """(NIC IRQ CPUs, source) from the host's tuning.json (bench_tune.py: read-back affinity, else the plan)."""
    tuning = _json(Path(host_dir) / 'tuning.json') or {}
    iface = tuning.get('interface') or 'primary NIC'
    affinity = (tuning.get('readback') or {}).get('irq_affinity') or {}
    cpus = set()
    for value in affinity.values():
        cpus.update(host_sampler.parse_cpu_list(str(value)))
    if cpus:
        return sorted(cpus), 'tuning.json readback.irq_affinity (%s)' % iface
    planned = (tuning.get('plan') or {}).get('irq_cpus')
    if planned:
        return sorted(int(cpu) for cpu in planned), 'tuning.json plan.irq_cpus (%s)' % iface
    return None, None


def host_softirq(samples, host_dir, start_epoch=None, end_epoch=None, cutoff_epoch=None, run_start_epoch=None):
    irq_cpus, source = irq_cpus_from_tuning(host_dir)
    return caller_policy.softirq_saturation(samples, irq_cpus, source, start_epoch, end_epoch, cutoff_epoch, run_start_epoch)


# Counters read from bench-agent.sh net-*.txt; every one of them is a problem when it grows during a run.
NET_ETHTOOL_KEYS = ('rx_drops', 'tx_drops', 'pps_allowance_exceeded', 'bw_in_allowance_exceeded',
                    'bw_out_allowance_exceeded', 'conntrack_allowance_exceeded', 'linklocal_allowance_exceeded')
NET_SYSFS_KEYS = ('rx_dropped', 'rx_missed_errors', 'rx_fifo_errors', 'rx_over_errors', 'tx_dropped')
NET_SNMP_KEYS = ('UdpInErrors', 'UdpRcvbufErrors', 'UdpSndbufErrors', 'TcpExtTCPTimeouts', 'TcpExtListenDrops')
NET_SOFTNET_KEYS = ('softnet_dropped', 'softnet_time_squeeze')


def parse_net_snapshot(text):
    """bench-agent.sh net-*.txt -> {'at', 'iface', 'counters': {name: int}}. Sections start with '### <name>'."""
    out = {'at': None, 'iface': None, 'counters': {}}
    counters = out['counters']
    section, snmp_header = None, {}
    for raw in (text or '').splitlines():
        line = raw.rstrip()
        if line.startswith('### '):
            head = line[4:].strip()
            if head.startswith('at '):
                try:
                    out['at'] = float(head[3:])
                except ValueError:
                    pass
                section = None
            elif head.startswith('iface '):
                out['iface'] = head[6:].strip() or None
                section = None
            else:
                section = head.split()[0]
            continue
        if not line.strip() or section is None:
            continue
        if section in ('ethtool', 'sysfs'):
            key, sep, value = line.strip().partition(':')
            if sep:
                try:
                    counters[('' if section == 'ethtool' else 'sysfs_') + key.strip()] = int(value.strip())
                except ValueError:
                    pass
        elif section == 'softnet':
            fields = line.split()
            try:
                counters['softnet_processed'] = counters.get('softnet_processed', 0) + int(fields[0], 16)
                counters['softnet_dropped'] = counters.get('softnet_dropped', 0) + int(fields[1], 16)
                counters['softnet_time_squeeze'] = counters.get('softnet_time_squeeze', 0) + int(fields[2], 16)
            except (IndexError, ValueError):
                pass
        elif section == 'snmp':
            # /proc/net/snmp and /proc/net/netstat: a header line then a value line per protocol.
            proto, _, rest = line.partition(':')
            fields = rest.split()
            if proto not in snmp_header:
                snmp_header[proto] = fields
            else:
                for name, value in zip(snmp_header.pop(proto), fields):
                    try:
                        counters[proto + name] = int(value)
                    except ValueError:
                        pass
        elif section == 'conntrack':
            key, sep, value = line.partition(':')
            try:
                counters['conntrack_count'] = int((value if sep else key).strip())
            except ValueError:
                pass
    return out


def net_section(host_dir):
    """Counter deltas between net-before.txt and net-after.txt of one host dir, or None when either is absent."""
    host_dir = Path(host_dir)
    try:
        before = parse_net_snapshot((host_dir / 'net-before.txt').read_text(errors='replace'))
        after = parse_net_snapshot((host_dir / 'net-after.txt').read_text(errors='replace'))
    except OSError:
        return None
    a, b = before['counters'], after['counters']
    deltas = {}
    for key in NET_ETHTOOL_KEYS + tuple('sysfs_' + k for k in NET_SYSFS_KEYS) + NET_SNMP_KEYS + NET_SOFTNET_KEYS:
        if key in a and key in b:
            deltas[key] = b[key] - a[key]
    flagged = {key: value for key, value in deltas.items() if value > 0}
    return {'iface': after['iface'] or before['iface'], 'seconds': (after['at'] - before['at'])
            if after['at'] is not None and before['at'] is not None else None,
            'deltas': deltas, 'flagged': flagged,
            'softnet_processed': (b.get('softnet_processed', 0) - a.get('softnet_processed', 0))
            if 'softnet_processed' in a and 'softnet_processed' in b else None,
            'conntrack_count_before': a.get('conntrack_count'), 'conntrack_count_after': b.get('conntrack_count'),
            'conntrack_allowance_available': b.get('conntrack_allowance_available'),
            'missing': sorted(key for key in NET_ETHTOOL_KEYS[2:] + NET_SNMP_KEYS[:4] + NET_SOFTNET_KEYS
                              if key not in deltas)}


# ---------------------------------------------------------------- DUT

def dut_section(dut_dir, run_start_epoch, timeline_end_epoch, window):
    """DUT evidence: ready, drain summary (true_zero, clean_shutdown), counters, 503s, peaks. None = missing."""
    dut_dir = Path(dut_dir)
    missing = []
    ready = _json(dut_dir / 'ready.json')
    summary = _json(dut_dir / 'summary.json')
    stats = _jsonl(dut_dir / 'stats.jsonl')
    samples = _jsonl(dut_dir / 'resources.jsonl')
    for name, value in (('ready.json', ready), ('summary.json', summary), ('stats.jsonl', stats),
                        ('resources.jsonl', samples)):
        if value is None:
            missing.append(name)
    snapshots = sorted(dut_dir.glob('metrics-*.prom')) if dut_dir.is_dir() else []
    series, snapshot = [], None
    for path in snapshots:
        parsed = parse_prometheus(path.read_text(errors='replace'))
        invites = prom_value(parsed, METRIC_PREFIX + 'invites_received_total')
        best = prom_value(series, METRIC_PREFIX + 'invites_received_total') if series else None
        if not series or (invites or 0) >= (best or 0):
            series, snapshot = parsed, path.name
    if not snapshots:
        missing.append('metrics-*.prom')
    final = summary or (stats[-1] if stats else None)
    responses_503 = counter(summary, 'responses_sent_total', 503)
    responses_503_source = 'summary.json'
    if responses_503 is None and series and prom_value(series, METRIC_PREFIX + 'responses_sent_total') is not None:
        responses_503 = int(prom_value(series, METRIC_PREFIX + 'responses_sent_total', code='503') or 0)
        responses_503_source = snapshot
    first_503_epoch = None
    for row in stats or []:
        value = counter(row, 'responses_sent_total', 503)
        if value and (timeline_end_epoch is None or row.get('t', 0) <= timeline_end_epoch):
            first_503_epoch = row.get('t')
            break
    active = [counter(row, 'active_calls') for row in stats or []]
    active = [value for value in active if value is not None]
    cpu_peak = None
    if stats and len(stats) >= 2:
        peaks = []
        for first, last in zip(stats, stats[1:]):
            a, b = counter(first, 'process_cpu_seconds_total'), counter(last, 'process_cpu_seconds_total')
            if a is not None and b is not None and last.get('t') and first.get('t') and last['t'] > first['t']:
                peaks.append((b - a) / (last['t'] - first['t']))
        cpu_peak = max(peaks) if peaks else None
    resources = resource_summary(samples, window)
    out = {'present': dut_dir.is_dir(), 'missing': missing, 'ready': ready,
           'version': (ready or {}).get('version'), 'git_sha': (ready or {}).get('git_sha'),
           'config_sha256': (ready or {}).get('config_sha256'), 'transport': (ready or {}).get('transport'),
           'media': (ready or {}).get('media'),
           'true_zero': (summary or {}).get('true_zero'), 'clean_shutdown': (summary or {}).get('clean_shutdown'),
           'drain_seconds': (summary or {}).get('drain_seconds'),
           'started_at': (summary or {}).get('started_at'), 'ended_at': (summary or {}).get('ended_at'),
           'counters': {key: counter(final, key) for key in (
               'invites_received_total', 'calls_established_total', 'byes_received_total', 'admission_rejected_total',
               'rtp_packets_received_total', 'rtp_packets_sent_total', 'srtp_unprotect_failures_total',
               'responses_sent_total', 'byes_answered_total')} if final else None,
           'responses_503': responses_503, 'responses_503_source': responses_503_source if responses_503 is not None else None,
           'admission_rejected': counter(final, 'admission_rejected_total'),
           'first_503_epoch': first_503_epoch,
           'first_503_second': int(first_503_epoch - run_start_epoch) if first_503_epoch and run_start_epoch else None,
           'peak_active_calls': max(active) if active else None,
           'peak_cpu_cores': (resources or {}).get('sampled_peak_cores') if resources else cpu_peak,
           'peak_cpu_cores_source': 'resources.jsonl' if resources else ('stats.jsonl' if cpu_peak is not None else None),
           'peak_host_cpu_pct': (resources or {}).get('host_peak_cpu_pct'),
           'peak_rss_bytes': (resources or {}).get('peak_rss_bytes') or (
               max((counter(row, 'process_resident_memory_bytes') or 0 for row in stats), default=None) if stats else None),
           'logical_cpus': (resources or {}).get('logical_cpus'), 'physical_cores': (resources or {}).get('physical_cores'),
           'metrics_snapshot': snapshot}
    return out, resources


# ---------------------------------------------------------------- summary

def _stages_from(plan, config):
    """Named stages: plan.json's profile (authoritative names, total cps) checked against the merged caller config."""
    profile = (plan or {}).get('profile') or {}
    stages = profile.get('stages')
    transient = profile.get('transient_stages') or [s['name'] for s in stages or [] if not str(s['name']).startswith('trial-')]
    problems = []
    if stages and config and config.get('stages'):
        configured = config['stages']
        if [s['seconds'] for s in stages] != [s['seconds'] for s in configured] or any(
                abs(float(a['cps']) - float(b['cps'])) > 1e-6 for a, b in zip(stages, configured)):
            problems.append('plan.json profile stages differ from the callers\' summed stages')
    if not stages and config and config.get('stages'):
        # No plan: the callers' stages without names (no stage is a scored trial-* stage).
        stages = [{'name': 'stage%d' % i, 'cps': s['cps'], 'seconds': s['seconds']} for i, s in enumerate(config['stages'])]
        problems.append('plan.json profile missing: stages unnamed, no scored trial-* stage')
    return stages or [], transient, problems


def _caller_stages(stages, config):
    """This host's share of every named stage (its own caller.json cps)."""
    if not config or not config.get('stages') or len(config['stages']) != len(stages):
        return None
    return [dict(stage, cps=mine['cps']) for stage, mine in zip(stages, config['stages'])]


def media_section(applicable, scored_ends, dut):
    if not applicable:
        return {'applicable': False}
    established = [end for end in scored_ends if end.get('sip_status') == 200]
    validated = [end for end in established if end.get('outcome') == 'pass']
    audio_failed = [end for end in established if end.get('reason') in ('audio_validation', 'setup_or_no_audio')]
    counters = (dut or {}).get('counters') or {}
    return {'applicable': True, 'basis': 'calls answered 200 in the scored stage; validated = the caller passed the call '
                                         '(it checks the DUT echo of its own tone over the hold)',
            'established': len(established), 'validated_audio': len(validated), 'audio_failures': len(audio_failed),
            'validated_pct': 100.0 * len(validated) / len(established) if established else None,
            'audio_match': percentiles([end.get('audio_match') for end in established]),
            'first_audio_ms': percentiles([end.get('first_audio_ms') for end in established]),
            'max_audio_gap_ms': percentiles([end.get('max_audio_gap_ms') for end in established]),
            'dut_rtp_packets_received': counters.get('rtp_packets_received_total'),
            'dut_rtp_packets_sent': counters.get('rtp_packets_sent_total'),
            'dut_srtp_unprotect_failures': counters.get('srtp_unprotect_failures_total')}


def summarize(trial_dir, criterion=None):
    """The merged summary of one trial directory (see the module docstring)."""
    trial_dir = Path(trial_dir)
    plan = _json(trial_dir / 'plan.json')
    missing_files = [] if plan is not None else ['plan.json']
    dirs = merge_callers.caller_dirs(trial_dir) if trial_dir.is_dir() else []
    callers = [merge_callers.load_caller(d) for d in dirs]
    callers_missing = [c['role'] for c in callers if c['missing']]
    usable = [c for c in callers if c.get('events')]
    out = {'schema': SCHEMA, 'trial_dir': str(trial_dir),
           'generated_at': datetime.datetime.now(datetime.timezone.utc).isoformat(timespec='seconds'),
           'plan': {key: (plan or {}).get(key) for key in ('row', 'rate', 'index', 'criterion', 'media', 'transport',
                                                           'hold_ms', 'dut_kind', 'search')},
           'evidence': {'callers': [c['role'] for c in callers], 'callers_missing': callers_missing,
                        'missing': dict({c['role']: c['missing'] for c in callers if c['missing']},
                                        **({'trial': missing_files} if missing_files else {}))}}
    checks, checks_missing, not_applicable = {}, {}, {}

    def check(name, value, why_missing=None):
        checks[name] = value
        if value is None:
            checks_missing[name] = why_missing or 'input missing'

    merged = merge_callers.merge(callers) if usable else None
    config = merged['config'] if merged else None
    report = merged['report'] if merged else None
    stages, transient, stage_problems = _stages_from(plan, config)
    if plan and not (plan.get('profile') or {}).get('stages'):
        stage_problems.append('plan.json has no profile.stages')
    out['stage_problems'] = stage_problems
    run_start_epoch = merged['run_start_epoch'] if merged else None
    run_start_ms = merged['run_start_ms'] if merged else 0
    total_seconds = sum(stage['seconds'] for stage in stages)
    hold_s = ((report or {}).get('hold_ms') or (plan or {}).get('hold_ms') or 0) / 1000.0
    out['run_start_epoch'] = run_start_epoch
    out['caller_start_skew_seconds'] = merged['start_skew_seconds'] if merged else None
    out['stage_assignment'] = dict(merged['planned_basis'], basis='each call by its own host schedule') \
        if merged else None
    windows = [(row_start, row_end) for row_start, row_end in rfc7502.trial_windows(
        {'stage_summaries': [{'name': s['name'], 'seconds': s['seconds']} for s in stages]})]
    scored_window = (windows[0][0], windows[-1][1]) if windows else None
    out['trial_window'] = {'start_seconds': scored_window[0], 'end_seconds': scored_window[1],
                           'start_epoch': run_start_epoch + scored_window[0] if run_start_epoch else None,
                           'end_epoch': run_start_epoch + scored_window[1] if run_start_epoch else None} if scored_window else None
    epoch_window = (out['trial_window']['start_epoch'], out['trial_window']['end_epoch'] + hold_s) \
        if scored_window and run_start_epoch else None
    timeline_end = run_start_epoch + total_seconds + hold_s + 15 if run_start_epoch else None

    # DUT
    dut_dir = trial_dir / 'dut'
    dut, dut_resources = dut_section(dut_dir, run_start_epoch, timeline_end, epoch_window)
    out['dut'] = dut

    # Calls, bins, stages
    calls = build_calls(merged['scheduled'], merged['sent'], merged['ended'], merged['lateness'],
                        merged['planned']) if merged else []
    bin_rows = bins(stages, run_start_ms, calls) if stages else []
    backpressure = first_backpressure(bin_rows, calls, run_start_ms, [dut.get('first_503_second')])
    out['backpressure'] = backpressure
    bp = backpressure['first_second']
    out['stage_summaries'] = stage_summaries(stages, run_start_ms, calls, transient) if stages else []
    out['bins_1s'] = bin_rows

    # Caller validity per host, then combined
    by_caller, resources = {}, {}
    for caller in usable:
        role = caller['role']
        events = caller['events']
        offset = merged['offsets'][role]
        samples = _jsonl(Path(caller['dir']) / 'resources.jsonl') or []
        own_window = (epoch_window[0] + offset, epoch_window[1] + offset) if epoch_window else None
        resources[role] = resource_summary(samples, own_window)
        own_stages = _caller_stages(stages, caller.get('config'))
        if own_stages is None or caller.get('report') is None:
            by_caller[role] = {'caller_valid': False, 'caller_valid_before_backpressure': False, 'checks': {},
                               'missing_inputs': ['caller.json stages' if own_stages is None else 'report.json']}
            continue
        own_calls = build_calls(events['scheduled'], events['sent'], events['ended'], events['lateness'])
        own_bins = bins(own_stages, events['run_start_ms'], own_calls)
        own_bp = None if bp is None else int(math.floor(bp - offset))
        lateness = [row['scheduler_lateness_ms'] for row in own_calls
                    if own_bp is None or row['scheduled_ms'] < events['run_start_ms'] + own_bp * 1000]
        hosts = (plan or {}).get('caller_hosts')
        host = (hosts.get(role) if isinstance(hosts, dict) else None) or {}
        physical = (resources[role] or {}).get('physical_cores') or host.get('physical_cores')
        irq_cpus, irq_source = irq_cpus_from_tuning(caller['dir'])
        by_caller[role] = caller_policy.predicate(
            own_bins, lateness, samples, caller['report'], own_bp,
            physical_cores=physical if isinstance(physical, int) else None,
            run_start_epoch=events['run_start_epoch'], irq_cpus=irq_cpus, irq_cpus_source=irq_source,
            scored_from_second=scored_window[0] if scored_window else None)
        softirq = by_caller[role].get('softirq')
        if resources[role] is not None and softirq:
            resources[role].update(softirq_peak_share=softirq['peak_share'],
                                   softirq_saturated_cpus=softirq['saturated_cpus'], softirq=softirq)
    usable_roles = {c['role'] for c in usable}
    for caller in callers:
        if caller['role'] not in usable_roles:
            by_caller[caller['role']] = {'caller_valid': False, 'caller_valid_before_backpressure': False, 'checks': {},
                                         'missing_inputs': caller['missing']}
    validity = caller_policy.combine(by_caller, bp)
    out['caller_validity'] = validity
    out['caller_validity_by_caller'] = by_caller
    out['callers'] = {c['role']: {'start_offset_seconds': (merged or {}).get('offsets', {}).get(c['role']),
                                  'missing': c['missing'],
                                  'report': {key: (c.get('report') or {}).get(key) for key in (
                                      'verdict', 'planned', 'attempted', 'passed', 'failed', 'generator_skipped',
                                      'interrupted', 'shutdown_outcome', 'shutdown_class', 'caller_watchdogs',
                                      'caller_cleanup_failures', 'journal_sample', 'call_model', 'transport', 'hold_ms',
                                      'connections_per_source', 'endpoints', 'rfc6076_trace')} if c.get('report') else None}
                      for c in callers}
    if dut_resources is not None:
        # Recorded only, never a validity check: says whether the DUT's (or ea-uas's) receive path saturated.
        softirq = host_softirq(_jsonl(dut_dir / 'resources.jsonl'), dut_dir,
                               epoch_window[0] if epoch_window else None, epoch_window[1] if epoch_window else None,
                               run_start_epoch=run_start_epoch)
        dut_resources.update(softirq_peak_share=softirq['peak_share'], softirq_saturated_cpus=softirq['saturated_cpus'],
                             softirq=softirq)
    resources['dut'] = dut_resources
    for role, host_dir in [(c['role'], c['dir']) for c in callers] + [('dut', dut_dir)]:
        net = net_section(host_dir)
        if net is not None:
            resources[role] = dict(resources.get(role) or {}, net=net)
    out['resource'] = resources
    out['generator'] = {'hosts': len(callers), 'sources': (validity.get('source_pools') or {}).get('sources'),
                        'connections_per_source': (report or {}).get('connections_per_source'),
                        'endpoints': (report or {}).get('endpoints'),
                        'transport': (report or {}).get('transport'), 'call_model': (report or {}).get('call_model'),
                        'hold_ms': (report or {}).get('hold_ms')}

    # RFC 6076 and latency
    trial_rows = [row for row in out['stage_summaries'] if str(row['name']).startswith('trial-')]
    scored_ids = set()
    if trial_rows and merged:
        bounds = [(run_start_ms + r['start_seconds'] * 1000, run_start_ms + r['end_seconds'] * 1000) for r in trial_rows]
        scored_ids = {row['id'] for row in calls if any(a <= row['plan_ms'] < b for a, b in bounds)}
    scored_ends = [merged['ended'][cid] for cid in scored_ids if cid in merged['ended']] if merged else []
    all_ends = list(merged['ended'].values()) if merged else []
    out['rfc6076'] = {'all_calls': rfc6076.metrics(all_ends) if merged else None,
                      'scored': rfc6076.metrics(scored_ends) if trial_rows else None,
                      'caller_report_trace': (report or {}).get('rfc6076_trace'),
                      'call_model': (report or {}).get('call_model'), 'hold_ms': (report or {}).get('hold_ms'),
                      'transport': str((report or {}).get('transport') or '').replace('_', '-') or None}
    out['setup_ms'] = percentiles([end.get('setup_ms') for end in scored_ends if end.get('sip_status') == 200]) \
        if trial_rows else None
    out['final_status_counts'] = (out['rfc6076']['scored'] or {}).get('final_status_counts')
    out['disconnect_failures'] = {'all_calls': (out['rfc6076']['all_calls'] or {}).get('disconnect_failures'),
                                  'scored': (out['rfc6076']['scored'] or {}).get('disconnect_failures')}

    # Media
    call_model = (report or {}).get('call_model')
    media_plan = (plan or {}).get('media')
    if media_plan is not None:
        media_row = media_plan != 'none'
    elif call_model is not None:
        media_row = call_model != 'signaling-only'
    else:
        media_row = None
    out['media'] = media_section(bool(media_row), scored_ends, dut) if media_row is not None else None

    # Checks
    check('caller_evidence_complete', bool(callers) and not callers_missing)
    if report is not None:
        planned = report.get('planned') if report.get('planned') is not None else (config or {}).get('calls')
        check('caller_all_planned_attempts', report.get('attempted') == planned and not report.get('generator_skipped')
              and not report.get('interrupted'))
        check('caller_clean_shutdown', all((c.get('report') or {}).get('shutdown_outcome') == 'ok'
                                           and not (c.get('report') or {}).get('caller_watchdogs')
                                           and not (c.get('report') or {}).get('caller_cleanup_failures') for c in usable))
    else:
        check('caller_all_planned_attempts', None, 'report.json missing')
        check('caller_clean_shutdown', None, 'report.json missing')
    skew = out['caller_start_skew_seconds']
    check('caller_start_skew', None if skew is None else skew <= merge_callers.MAX_START_SKEW_SECONDS, 'no caller events')
    check('caller_valid_before_backpressure', bool(validity.get('caller_valid_before_backpressure')) if usable else None,
          'no caller events')
    check('scored_stage_present', bool(trial_rows))
    whole = [row['whole_stage'] for row in out['stage_summaries']]
    check('terminal_evidence_complete', (sum(w['missing_terminal_observations'] for w in whole) == 0) if whole else None,
          'no stages')
    scored_m = out['rfc6076']['scored']
    if scored_m and scored_m['invites']:
        gaps = scored_m['trace']['incomplete_records'] + scored_m['trace']['calls_without_record']
        check('rfc6076_trace_coverage', gaps <= TRACE_COVERAGE_LIMIT * scored_m['invites'])
    else:
        check('rfc6076_trace_coverage', None, 'no scored calls')
    if (plan or {}).get('dut_kind') == 'ea-uas':
        for name in ('dut_true_zero', 'dut_clean_shutdown'):
            not_applicable[name] = 'testbed baseline: the far end is ea-uas, not the DUT'
    else:
        check('dut_ready', True if dut['ready'] else None, 'dut/ready.json missing')
        check('dut_true_zero', dut['true_zero'] if isinstance(dut['true_zero'], bool) else None, 'dut/summary.json missing')
        check('dut_clean_shutdown', dut['clean_shutdown'] if isinstance(dut['clean_shutdown'], bool) else None,
              'dut/summary.json missing')
    media = out['media']
    if media and media.get('applicable'):
        sent = media.get('dut_rtp_packets_sent')
        echoed = media['validated_audio'] > 0 and (sent is None or sent > 0)
        check('media_echo_observed', echoed if media['established'] else None, 'no established call in the scored stage')
        check('media_all_audio_validated', media['validated_audio'] == media['established'] if media['established'] else None,
              'no established call in the scored stage')
        if (plan or {}).get('media') == 'srtp' or str((report or {}).get('transport')).startswith('tls'):
            failures = media.get('dut_srtp_unprotect_failures')
            if (plan or {}).get('dut_kind') != 'ea-uas':
                check('dut_srtp_unprotect_failures_zero', None if failures is None else failures == 0,
                      'DUT srtp_unprotect_failures_total missing')
    elif media_row is None:
        not_applicable['media_echo_observed'] = 'row media unknown (no plan.json media, no caller report)'
    out['checks'] = checks
    out['checks_missing'] = checks_missing
    out['checks_not_applicable'] = not_applicable
    if any(value is False for value in checks.values()):
        out['verdict'] = 'fail'
    elif checks_missing:
        out['verdict'] = 'incomplete'
    else:
        out['verdict'] = 'pass'
    if criterion:
        out['evaluation'] = rfc7502.trial_verdict(out, criterion)
    return out


def write(trial_dir, out_path=None, criterion=None):
    """summarize() and write it (default <trial-dir>/merged/summary.json); returns (path, summary)."""
    summary = summarize(trial_dir, criterion)
    out_path = Path(out_path) if out_path else Path(trial_dir) / 'merged' / 'summary.json'
    out_path.parent.mkdir(parents=True, exist_ok=True)
    tmp = out_path.with_suffix('.tmp')
    tmp.write_text(json.dumps(summary, indent=1, sort_keys=False) + '\n')
    tmp.replace(out_path)
    return out_path, summary


def main(argv=None):
    parser = argparse.ArgumentParser(description='Summarize one RFC 7502 trial directory.')
    parser.add_argument('trial_dir')
    parser.add_argument('--out')
    parser.add_argument('--criterion', choices=('rfc', 'full'), default=None,
                        help='also score the trial (default: the plan.json criterion, else rfc)')
    args = parser.parse_args(argv)
    plan = _json(Path(args.trial_dir) / 'plan.json') or {}
    criterion = args.criterion or plan.get('criterion') or 'rfc'
    path, summary = write(args.trial_dir, args.out, criterion)
    evaluation = summary.get('evaluation') or {}
    print(json.dumps({'summary': str(path), 'verdict': summary['verdict'], 'outcome': evaluation.get('outcome'),
                      'attempted': evaluation.get('attempted'), 'succeeded': evaluation.get('succeeded'),
                      'checks_failed': sorted(k for k, v in summary['checks'].items() if v is False),
                      'checks_missing': sorted(summary['checks_missing'])}))
    return 0


if __name__ == '__main__':
    sys.exit(main())
