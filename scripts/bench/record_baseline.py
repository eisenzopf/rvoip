#!/usr/bin/env python3
"""Record a finished §6.1 testbed baseline into perf-baselines/rfc7502/<env-id>/rates.json without re-running it.

  record_baseline.py --receipt <baseline receipt.json> [--rows R1,R5,R2] [--bucket <evidence bucket>]

A baseline run records its rates only when the whole run passes, so one row that ran out of trials could drop
every other row's result. This applies run_lane's own baseline row rules to the run's receipt and writes the
rows with a value through score.record_rates (rates.json beside the env-id's env.json, merged with what it holds,
plus the S3 copy under the run's record/):

- a row with `r`: as run_lane scored it (tester_limited when it stopped on caller-invalid trials);
- a row whose search stopped on max_trials or max_hours with every trial passing: its highest passing rate,
  flagged `lower_bound` (the tester's ceiling was never observed).

It refuses a receipt of any profile but `baseline`, an env-id other than perf-baselines/rfc7502/ENVIRONMENT,
and a run without a final verdict (status INFRA, or no `final` in the run's S3 state.json).
Exit codes: 0 recorded, 2 refused.
"""
import argparse
import json
import os
from pathlib import Path
import sys
import types

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import run_lane  # noqa: E402
import score  # noqa: E402


class Refused(RuntimeError):
    pass


def rows_to_record(receipt, wanted):
    """The receipt rows to record (copies), with run_lane's baseline rules applied."""
    out = []
    for row in receipt.get('rows', []):
        if row.get('id') == 'R4' or (wanted and row['id'] not in wanted):
            continue
        row = dict(row)
        if row.get('r') is None:
            bound = run_lane.baseline_lower_bound(row.get('search'))
            if not bound:
                continue
            row.update(r=bound, tester_limited=False, lower_bound=True,
                       lower_bound_reason=(row.get('search') or {}).get('stop_reason'))
        out.append(row)
    missing = sorted(set(wanted or ()) - {r['id'] for r in out})
    if missing:
        raise Refused('no recordable result for %s in %s' % (', '.join(missing), receipt.get('run_id')))
    return out


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument('--receipt', required=True)
    p.add_argument('--rows', help='comma list (default: every recordable row)')
    p.add_argument('--bucket', default=os.environ.get('RVOIP_AWS_EVIDENCE_BUCKET'))
    p.add_argument('--region', default=os.environ.get('RVOIP_AWS_REGION', 'us-west-2'))
    a = p.parse_args(argv)
    try:
        receipt = json.loads(Path(a.receipt).read_text())
        if receipt.get('profile') != 'baseline':
            raise Refused('a %r receipt: only a baseline run records tester caps' % receipt.get('profile'))
        env_id = (receipt.get('environment') or {}).get('env_id')
        current = (run_lane.BASELINES / 'ENVIRONMENT')
        current = current.read_text().strip() if current.exists() else None
        if not env_id or env_id != current:
            raise Refused('the receipt is for env-id %r but ENVIRONMENT is %r' % (env_id, current))
        if not (run_lane.BASELINES / env_id / 'env.json').exists():
            raise Refused('no env.json for %s: pin the environment first' % env_id)
        if receipt.get('status') not in ('PASS', 'WARN', 'FAIL'):
            raise Refused('run %s never finished (status %s)' % (receipt.get('run_id'), receipt.get('status')))
        s3_run = (receipt.get('evidence') or {}).get('s3_prefix')
        aws = run_lane.Aws(a.region, log=print)
        state = aws.s3_get_json(s3_run + 'state.json') if s3_run else None
        if not (state or {}).get('final'):
            raise Refused('run %s has no final verdict in %sstate.json' % (receipt.get('run_id'), s3_run))
        rows = rows_to_record(receipt, a.rows.split(',') if a.rows else None)
        if not rows:
            raise Refused('no recordable row in %s' % receipt.get('run_id'))
    except Refused as error:
        print('record_baseline: refused: %s' % error, file=sys.stderr)
        return 2
    ctx = types.SimpleNamespace(s3_run=s3_run, aws=aws, log=print,
                                args=types.SimpleNamespace(candidate=receipt.get('candidate_sha')))
    path = score.record_rates(rows, env_id, run_lane.BASELINES, ctx, 'baseline')
    print(json.dumps({'recorded': str(path), 'env_id': env_id, 'source_run': s3_run,
                      'rows': {r['id']: {'r': r['r'], 'tester_limited': bool(r.get('tester_limited')),
                                         'lower_bound': bool(r.get('lower_bound'))} for r in rows}}))
    return 0


if __name__ == '__main__':
    sys.exit(main())
