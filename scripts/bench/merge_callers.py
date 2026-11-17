#!/usr/bin/env python3
"""Merge N caller hosts' evidence into one caller view.

Ported from the E74 harness (merge_callers.py), generalized from two to N
caller hosts and made an in-memory library for summarize_trial.py.

usage: merge_callers.py <trial-dir> [--out <dir>]
       (writes <out>/events.jsonl, report.json, caller.json; default
        <trial-dir>/merged/caller)

Each caller host ran its own generator with an even share of every stage on
the same timeline (start barrier), so a trial directory holds caller-1/,
caller-2/, ... (RFC7502_BENCHMARK.md §5). The merged view:

* puts every host's calls on the first host's clock: each host's elapsed_ms
  is shifted by its measured offset (the epoch of its elapsed 0, taken from
  its `run_started` event), so the cross-host time series (1 s bins,
  backpressure seconds, resource windows) are on one wall clock;
* places every call in its stage by its OWN host's schedule (`planned`):
  call `<run_id>-<n>` is the n-th call of that host's caller.json stages,
  due at the caller's schedule_offset(n) after its own `run_started`, so a
  start skew between hosts never moves a call into a neighbouring stage
  (expressed on the first host's elapsed clock: first run_start_ms + offset);
* namespaces call ids per host: the first host keeps `<run_id>-<n>`, host k
  becomes `<run_id>-c<k>-<n>`;
* sums counts (report counters, stage cps, calls, max_concurrent), AND-s the
  verdicts and shutdown outcomes, and keeps every host's sources.

The start skew (difference of the hosts' `run_started` instants) is reported;
above MAX_START_SKEW_SECONDS the trial's `caller_start_skew` check fails.
Standard library only.
"""
import argparse
import datetime
import json
from pathlib import Path
import re
import sys

MAX_START_SKEW_SECONDS = 2.0
ADDITIVE = ('attempted', 'caller_cleanup_failures', 'caller_watchdogs', 'capacity_503_attempts', 'failed',
            'generator_skipped', 'passed', 'planned', 'recovered_calls', 'retried_calls', 'shutdown_attempts',
            'shutdown_retryable_errors', 'signaling_attempts', 'endpoints')
# Run-level singletons of a caller's event stream: the first caller's are kept.
SINGLETON_EVENTS = ('run_started', 'caller_sources', 'shutdown_started', 'shutdown_finished', 'run_verdict')
# call_ended fields the summaries read (the rest of an observation is dropped to bound memory).
ENDED_KEYS = ('outcome', 'reason', 'phase', 'failure_class', 'failure_operation', 'sip_status', 'setup_ms',
              'first_audio_ms', 'audio_match', 'received_samples', 'received_frames', 'max_audio_gap_ms',
              'remote_ended', 'duration_ms', 'call_model', 'held_ms', 'caller_bye_ms', 'rfc6076', 'source_ip')
CALLER_DIR = re.compile(r'caller-(\d+)$')
CALL_INDEX = re.compile(r'(\d+)$')


def parse_stamp(text):
    """RFC 3339 to epoch seconds (any fraction length, 'Z' or an offset)."""
    text = text.replace('Z', '+00:00')
    match = re.fullmatch(r'(.*T\d\d:\d\d:\d\d)(?:\.(\d+))?(.*)', text)
    if match and match.group(2):
        text = match.group(1) + '.' + match.group(2)[:6].ljust(6, '0') + match.group(3)
    return datetime.datetime.fromisoformat(text).timestamp()


def caller_dirs(trial_dir):
    """caller-<i> directories of a trial, in host order."""
    found = []
    for entry in Path(trial_dir).iterdir():
        match = CALLER_DIR.match(entry.name)
        if match and entry.is_dir():
            found.append((int(match.group(1)), entry))
    return [path for _, path in sorted(found)]


def _json(path):
    try:
        return json.loads(Path(path).read_text())
    except (OSError, ValueError):
        return None


def parse_events(path):
    """A caller's events.jsonl -> run start and per-call schedule/send/establish/end (elapsed ms)."""
    parsed = {'run_started': None, 'run_start_epoch': None, 'run_start_ms': None, 'epoch0': None,
              'scheduled': {}, 'lateness': {}, 'sent': {}, 'established': {}, 'ended': {}, 'duplicate_ends': 0,
              'singletons': {}, 'sip_finals': {}}
    with open(path) as stream:
        for line in stream:
            if not line.strip():
                continue
            try:
                event = json.loads(line)
            except ValueError:
                continue
            fields = event.get('fields') or {}
            kind = fields.get('event')
            call_id = fields.get('call_id')
            if kind == 'call_scheduled':
                parsed['scheduled'][call_id] = fields['elapsed_ms']
                parsed['lateness'][call_id] = fields.get('scheduler_lateness_ms')
            elif kind == 'invite_sent':
                if call_id is not None:
                    parsed['sent'].setdefault(call_id, fields['elapsed_ms'])
            elif kind == 'call_established':
                parsed['established'].setdefault(call_id, fields['elapsed_ms'])
            elif kind == 'call_ended':
                parsed['duplicate_ends'] += int(call_id in parsed['ended'])
                parsed['ended'][call_id] = {key: fields[key] for key in ENDED_KEYS if key in fields}
            elif kind == 'run_started':
                stamp = parse_stamp(event['timestamp'])
                parsed.update(run_started=fields, run_start_epoch=stamp, run_start_ms=fields['elapsed_ms'],
                              epoch0=stamp - fields['elapsed_ms'] / 1000.0)
            elif kind == 'sip_final':
                key = str(fields.get('sip_status'))
                parsed['sip_finals'][key] = parsed['sip_finals'].get(key, 0) + 1
            if kind in SINGLETON_EVENTS:
                parsed['singletons'][kind] = fields
    return parsed


def load_caller(directory):
    """One caller host's evidence. `missing` names every absent input; events are None when absent."""
    directory = Path(directory)
    out = {'role': directory.name, 'dir': str(directory), 'missing': []}
    out['config'] = _json(directory / 'caller.json')
    report = _json(directory / 'report.json')
    if report is not None:
        report.pop('calls', None)   # per-call observations duplicate events.jsonl and bound memory
    out['report'] = report
    out['analysis'] = _json(directory / 'analysis.json')
    for name, value in (('caller.json', out['config']), ('report.json', report)):
        if value is None:
            out['missing'].append(name)
    events = directory / 'events.jsonl'
    out['events'] = parse_events(events) if events.exists() else None
    if out['events'] is None:
        out['missing'].append('events.jsonl')
    elif out['events']['run_start_epoch'] is None:
        out['missing'].append('run_started event')
        out['events'] = None
    if report is not None and (report.get('journal_sample') or 1) > 1:
        # A sampled journal holds only every n-th call: per-call scoring would be invented.
        out['missing'].append('events.jsonl (journal_sample %s > 1)' % report.get('journal_sample'))
        out['events'] = None
    return out


def schedule_offset_ms(index, stages, cps=None):
    """The caller's schedule_offset(index) in ms after its run start: the index-th call of `stages`
    ([{cps, seconds}], each a whole number of calls), or index / cps without stages. None past the plan."""
    if not stages:
        return index * 1000.0 / cps if cps else None
    elapsed, remaining = 0.0, index
    for stage in stages:
        count = int(round(stage['cps'] * stage['seconds']))
        if remaining < count:
            return (elapsed + remaining / float(stage['cps'])) * 1000.0
        remaining -= count
        elapsed += stage['seconds']
    return None


def call_index(call_id, label):
    """n of `<label>-<n>`, or None."""
    if not isinstance(call_id, str) or not label or not call_id.startswith(label + '-'):
        return None
    match = CALL_INDEX.fullmatch(call_id[len(label) + 1:])
    return int(match.group(1)) if match else None


def planned_offsets(events, config, label):
    """Call id -> ms after this host's own run start at which its scheduler was due to place the call.

    From the call's schedule index and this host's caller.json stages (exact); a call whose id or plan does not
    give it falls back to its own clock: call_scheduled elapsed minus scheduler_lateness_ms minus run_started.
    Returns (offsets, basis counts)."""
    stages = [s for s in (config or {}).get('stages') or []]
    cps = (config or {}).get('cps')
    offsets, basis = {}, {'call_index': 0, 'own_clock': 0}
    for call_id, at in events['scheduled'].items():
        index = call_index(call_id, label)
        offset = schedule_offset_ms(index, stages, cps) if index is not None else None
        if offset is None:
            offset = at - (events['lateness'].get(call_id) or 0.0) - events['run_start_ms']
            basis['own_clock'] += 1
        else:
            basis['call_index'] += 1
        offsets[call_id] = offset
    return offsets, basis


def namespace(call_id, label, tag):
    """`<label>-<n>` -> `<label>-<tag>-<n>` (other ids get a `<tag>-` prefix); unchanged without a tag."""
    if not tag or not isinstance(call_id, str):
        return call_id
    return label + '-' + tag + '-' + call_id[len(label) + 1:] if label and call_id.startswith(label + '-') else tag + '-' + call_id


def merge(callers):
    """Merge loaded callers (load_caller rows with events) onto the first one's clock.

    Returns {'roles', 'offsets' (s, schedule start vs the first host), 'start_skew_seconds',
    'run_start_epoch', 'run_start_ms', 'scheduled', 'lateness', 'sent', 'established', 'ended',
    'planned' (call id -> its own host's schedule, on the first host's elapsed clock: what places a call in its
    stage), 'planned_basis', 'host_of' (call id -> role), 'config', 'report'}.

    scheduled/sent/established are wall-clock aligned (cross-host time series); `planned` ignores the start
    skew, so stage membership is exact on every host."""
    usable = [c for c in callers if c.get('events')]
    if not usable:
        raise ValueError('no caller host with events')
    first = usable[0]['events']
    merged = {'roles': [c['role'] for c in usable], 'offsets': {}, 'scheduled': {}, 'lateness': {}, 'sent': {},
              'established': {}, 'ended': {}, 'host_of': {}, 'duplicate_ends': 0, 'planned': {},
              'planned_basis': {'call_index': 0, 'own_clock': 0},
              'run_start_epoch': first['run_start_epoch'], 'run_start_ms': first['run_start_ms']}
    for index, caller in enumerate(usable):
        events = caller['events']
        tag = '' if index == 0 else 'c%d' % (index + 1)
        label = ((caller.get('config') or {}).get('run_id') or (events.get('run_started') or {}).get('run_id') or '')
        shift = int(round((events['epoch0'] - first['epoch0']) * 1000))
        merged['offsets'][caller['role']] = round(events['run_start_epoch'] - first['run_start_epoch'], 3)
        merged['duplicate_ends'] += events['duplicate_ends']
        for key in ('scheduled', 'sent', 'established'):
            target = merged[key]
            for call_id, at in events[key].items():
                target[namespace(call_id, label, tag)] = at + shift
        for call_id, value in events['lateness'].items():
            merged['lateness'][namespace(call_id, label, tag)] = value
        planned, basis = planned_offsets(events, caller.get('config'), label)
        for call_id, offset in planned.items():
            merged['planned'][namespace(call_id, label, tag)] = first['run_start_ms'] + offset
        for key, count in basis.items():
            merged['planned_basis'][key] += count
        for call_id, fields in events['ended'].items():
            # The terminal fields are shared with the host's own view (no copy: they carry no clock).
            new_id = namespace(call_id, label, tag)
            merged['ended'][new_id] = fields
            merged['host_of'][new_id] = caller['role']
    starts = list(merged['offsets'].values())
    merged['start_skew_seconds'] = round(max(starts) - min(starts), 3)
    merged['config'] = merge_configs([c.get('config') for c in usable])
    merged['report'] = merge_reports([(c['role'], c.get('report')) for c in usable])
    return merged


def merge_configs(configs):
    """The sum of the hosts' shares: stage cps, cps, calls and max_concurrent summed (None when any is absent)."""
    if not configs or any(c is None for c in configs):
        return None
    merged = dict(configs[0])
    if all(len(c.get('stages') or []) == len(configs[0].get('stages') or []) for c in configs):
        merged['stages'] = [dict(stage, cps=sum(c['stages'][i]['cps'] for c in configs))
                            for i, stage in enumerate(configs[0].get('stages') or [])]
    for key in ('cps', 'calls', 'max_concurrent'):
        if all(key in c for c in configs):
            merged[key] = sum(c[key] for c in configs)
    merged['merged_from_callers'] = len(configs)
    return merged


def merge_reports(rows):
    """Summed counters, AND-ed verdicts, every host's sources and RFC 6076 trace coverage (None if any is absent)."""
    reports = [r for _, r in rows]
    if not reports or any(r is None for r in reports):
        return None
    report = {key: value for key, value in reports[0].items() if key != 'calls'}
    for key in ADDITIVE:
        if any(key in r for r in reports):
            report[key] = sum(r.get(key) or 0 for r in reports)
    report['interrupted'] = any(r.get('interrupted') for r in reports)
    report['verdict'] = 'pass' if all(r.get('verdict') == 'pass' for r in reports) else (
        'inconclusive' if any(r.get('verdict') == 'inconclusive' for r in reports) else 'fail')
    report['shutdown_outcome'] = next((r.get('shutdown_outcome') for r in reports if r.get('shutdown_outcome') != 'ok'), 'ok')
    report['shutdown_duration_ms'] = max(r.get('shutdown_duration_ms') or 0 for r in reports)
    report['sources'] = [dict(source, caller_host=role) for role, r in rows for source in r.get('sources') or []]
    if any('rfc6076_trace' in r for r in reports):
        report['rfc6076_trace'] = dict(reports[0].get('rfc6076_trace') or {}, **{
            key: sum((r.get('rfc6076_trace') or {}).get(key) or 0 for r in reports)
            for key in ('records', 'incomplete', 'table_dropped')})
    report['merged_from_callers'] = [role for role, _ in rows]
    return report


def write_view(trial_dir, out):
    """Write the merged caller view (events.jsonl with namespaced, shifted calls; report.json; caller.json)."""
    callers = [load_caller(d) for d in caller_dirs(trial_dir)]
    usable = [c for c in callers if c.get('events')]
    if not usable:
        raise SystemExit('no caller evidence under %s' % trial_dir)
    first = usable[0]['events']
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    with open(out / 'events.jsonl', 'w') as sink:
        for index, caller in enumerate(usable):
            tag = '' if index == 0 else 'c%d' % (index + 1)
            label = (caller.get('config') or {}).get('run_id') or ''
            shift = int(round((caller['events']['epoch0'] - first['epoch0']) * 1000))
            with open(Path(caller['dir']) / 'events.jsonl') as stream:
                for line in stream:
                    if not line.strip():
                        continue
                    event = json.loads(line)
                    fields = event.get('fields') or {}
                    if index and fields.get('event') in SINGLETON_EVENTS:
                        continue
                    if index:
                        if isinstance(fields.get('elapsed_ms'), (int, float)):
                            fields['elapsed_ms'] = fields['elapsed_ms'] + shift
                        if 'call_id' in fields:
                            fields['call_id'] = namespace(fields['call_id'], label, tag)
                    fields['caller_host'] = caller['role']
                    sink.write(json.dumps(event, separators=(',', ':')) + '\n')
    merged = merge(callers)
    (out / 'report.json').write_text(json.dumps(merged['report'], indent=1) + '\n')
    (out / 'caller.json').write_text(json.dumps(merged['config'], indent=1) + '\n')
    (out / 'merge.json').write_text(json.dumps({'roles': merged['roles'], 'offsets': merged['offsets'],
                                                'start_skew_seconds': merged['start_skew_seconds'],
                                                'max_start_skew_seconds': MAX_START_SKEW_SECONDS,
                                                'missing': {c['role']: c['missing'] for c in callers if c['missing']}},
                                               indent=1) + '\n')
    return out


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('trial_dir')
    parser.add_argument('--out')
    args = parser.parse_args(argv)
    out = write_view(args.trial_dir, args.out or Path(args.trial_dir) / 'merged' / 'caller')
    print(out)
    return 0


if __name__ == '__main__':
    sys.exit(main())
