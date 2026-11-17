#!/usr/bin/env python3
"""Bake the rvoip bench AMI: Ubuntu 24.04 + bench_tune.py (E74 host tuning).

  bake.py --region us-west-2 [--template rvoip-bench-dut] [--bucket <evidence bucket>] [--keep]

Launches one builder from the DUT launch template with the stock Ubuntu AMI
(the templates fall back to it until bench_ami_id is set), uploads this
directory to s3://<bucket>/bench/ami/<stamp>/, runs provision.sh over SSM,
creates the image (reboot), tags it rvoip-bench=true, and terminates the
builder. Prints the AMI id; put it in infra/aws/bench as bench_ami_id and
re-apply. Cost: one c8i.4xlarge for about 15 minutes.
"""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import sys
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from aws import Aws, AwsError  # noqa: E402

UBUNTU = '/aws/service/canonical/ubuntu/server/24.04/stable/current/amd64/hvm/ebs-gp3/ami-id'
FILES = ('bench_tune.py', 'rvoip-bench-tune.service', 'provision.sh')


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument('--region', default='us-west-2')
    p.add_argument('--template', default='rvoip-bench-dut')
    p.add_argument('--bucket', required=True)
    p.add_argument('--keep', action='store_true', help='leave the builder running (debug)')
    a = p.parse_args()
    aws = Aws(a.region)
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ')
    digest = hashlib.sha256(b''.join((HERE / f).read_bytes() for f in FILES)).hexdigest()[:12]
    prefix = 's3://%s/bench/ami/%s/' % (a.bucket, stamp)
    for name in FILES:
        aws.s3_cp(HERE / name, prefix + name)
    ubuntu = aws.call('ssm', 'get-parameter', '--name', UBUNTU)['Parameter']['Value']
    expires = (datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(hours=2)).strftime('%Y-%m-%dT%H:%M:%SZ')
    tags = {'Name': 'rvoip-bench-ami-builder', 'rvoip-bench': 'true', 'managed-by': 'github-actions',
            'rvoip-expires-at': expires, 'rvoip-role': 'builder', 'rvoip-run-id': 'ami-' + stamp}
    ids = aws.run_from_template(a.template, 1, tags, ami=ubuntu)
    print('builder', ids[0], 'from', ubuntu, flush=True)
    try:
        aws.wait_running(ids)
        aws.wait_ssm_online(ids)
        script = '\n'.join([
            'set -euo pipefail', 'export PATH=$PATH:/snap/bin', 'd=$(mktemp -d); cd $d',
            # The stock image has no AWS CLI yet; fetch the three files with the snap CLI first.
            'snap list aws-cli >/dev/null 2>&1 || snap install aws-cli --classic',
            *['aws s3 cp --only-show-errors %s%s .' % (prefix, f) for f in FILES],
            'bash provision.sh'])
        out = aws.run(ids, script, 'rvoip bench ami provision', 2400)
        print(out[ids[0]]['StandardOutputContent'][-1500:], flush=True)
        name = 'rvoip-bench-%s-%s' % (stamp, digest)
        image = aws.call('ec2', 'create-image', '--instance-id', ids[0], '--name', name,
                         '--description', 'rvoip bench lane host (Ubuntu 24.04 + E74 tuning, scripts/bench/ami %s)' % digest,
                         '--tag-specifications', json.dumps([
                             {'ResourceType': 'image', 'Tags': [{'Key': 'rvoip-bench', 'Value': 'true'},
                                                                {'Key': 'Name', 'Value': name},
                                                                {'Key': 'rvoip-bench-ami-digest', 'Value': digest}]}]))['ImageId']
        print('image', image, name, flush=True)
        deadline = time.time() + 3600
        while time.time() < deadline:
            state = aws.call('ec2', 'describe-images', '--image-ids', image)['Images'][0]['State']
            if state == 'available':
                break
            if state == 'failed':
                raise AwsError('image %s failed' % image)
            time.sleep(20)
        print(json.dumps({'ami_id': image, 'name': name, 'digest': digest, 'source_ami': ubuntu}))
    finally:
        if not a.keep:
            aws.terminate(ids)
            print('terminated', ids[0], flush=True)


if __name__ == '__main__':
    sys.exit(main())
