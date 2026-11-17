#!/usr/bin/env python3
"""One RFC 7502 trial on loopback: the evidence contract end to end, no AWS.

  loopback_trial.py --dut-bin <rvoip-bench-uas> --caller-bin <rvoip-loadgen> --row R1|R2|R5|R6
                    [--rate SPS] [--seconds S] [--warmup S] [--admission-max-cps N] [--out DIR]

A local stand-in for one run_lane `Trials.run`, built from run_lane's own
plan and config builders on a fake lane: the DUT on 127.0.0.1, caller host 1
sourcing from 127.0.0.2/.3 and caller host 2 from 127.0.0.4/.5 (Linux routes
all of 127/8 locally), a throwaway CA from `run_lane.make_tls`. Every host
shares this machine, so only ports are adjusted: the DUT's RTP range is
10000-29999, caller 1 uses 30000-44999, caller 2 45000-59999, and the metrics
listeners are 9464 (DUT), 9465 and 9466 (callers).

It writes plan.json, starts the DUT (+ host_sampler), waits for ready.json,
starts both callers at one instant (+ host_sampler each), waits for them,
touches `finish`, waits for the DUT to drain and lays the files out as
bench-agent.sh's tarballs extract (`<trial>/dut/`, `<trial>/caller-<i>/`).
Then it runs summarize_trial.write(criterion='rfc') and prints
rfc7502.trial_verdict and search.halt_reasons. Exit 0 when the outcome is
the one expected (`--expect`, default pass) and nothing halts.

This is a correctness check of the evidence pipeline, not a capacity
measurement: the DUT and both callers share the host's CPUs.
"""
import argparse
import datetime
import json
import os
from pathlib import Path
import resource
import subprocess
import sys
import time
import urllib.request

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import evidence  # noqa: E402
import profiles  # noqa: E402
import rfc7502  # noqa: E402
import run_lane  # noqa: E402
import search  # noqa: E402
import summarize_trial  # noqa: E402

DUT_IP = '127.0.0.1'
DUT_RTP_PORTS = [10000, 29999]
DUT_METRICS_PORT = 9464
CALLERS = [
    {'ips': ['127.0.0.2', '127.0.0.3'], 'media_ports': (30000, 44999), 'metrics_port': 9465},
    {'ips': ['127.0.0.4', '127.0.0.5'], 'media_ports': (45000, 59999), 'metrics_port': 9466},
]
# A loopback caller shares the host with the DUT and the other caller: a small explicit
# executor budget (caller_policy.validate's bounds) instead of a caller-host policy.
LOOPBACK_CALLER_ENV = {'RVOIP_LOADGEN_SIGNALING_THREADS': '2', 'RVOIP_LOADGEN_LANES': '2',
                       'RVOIP_LOADGEN_WORKERS_PER_LANE': '2', 'RVOIP_MEDIA_WORKER_THREADS': '2',
                       'RVOIP_MEDIA_RUNTIME_LANES': '1'}


def log(*args):
    print('%s %s' % (datetime.datetime.now().strftime('%H:%M:%S'), ' '.join(str(a) for a in args)), flush=True)


class FakeLane:
    """run_lane.Lane's `info` for one loopback host."""

    def __init__(self):
        cores = max(1, (os.cpu_count() or 2) // 2)
        self.info = {'dut': {'id': 'loopback-dut', 'ip': DUT_IP},
                     'callers': [{'id': 'loopback-caller-%d' % (i + 1), 'ips': list(c['ips']),
                                  'logical_cpus': os.cpu_count(), 'physical_cores': cores}
                                 for i, c in enumerate(CALLERS)]}
        self.ea_uas = False


class FakeArgs:
    def __init__(self, run_id):
        self.run_id = run_id


def raise_nofile():
    soft, hard = resource.getrlimit(resource.RLIMIT_NOFILE)
    resource.setrlimit(resource.RLIMIT_NOFILE, (hard, hard))


def sampler(pid, out):
    return subprocess.Popen([sys.executable, str(HERE / 'host_sampler.py'), '--pid', str(pid), '--out', str(out)],
                            stdout=subprocess.DEVNULL, stderr=open(out.parent / 'sampler.log', 'w'),
                            start_new_session=True)


def scrape(port, out):
    try:
        with urllib.request.urlopen('http://127.0.0.1:%d/metrics' % port, timeout=10) as response:
            out.write_bytes(response.read())
    except OSError as error:
        log('WARNING metrics scrape on', port, error)


def tls_peers(port):
    """Established TCP connections to the DUT's TLS port, counted by peer IP (`ss -tn`)."""
    try:
        out = subprocess.run(['ss', '-Htn', 'state', 'established', '( sport = :%d )' % port],
                             capture_output=True, text=True, timeout=10).stdout
    except (OSError, subprocess.TimeoutExpired) as error:
        return {'error': str(error)}
    peers = {}
    for line in out.splitlines():
        fields = line.split()
        if len(fields) >= 4:
            ip = fields[-1].rsplit(':', 1)[0]
            peers[ip] = peers.get(ip, 0) + 1
    return peers


def wait_exit(proc, seconds):
    try:
        return proc.wait(timeout=seconds)
    except subprocess.TimeoutExpired:
        return None


def build(args, trial):
    """plan.json and the per-host configs, from run_lane's builders with loopback paths and ports."""
    row = profiles.ROWS[args.row]
    hosts = len(CALLERS)
    profile = rfc7502.trial_profile(args.rate, n=args.rate * args.seconds, warmup_seconds=args.warmup,
                                    minimum_seconds=args.seconds, hosts=hosts)
    admission = ({'mode': 'max-cps', 'max_cps': args.admission_max_cps, 'retry_after_secs': 2}
                 if args.admission_max_cps else None)
    trials = run_lane.Trials(FakeArgs(args.run_id), None, FakeLane(), log, '')
    plan = trials.plan(args.row, row, args.rate, 0, profile, args.rel, admission)
    tls = args.out / 'tls'
    dut = plan['dut_config']
    dut.update(run_dir=str(trial / 'dut'), tls_cert=str(tls / 'dut.crt'), tls_key=str(tls / 'dut.key'),
               rtp_port_range=DUT_RTP_PORTS, metrics_listen='%s:%d' % (DUT_IP, DUT_METRICS_PORT))
    for i, (config, host) in enumerate(zip(plan['caller_configs'], CALLERS)):
        config.update(output=str(trial / ('caller-%d' % (i + 1))), tls_cert_path=str(tls / 'caller.crt'),
                      tls_key_path=str(tls / 'caller.key'), tls_ca_path=str(tls / 'ca.crt'),
                      media_port_start=host['media_ports'][0], media_port_end=host['media_ports'][1],
                      metrics_listen='%s:%d' % (host['ips'][0], host['metrics_port']))
        if args.linger is not None:
            config['linger_seconds'] = args.linger
    plan['loopback'] = True
    return plan


def run(args):
    trial = args.out / args.rel
    if trial.exists():
        raise SystemExit('%s exists; pick another --out' % trial)
    trial.mkdir(parents=True)
    tls = args.out / 'tls'
    if not (tls / 'dut.crt').exists():
        run_lane.make_tls(tls, [DUT_IP], [ip for c in CALLERS for ip in c['ips']])
    plan = build(args, trial)
    (trial / 'plan.json').write_text(json.dumps(plan, indent=1, sort_keys=True))
    log('trial', args.rel, 'stages', [(s['name'], s['cps'], s['seconds']) for s in plan['profile']['stages']])

    # 1. Fresh DUT process (bench-agent.sh dut-start).
    run_dir = trial / 'dut'
    run_dir.mkdir()
    (run_dir / 'config.json').write_text(json.dumps(plan['dut_config'], indent=1))
    dut = subprocess.Popen([str(args.dut_bin), '--config', str(run_dir / 'config.json')], cwd=run_dir,
                           stdin=subprocess.DEVNULL, stdout=open(run_dir / 'dut.log', 'w'), stderr=subprocess.STDOUT,
                           preexec_fn=raise_nofile, start_new_session=True)
    (run_dir / 'pid').write_text('%d\n' % dut.pid)
    samplers = [sampler(dut.pid, run_dir / 'resources.jsonl')]
    deadline = time.time() + 60
    while not (run_dir / 'ready.json').exists() or not (run_dir / 'ready.json').stat().st_size:
        if dut.poll() is not None or time.time() > deadline:
            raise SystemExit('DUT not ready (rc %s); see %s' % (dut.returncode, run_dir / 'dut.log'))
        time.sleep(0.2)
    log('DUT ready', (run_dir / 'ready.json').read_text().strip())

    # 2. Callers from one start instant (the S3 barrier's go.json start_at).
    procs = []
    env = dict(os.environ, **LOOPBACK_CALLER_ENV)
    start_at = time.time() + 2.0
    for i, config in enumerate(plan['caller_configs']):
        out = trial / ('caller-%d' % (i + 1))
        out.mkdir()
        (out / 'caller.json').write_text(json.dumps(config, indent=1))
        (out / 'caller-env.sh').write_text(''.join('export %s=%s\n' % kv for kv in sorted(LOOPBACK_CALLER_ENV.items())))
        (out / 'go.json').write_text(json.dumps({'start_at': start_at, 'callers': len(CALLERS)}))
    time.sleep(max(0.0, start_at - time.time()))
    for i in range(len(CALLERS)):
        out = trial / ('caller-%d' % (i + 1))
        (out / 'start.json').write_text(json.dumps({'started': time.time()}))
        procs.append(subprocess.Popen([str(args.caller_bin), str(out / 'caller.json')], cwd=out, env=env,
                                      stdin=subprocess.DEVNULL, stdout=open(out / 'caller.log', 'w'),
                                      stderr=subprocess.STDOUT, preexec_fn=raise_nofile, start_new_session=True))
    for i, proc in enumerate(procs):
        samplers.append(sampler(proc.pid, trial / ('caller-%d' % (i + 1)) / 'resources.jsonl'))
    total = sum(s['seconds'] for s in plan['profile']['stages'])
    log('callers started; generating for %d s' % total)

    # 3. Generate: the callers exit after their stages, the hold and the linger.
    budget = total + profiles.ROWS[args.row]['hold_ms'] / 1000 + 600
    if plan['transport'] == 'tls':
        # Mid-run: the TLS connections on the DUT's port by peer address (one per caller endpoint,
        # each from its own source IP).
        time.sleep(max(0.0, start_at + total / 2.0 - time.time()))
        peers = tls_peers(int(plan['dut_config']['bind'].rsplit(':', 1)[1]))
        (trial / 'loopback-tls-peers.json').write_text(json.dumps(peers, indent=1, sort_keys=True))
        log('DUT TLS connections by source IP:', peers)
    for i, proc in enumerate(procs):
        rc = wait_exit(proc, max(1, start_at + budget - time.time()))
        if rc is None:
            proc.kill()
            rc = proc.wait()
        out = trial / ('caller-%d' % (i + 1))
        (out / 'exit.json').write_text(json.dumps({'code': rc, 'ended': time.time()}))
        log('caller-%d exit %s' % (i + 1, rc))

    # 4. Drain the DUT (bench-agent.sh dut-finish).
    scrape(DUT_METRICS_PORT, run_dir / 'metrics-before-drain.prom')
    (run_dir / 'finish').touch()
    rc = wait_exit(dut, 300)
    killed = rc is None
    if killed:
        dut.kill()
        rc = dut.wait()
    (run_dir / 'exit.json').write_text(json.dumps({'killed': killed, 'code': rc}))
    log('DUT exit', rc, '(killed)' if killed else '')
    for proc in samplers:
        if wait_exit(proc, 10) is None:
            proc.terminate()

    # 5. Summarize (bench-agent.sh summarize) and score (Trials.verdict).
    started = time.time()
    before = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    path, summary = summarize_trial.write(trial, criterion='rfc')
    cost = {'seconds': round(time.time() - started, 2),
            'peak_rss_mib': round(resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024, 1),
            'rss_before_mib': round(before / 1024, 1)}
    verdict = rfc7502.trial_verdict(summary, 'rfc')
    verdict['halt'] = search.halt_reasons(summary)
    verdict['run'] = args.rel
    (trial / 'verdict.json').write_text(json.dumps(verdict, indent=1, sort_keys=True))
    report(summary, verdict, cost, path)
    ok = verdict['outcome'] == args.expect and not verdict['halt']
    return 0 if ok else 1


def report(summary, verdict, cost, path):
    checks = summary.get('checks') or {}
    scored = (summary.get('rfc6076') or {}).get('scored') or {}
    srd = scored.get('srd_successful_ms') or {}
    sdd = scored.get('sdd_ms') or {}
    media = summary.get('media') or {}
    print(json.dumps({
        'summary': str(path), 'summarize': cost,
        'outcome': verdict['outcome'], 'attempted': verdict.get('attempted'), 'succeeded': verdict.get('succeeded'),
        'failed': verdict.get('failed'), 'reasons': verdict.get('reasons'), 'halt_reasons': verdict['halt'],
        'final_status_counts': (verdict.get('detail') or {}).get('final_status_counts'),
        'summary_verdict': summary.get('verdict'),
        'checks': checks, 'checks_missing': summary.get('checks_missing'),
        'srd_ms': {k: srd.get(k) for k in ('p50', 'p99', 'count')},
        'sdd_ms': {k: sdd.get(k) for k in ('p50', 'p99', 'count')},
        'media': {k: media.get(k) for k in ('applicable', 'established', 'validated_audio', 'audio_failures',
                                            'dut_rtp_packets_received', 'dut_rtp_packets_sent')} if media else None,
        'dut': {k: (summary.get('dut') or {}).get(k) for k in ('true_zero', 'clean_shutdown', 'drain_seconds',
                                                                'responses_503', 'admission_rejected')},
    }, indent=1, sort_keys=True, default=str))


def parse_args(argv):
    p = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    p.add_argument('--dut-bin', type=Path, required=True)
    p.add_argument('--caller-bin', type=Path, required=True)
    p.add_argument('--row', choices=sorted(profiles.ROWS), required=True)
    p.add_argument('--rate', type=int, default=400, help='scored-stage rate (sps), a multiple of 4 x 2 hosts')
    p.add_argument('--seconds', type=int, default=20, help='scored stage length')
    p.add_argument('--warmup', type=int, default=10, help='warm-up seconds at rate/2')
    p.add_argument('--admission-max-cps', type=int, help="DUT admission {'mode': 'max-cps', 'max_cps': N}")
    p.add_argument('--linger', type=int, help='override the callers\' linger_seconds')
    p.add_argument('--expect', choices=('pass', 'fail', 'invalid'), default='pass')
    p.add_argument('--out', type=Path, help='default: ./loopback-trials/<run id>')
    args = p.parse_args(argv)
    for name in ('dut_bin', 'caller_bin'):
        if not getattr(args, name).is_file():
            p.error('--%s: no such file' % name.replace('_', '-'))
    args.run_id = 'lb-%s' % run_lane.stamp()
    args.out = (args.out or Path.cwd() / 'loopback-trials' / args.run_id).resolve()
    args.rel = evidence.trial_path(args.row, 0, args.rate)
    return args


if __name__ == '__main__':
    sys.exit(run(parse_args(sys.argv[1:])))
