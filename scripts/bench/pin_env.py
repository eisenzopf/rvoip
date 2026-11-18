#!/usr/bin/env python3
"""Pin a bench environment: the tester builds and the AMI an env-id stands for.

  pin_env.py --caller-bin <rvoip-loadgen> [--ea-uas-bin <ea-uas>] --ami <ami-id>
             [--bucket <evidence bucket>] [--region us-west-2] [--callers 2]
             [--dut-type c8i.4xlarge] [--caller-type c8in.24xlarge] [--az us-west-2c]
             [--media-worker-threads 24] [--media-lanes 4] [--set-current]

Uploads each binary to s3://<bucket>/bench/builds/<name>-<sha256[:16]> (immutable
by name), computes the env-id the lane would compute, and writes
crates/sip/rvoip-sip/perf-baselines/rfc7502/<env-id>/env.json. With
--set-current it also points perf-baselines/rfc7502/ENVIRONMENT at it.
run_lane.py then fetches the pinned builds when --caller-bin/--ea-uas-bin are
omitted (the release controller builds only the candidate's DUT).

A new env-id needs a `baseline` run and a `minor --record` run before the
`release` profile can gate on it.
"""
import argparse
import datetime
import json
import os
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from aws import Aws  # noqa: E402
import run_lane  # noqa: E402


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument('--caller-bin', required=True)
    p.add_argument('--ea-uas-bin')
    p.add_argument('--ami', required=True)
    p.add_argument('--bucket', default=os.environ.get('RVOIP_AWS_EVIDENCE_BUCKET'))
    p.add_argument('--region', default='us-west-2')
    p.add_argument('--callers', type=int, default=2)
    p.add_argument('--dut-type', default='c8i.4xlarge')
    p.add_argument('--caller-type', default='c8in.24xlarge')
    p.add_argument('--caller-source', help='where the caller build came from (branch@sha)')
    p.add_argument('--media-worker-threads', type=int, default=24,
                   help='rvoip media executor threads per caller host (0: rvoip default, 2-8 workers)')
    p.add_argument('--media-lanes', type=int, default=4, help='media lanes splitting those threads')
    p.add_argument('--az', help='the AZ every run on this env-id launches in (run_lane --az); '
                                'pin it before the baseline so the baseline and the gated runs share it')
    p.add_argument('--set-current', action='store_true')
    a = p.parse_args()
    assert a.bucket, '--bucket or RVOIP_AWS_EVIDENCE_BUCKET'
    aws = Aws(a.region)
    builds = {}
    for name, path in (('rvoip-loadgen', a.caller_bin), ('ea-uas', a.ea_uas_bin)):
        if not path:
            continue
        sha = run_lane.sha256_file(path)
        uri = 's3://%s/bench/builds/%s-%s' % (a.bucket, name, sha[:16])
        aws.s3_cp(path, uri)
        builds[name] = {'sha256': sha, 's3_uri': uri}
    lane = {'region': a.region, 'ami_id': a.ami, 'callers': [None] * a.callers,
            'tf': {'dut_instance_type': a.dut_type, 'caller_instance_type': a.caller_type}}
    env_id = run_lane.env_id_for(lane, builds['rvoip-loadgen']['sha256'])
    env = {'schema': 'rvoip-rfc7502-env-v1', 'env_id': env_id, 'region': a.region, 'ami_id': a.ami,
           'dut_instance_type': a.dut_type, 'caller_instance_type': a.caller_type, 'caller_count': a.callers,
           'builds': builds, 'caller_source': a.caller_source, 'az': a.az,
           'caller_media': ({'worker_threads': a.media_worker_threads, 'lanes': a.media_lanes}
                            if a.media_worker_threads else None),
           'pinned_at': datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')}
    out = run_lane.BASELINES / env_id / 'env.json'
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(env, indent=1, sort_keys=True) + '\n')
    if a.set_current:
        (run_lane.BASELINES / 'ENVIRONMENT').write_text(env_id + '\n')
    print(json.dumps({'env_id': env_id, 'env_json': str(out)}))


if __name__ == '__main__':
    sys.exit(main())
