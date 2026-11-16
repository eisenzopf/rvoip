"""Glue between run_lane.py and the ported analysis modules: caller / ea-uas
configs, RFC 6076 latency budgets, the R4 overload verdict, the report spec
and recorded rates. Standard library only."""
import datetime
import json
from pathlib import Path

import rfc_report
import rfc_report_ietf

LATENCY_KEYS = ('p50', 'p95', 'p99', 'p99_9')
DID = '+15550100000'


# ---------------------------------------------------------------------------
# Configs (crates/sip/rvoip-sip/docs/RFC7502_BENCHMARK.md §3-§4)
# ---------------------------------------------------------------------------

def caller_config(row, run_id, sources, connections, dut_ip, dut_port, stages, output):
    """rvoip-loadgen config for one caller host (this host's share of every stage)."""
    hold = row['hold_ms']
    signaling_only = row['media'] == 'none'
    calls = sum(round(s['cps'] * s['seconds']) for s in stages)
    peak = max(s['cps'] for s in stages)
    # Concurrency: calls in flight for the hold, setup and teardown, with margin.
    concurrent = min(35000 * len(sources), int(peak * (hold / 1000 + 12)) + 2000)
    return {
        'run_id': run_id,
        'scenario': row.get('id', 'bench'),
        'bind_ip': sources[0],
        'bind_ips': list(sources),
        'connections_per_source': connections,
        'contact_host': sources[0],
        'targets': [{'name': 'dut', 'host': dut_ip}],
        'target_sip_port': dut_port,
        'local_sip_port': 5061,
        'transport': 'tls-srtp' if row['transport'] == 'tls' else 'udp-rtp',
        'tls_cert_path': '/opt/rvoip-bench/tls/caller.crt',
        'tls_key_path': '/opt/rvoip-bench/tls/caller.key',
        'tls_ca_path': '/opt/rvoip-bench/tls/ca.crt',
        'did': DID,
        'codec': 'pcmu',
        'rtcp_mux_required': True,
        'media_port_start': 30000,
        'media_port_end': 59999,
        'calls': calls,
        'cps': peak,
        'stages': stages,
        'max_concurrent': concurrent,
        'hold_ms': hold,
        'signaling_only': signaling_only,
        'journal_sample': 1,
        'output': output,
        'linger_seconds': 30,
    }


def ea_uas_config(row, ip, port):
    """ea-uas (RFC 7502 §6.1 testbed UAS) config; echoes media for media rows."""
    tls = row['transport'] == 'tls'
    config = {
        'bind_ip': ip,
        'port': port,
        'transport': 'tls' if tls else 'udp',
        'contact_host': ip,
        'advertised_ip': ip,
        'sdp_port_range': [10000, 64999],
        'codecs': ['pcmu'],
        'stats_path': '/var/lib/rvoip-bench/run/stats.jsonl',
        'summary_path': '/var/lib/rvoip-bench/run/summary.json',
        'stats_interval_ms': 1000,
        'max_dialogs': 1000000,
        'media_echo': row['media'] != 'none',
    }
    if tls:
        config.update(tls_cert_path='/opt/rvoip-bench/tls/dut.crt', tls_key_path='/opt/rvoip-bench/tls/dut.key',
                      tls_ca_path='/opt/rvoip-bench/tls/ca.crt', tls_server_name=ip)
    return config


# ---------------------------------------------------------------------------
# RFC 6076 latency
# ---------------------------------------------------------------------------

def _block(summary):
    return ((summary or {}).get('rfc6076') or {}).get('scored') or {}


def srd(summary):
    d = _block(summary).get('srd_successful_ms') or {}
    return {k: d.get(k) for k in LATENCY_KEYS + ('max', 'count')} if d else None


def sdd(summary):
    d = _block(summary).get('sdd_ms') or {}
    return {k: d.get(k) for k in LATENCY_KEYS + ('max', 'count')} if d else None


def latency_verdict(summary, recorded, tolerance, rate):
    """SRD/SDD p50..p99.9 at 0.9R against the recorded values: a regression beyond
    max(relative x baseline, floor_ms) fails (the perf_audit.py rule)."""
    now = {'srd_ms': srd(summary), 'sdd_ms': sdd(summary)}
    out = dict(rate=rate, tolerance=tolerance, **now)
    base = {'srd_ms': (recorded or {}).get('srd_ms'), 'sdd_ms': (recorded or {}).get('sdd_ms')}
    out['baseline_srd_ms'], out['baseline_sdd_ms'] = base['srd_ms'], base['sdd_ms']
    if not now['srd_ms']:
        out.update(verdict='fail', reason='no SRD measured at %d sps' % rate)
        return out
    if not base['srd_ms']:
        out.update(verdict='pass', reason='no recorded latency baseline; values recorded only')
        return out
    regressions = []
    for metric in ('srd_ms', 'sdd_ms'):
        for key in LATENCY_KEYS:
            b, v = (base[metric] or {}).get(key), (now[metric] or {}).get(key)
            if b is None or v is None:
                continue
            limit = b + max(tolerance['relative'] * b, tolerance['floor_ms'])
            if v > limit:
                regressions.append('%s %s %.1f ms > %.1f ms (baseline %.1f)' % (metric[:3].upper(), key, v, limit, b))
    out['verdict'] = 'fail' if regressions else 'pass'
    if regressions:
        out['reason'] = '; '.join(regressions)
    return out


# ---------------------------------------------------------------------------
# R4 overload
# ---------------------------------------------------------------------------

def overload_verdict(summary, ceiling, profile):
    """Pass: every failure in the overload stages is a 503 (admission sheds),
    and both recovery stages establish every attempt."""
    stages = {s.get('name'): s for s in (summary or {}).get('stage_summaries') or []}
    names = [s['name'] for s in profile['stages'] if s['name'].startswith('trial-')]
    rows, problems = [], []
    for k, name in enumerate(names):
        st = (stages.get(name) or {}).get('whole_stage') or {}
        counts = ((st.get('rfc6076') or {}).get('final_status_counts')) or {}
        offered = st.get('offered') or 0
        established = int(counts.get('200', 0))
        rejected = int(counts.get('503', 0))
        other = sum(int(v) for code, v in counts.items() if code not in ('200', '503'))
        rows.append({'stage': name, 'offered_cps': (stages.get(name) or {}).get('requested_cps'), 'offered': offered,
                     'established': established, 'rejected_503': rejected, 'other_failures': other,
                     'setup_p99_ms': (st.get('setup_ms') or {}).get('p99')})
        if other:
            problems.append('%s: %d non-503 failures %s' % (name, other, counts))
        if k >= len(names) - 2 and offered and established != offered:
            problems.append('recovery %s: %d of %d established' % (name, established, offered))
        if not offered:
            problems.append('%s: no attempts recorded' % name)
    verdict = 'fail' if problems else 'pass'
    if summary is None:
        verdict, problems = 'infra', ['no merged summary']
    return {'verdict': verdict, 'reason': '; '.join(problems) or None, 'stages': rows}


# ---------------------------------------------------------------------------
# Report and recorded rates
# ---------------------------------------------------------------------------

def render_report(ctx, rows, env_id):
    lane = ctx.lane.info
    facts = next(iter((lane.get('facts') or {}).values()), {})
    spec = {
        'title': 'RFC 7501/7502 benchmark: rvoip-bench-uas on %s' % lane.get('dut_type'),
        'report_date': datetime.date.today().isoformat(),
        'status': 'final', 'profile': ctx.args.profile,
        'evidence_root': str(ctx.args.local), 'evidence_uri': ctx.s3_run,
        'dut': {'name': 'ea-uas (testbed baseline)' if ctx.lane.ea_uas else 'rvoip-bench-uas',
                'git_sha': ctx.args.candidate, 'instance_type': lane.get('dut_type'),
                'vcpu': 16 if lane.get('dut_type') == 'c8i.4xlarge' else None,
                'physical_cores': 8 if lane.get('dut_type') == 'c8i.4xlarge' else None},
        'testbed': {'caller_hosts': len(lane.get('callers', [])), 'caller_type': lane.get('caller_type'),
                    'sources_per_host': min(len(c['ips']) for c in lane['callers']),
                    'connections_per_source': 8},
        'environment': {'env_id': env_id, 'region': ctx.args.region, 'ami': lane.get('ami_id'),
                        'az': lane.get('az'), 'kernel': facts.get('kernel')},
        'rows': [{'row': r['id'], 'search': '%s/search.json' % r['id'], 'trials': '%s/trials.jsonl' % r['id']}
                 for r in rows if r['id'] != 'R4' and r.get('search')],
        'summary': [], 'findings': [], 'deviations': [
            'Caller hosts: %d x %s, %d source IPs each, 8 connections per source (one TLS connection or UDP 5-tuple per endpoint).'
            % (len(lane.get('callers', [])), lane.get('caller_type'), min(len(c['ips']) for c in lane['callers'])),
            'Signaling-only rows: the DUT answers the SDP offer without allocating RTP (media: none).',
        ],
        'assumptions': [],
    }
    r4 = next((r for r in rows if r['id'] == 'R4'), None)
    if r4 and r4.get('trials'):
        spec['overload'] = {'row': 'R4', 'trial': r4['trials'][0]['run']}
    out = ctx.args.local / 'report'
    (out / 'spec.json').parent.mkdir(parents=True, exist_ok=True)
    (out / 'spec.json').write_text(json.dumps(spec, indent=1, sort_keys=True))
    paths = {}
    for module, stem in ((rfc_report, 'rfc7502-report'), (rfc_report_ietf, 'rfc7502-ietf-report')):
        md, html = module.render(spec, out, stem=stem)
        for p in (md, html):
            ctx.aws.s3_cp(p, ctx.s3_run + 'report/' + p.name)
        paths[stem] = {'md': ctx.s3_run + 'report/' + md.name, 'html': ctx.s3_run + 'report/' + html.name}
    ctx.aws.s3_cp(out / 'spec.json', ctx.s3_run + 'report/spec.json')
    return paths


def record_rates(rows, env_id, baselines, ctx, profile):
    """Write perf-baselines/rfc7502/<env-id>/rates.json from a minor/full (R) or baseline (tester caps) run."""
    path = Path(baselines) / env_id / 'rates.json'
    current = json.loads(path.read_text()) if path.exists() else {
        'schema': 'rvoip-rfc7502-rates-v1', 'env_id': env_id, 'rows': {}, 'tester_caps': {}, 'baseline_sps': {}}
    now = datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')
    for r in rows:
        if r.get('r') is None:
            continue
        if profile == 'baseline':
            current['baseline_sps'][r['id']] = {'r': r['r'], 'tester_limited': r.get('tester_limited'),
                                                'source_run': ctx.s3_run, 'recorded_at': now}
            q = 4
            current['tester_caps'][r['id']] = max(q, int(r['r'] * 0.67) // q * q)
        else:
            lat = r.get('latency') or {}
            current['rows'][r['id']] = {'r': r['r'], 'tester_limited': r.get('tester_limited'),
                                        'tester_cap': current['tester_caps'].get(r['id']),
                                        'srd_ms': lat.get('srd_ms'), 'sdd_ms': lat.get('sdd_ms'),
                                        'latency_rate': lat.get('rate'),
                                        'source_run': ctx.s3_run, 'candidate_sha': ctx.args.candidate,
                                        'recorded_at': now}
    current['recorded_at'] = now
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(current, indent=1, sort_keys=True) + '\n')
    ctx.log('recorded', path)
    return path
