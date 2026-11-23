#!/usr/bin/env python3
"""Re-score a finished DUT search run from its stored evidence, and optionally record its R values.

  rescore_run.py --receipt <minor/full receipt.json> [--rows R1,R5,R2,R6] [--record] [--bucket <evidence bucket>]

Each trial's merged summary.json (in the run's S3 evidence) is scored again with the current rules
(rfc7502.trial_verdict, and score.overload_verdict for R4), so a scoring fix applies to a run without
re-running it. For every row it reports the trial outcomes and its R: the highest rate that passed with no
failing trial at or below it (RFC 7502 §4.10: zero Session Attempt Failures), how many times R passed
(`confirmations`, against the profile's `confirmations_required`), and the lowest failing rate above it.

--record merges those rows into perf-baselines/rfc7502/<env-id>/rates.json `rows` (each naming this run as
source_run, its candidate_sha, and that it was re-scored), and copies the file beside the run's evidence
under record/rescore/. A row whose R did not reach the required confirmations is recorded with the count it
has, never as a full confirmation. It refuses a receipt of a profile that does not search the DUT, an env-id
other than ENVIRONMENT, and a run without a final verdict. Exit codes: 0 done, 2 refused.
"""
import argparse
import datetime
import json
import os
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import profiles  # noqa: E402
import rfc7502  # noqa: E402
import run_lane  # noqa: E402
import score  # noqa: E402

SEARCH_PROFILES = ('minor', 'minor-lite', 'full')
RULE = ('R = the highest rate with a passing trial and no failing trial at or below it; trials re-scored with '
        'the current rfc7502.trial_verdict (a DUT setup-delay collapse is a DUT fail, not tester-invalid)')


class Refused(RuntimeError):
    pass


def rel_of(evidence, s3_run):
    return evidence[len(s3_run):] if evidence.startswith(s3_run) else evidence


def row_result(row_id, trials, required):
    """R, confirmations and bracket from re-scored (rate, outcome) search trials."""
    passes = {}
    fails = set()
    for rate, outcome in trials:
        if outcome == 'pass':
            passes[rate] = passes.get(rate, 0) + 1
        elif outcome == 'fail':
            fails.add(rate)
    clean = [rate for rate in passes if not any(f <= rate for f in fails)]
    r = max(clean) if clean else None
    above = sorted(f for f in fails if r is None or f > r)
    return {'id': row_id, 'r': r, 'confirmations': passes.get(r, 0) if r is not None else 0,
            'confirmations_required': required, 'lowest_failing_sps': above[0] if above else None,
            'trials': [{'rate': rate, 'outcome': outcome} for rate, outcome in trials]}


def rescore(receipt, fetch_summary, fetch_plan, rows=None):
    """Re-scored results per row of `receipt` (fetch_* take a trial's evidence prefix). Pure: tested offline."""
    profile = profiles.PROFILES[receipt['profile']]
    required = profile.get('confirmations', 1)
    out = {}
    for row in receipt['rows']:
        if rows and row['id'] not in rows:
            continue
        if row['id'] == 'R4':
            trial = (row.get('trials') or [None])[0]
            if trial:
                plan = fetch_plan(trial['evidence'])
                verdict = score.overload_verdict(fetch_summary(trial['evidence']), plan['rate'], plan['profile'])
                out['R4'] = {'id': 'R4', 'verdict': verdict['verdict'], 'reason': verdict['reason'],
                             'previous_verdict': row.get('verdict')}
            continue
        scored = []
        for trial in row.get('trials') or []:
            if trial.get('kind', 'search') not in ('search', 'confirm', None):
                continue
            verdict = rfc7502.trial_verdict(fetch_summary(trial['evidence']), 'rfc')
            scored.append((trial['rate'], verdict['outcome']))
        result = row_result(row['id'], scored, required)
        result['previous_verdict'] = row.get('verdict')
        latency = row.get('latency') or {}
        if result['r'] and latency.get('srd_ms') and latency.get('rate') == profiles.quantize(result['r'] * 0.9):
            result.update(srd_ms=latency['srd_ms'], sdd_ms=latency.get('sdd_ms'), latency_rate=latency['rate'])
        out[row['id']] = result
    return out


def record(results, receipt, rates_path):
    """Merge the re-scored rows (with an R) into rates.json `rows`."""
    rates = json.loads(rates_path.read_text())
    now = datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')
    caps = rates.get('tester_caps') or {}
    for row_id, res in sorted(results.items()):
        if row_id == 'R4' or res.get('r') is None:
            continue
        cap = caps.get(row_id)
        rates.setdefault('rows', {})[row_id] = {
            'r': res['r'], 'tester_limited': bool(cap and res['r'] >= cap), 'tester_cap': cap,
            'tester_cap_lower_bound': bool(((rates.get('baseline_sps') or {}).get(row_id) or {}).get('lower_bound')),
            'srd_ms': res.get('srd_ms'), 'sdd_ms': res.get('sdd_ms'), 'latency_rate': res.get('latency_rate'),
            'confirmations': res['confirmations'], 'confirmations_required': res['confirmations_required'],
            'lowest_failing_sps': res['lowest_failing_sps'],
            'source_run': receipt['evidence']['s3_prefix'], 'candidate_sha': receipt['candidate_sha'],
            'recorded_at': now, 'rescored': {'tool': 'scripts/bench/rescore_run.py', 'rule': RULE}}
    rates['recorded_at'] = now
    rates_path.write_text(json.dumps(rates, indent=1, sort_keys=True) + '\n')
    return rates


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument('--receipt', required=True)
    p.add_argument('--rows', help='comma list (default: every row)')
    p.add_argument('--record', action='store_true', help='merge the R values into the env-id\'s rates.json')
    p.add_argument('--bucket', default=os.environ.get('RVOIP_AWS_EVIDENCE_BUCKET'))
    p.add_argument('--region', default=os.environ.get('RVOIP_AWS_REGION', 'us-west-2'))
    a = p.parse_args(argv)
    try:
        receipt = json.loads(Path(a.receipt).read_text())
        if receipt.get('profile') not in SEARCH_PROFILES:
            raise Refused('a %r receipt: only a DUT search (%s) has R values to re-score'
                          % (receipt.get('profile'), ', '.join(SEARCH_PROFILES)))
        env_id = (receipt.get('environment') or {}).get('env_id')
        pin = run_lane.BASELINES / 'ENVIRONMENT'
        current = pin.read_text().strip() if pin.exists() else None
        if not env_id or env_id != current:
            raise Refused('the receipt is for env-id %r but ENVIRONMENT is %r' % (env_id, current))
        rates_path = run_lane.BASELINES / env_id / 'rates.json'
        if a.record and not rates_path.is_file():
            raise Refused('no rates.json for %s (its §6.1 baseline records it first)' % env_id)
        if receipt.get('status') not in ('PASS', 'WARN', 'FAIL'):
            raise Refused('run %s never finished (status %s)' % (receipt.get('run_id'), receipt.get('status')))
        s3_run = receipt['evidence']['s3_prefix']
        aws = run_lane.Aws(a.region, log=lambda *x: None)
        state = aws.s3_get_json(s3_run + 'state.json')
        if not (state or {}).get('final'):
            raise Refused('run %s has no final verdict in %sstate.json' % (receipt.get('run_id'), s3_run))
    except Refused as error:
        print('rescore_run: refused: %s' % error, file=sys.stderr)
        return 2

    def fetch(prefix, name):
        data = aws.s3_get_json(prefix + name)
        if data is None:
            raise SystemExit('rescore_run: %s%s is missing' % (prefix, name))
        return data

    results = rescore(receipt, lambda prefix: fetch(prefix, 'merged/summary.json'),
                      lambda prefix: fetch(prefix, 'plan.json'), a.rows.split(',') if a.rows else None)
    print(json.dumps({'run': s3_run, 'rule': RULE, 'rows': results}, indent=1, sort_keys=True))
    if a.record:
        record(results, receipt, rates_path)
        aws.s3_cp(rates_path, s3_run + 'record/rescore/rates.json')
        print('recorded', rates_path, file=sys.stderr)
    return 0


if __name__ == '__main__':
    sys.exit(main())
