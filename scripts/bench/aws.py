"""Thin, stdlib-only wrapper over the AWS CLI v2 for the bench orchestrator.

Every call goes through `aws ... --output json` with the standard credential
chain (env, SSO profile, or an instance role). Transient errors (throttling,
5xx, connection resets) are retried with backoff; anything else raises
AwsError, which run_lane.py maps to exit code 3 (INFRA).
"""
import base64
import json
import os
from pathlib import Path
import subprocess
import time

TRANSIENT = ('Throttling', 'RequestLimitExceeded', 'ServiceUnavailable', 'InternalError',
             'Connection was closed', 'Could not connect', 'timed out', 'SlowDown', 'RequestTimeout',
             'InvalidInstanceId', 'InvalidInstanceID.NotFound', 'TooManyUpdates')


class AwsError(RuntimeError):
    pass


class Aws:
    def __init__(self, region, dry_run=False, log=print):
        self.region = region
        self.dry_run = dry_run
        self.log = log

    def call(self, *args, retries=6, parse=True, input_text=None, transient=TRANSIENT):
        argv = ['aws', *args, '--region', self.region, '--output', 'json']
        delay = 2.0
        for attempt in range(retries + 1):
            proc = subprocess.run(argv, capture_output=True, text=True, input=input_text)
            if proc.returncode == 0:
                if not parse:
                    return proc.stdout
                return json.loads(proc.stdout) if proc.stdout.strip() else {}
            err = (proc.stderr or proc.stdout).strip()
            if attempt < retries and any(t in err for t in transient):
                time.sleep(delay)
                delay = min(delay * 2, 30)
                continue
            raise AwsError('aws %s: %s' % (' '.join(args[:2]), err[-2000:]))
        raise AssertionError('unreachable')

    # ---- EC2
    def describe_instances(self, ids):
        out = self.call('ec2', 'describe-instances', '--instance-ids', *ids)
        return [i for r in out.get('Reservations', []) for i in r['Instances']]

    def template_data(self, template):
        return self.call('ec2', 'describe-launch-template-versions', '--launch-template-name', template,
                         '--versions', '$Default')['LaunchTemplateVersions'][0]['LaunchTemplateData']

    def placement_group_arn(self, name):
        return self.call('ec2', 'describe-placement-groups', '--group-names', name)['PlacementGroups'][0]['GroupArn']

    # ---- On-Demand Capacity Reservations (targeted: only launches that name them use them)
    def create_capacity_reservation(self, instance_type, az, count, end, tags, token, placement_group_arn=None):
        spec = [{'ResourceType': 'capacity-reservation', 'Tags': [{'Key': k, 'Value': v} for k, v in tags.items()]}]
        args = ['ec2', 'create-capacity-reservation', '--instance-type', instance_type,
                '--instance-platform', 'Linux/UNIX', '--availability-zone', az, '--instance-count', str(count),
                '--instance-match-criteria', 'targeted', '--end-date-type', 'limited', '--end-date', end,
                '--client-token', token, '--tag-specifications', json.dumps(spec)]
        if placement_group_arn:
            args += ['--placement-group-arn', placement_group_arn]
        return self.call(*args, retries=2)['CapacityReservation']['CapacityReservationId']

    def describe_capacity_reservations(self, ids=(), filters=()):
        args = ['ec2', 'describe-capacity-reservations']
        if ids:
            args += ['--capacity-reservation-ids', *ids]
        if filters:
            args += ['--filters', *('Name=%s,Values=%s' % (k, v) for k, v in filters)]
        return self.call(*args).get('CapacityReservations', [])

    def cancel_capacity_reservation(self, cid):
        self.call('ec2', 'cancel-capacity-reservation', '--capacity-reservation-id', cid)

    def wait_capacity_reservations(self, ids, timeout=300):
        deadline = time.time() + timeout
        while True:
            states = {r['CapacityReservationId']: r['State'] for r in self.describe_capacity_reservations(ids)}
            if all(states.get(i) == 'active' for i in ids):
                return
            if any(states.get(i) not in ('active', 'pending') for i in ids) or time.time() > deadline:
                raise AwsError('capacity reservations not active: %s' % states)
            time.sleep(5)

    def template_in_az(self, template, az):
        """run-instances overrides that move a launch template to `az`: its network interfaces with the
        subnet swapped for the same-named subnet of that AZ in the same VPC, and its placement (the
        cluster placement group takes the AZ of its first instance while empty)."""
        data = self.template_data(template)
        enis = [dict(e) for e in data.get('NetworkInterfaces', [])]
        if not enis or not enis[0].get('SubnetId'):
            raise AwsError('launch template %s names no subnet to move to %s' % (template, az))
        home = self.call('ec2', 'describe-subnets', '--subnet-ids', enis[0]['SubnetId'])['Subnets'][0]
        name = next((t['Value'] for t in home.get('Tags', []) if t['Key'] == 'Name'), '')
        if home['AvailabilityZone'] not in name:
            raise AwsError('subnet %s (%r) does not name its AZ: cannot find its %s twin' % (home['SubnetId'], name, az))
        twin = name.replace(home['AvailabilityZone'], az)
        found = self.call('ec2', 'describe-subnets', '--filters', 'Name=vpc-id,Values=%s' % home['VpcId'],
                          'Name=availability-zone,Values=%s' % az, 'Name=tag:Name,Values=%s' % twin)['Subnets']
        if len(found) != 1:
            raise AwsError('expected one subnet %r in %s, found %d' % (twin, az, len(found)))
        for e in enis:
            e['SubnetId'] = found[0]['SubnetId']
        placement = dict(data.get('Placement') or {}, AvailabilityZone=az)
        return {'network_interfaces': enis, 'placement': placement}

    def run_from_template(self, template, count, tags, ami=None, instance_type=None, user_data=None,
                          network_interfaces=None, placement=None, capacity_reservation=None):
        spec = [{'ResourceType': rt, 'Tags': [{'Key': k, 'Value': v} for k, v in tags.items()]}
                for rt in ('instance', 'volume')]
        args = ['ec2', 'run-instances', '--launch-template', 'LaunchTemplateName=%s,Version=$Default' % template,
                '--count', str(count), '--tag-specifications', json.dumps(spec)]
        if ami:
            args += ['--image-id', ami]
        if instance_type:
            args += ['--instance-type', instance_type]
        if network_interfaces:
            args += ['--network-interfaces', json.dumps(network_interfaces)]
        if placement:
            args += ['--placement', json.dumps(placement)]
        if capacity_reservation:
            args += ['--capacity-reservation-specification',
                     json.dumps({'CapacityReservationTarget': {'CapacityReservationId': capacity_reservation}})]
        if user_data:
            args += ['--user-data', base64.b64encode(user_data.encode()).decode()]
        out = self.call(*args, retries=2)
        return [i['InstanceId'] for i in out['Instances']]

    def terminate(self, ids):
        if ids:
            self.call('ec2', 'terminate-instances', '--instance-ids', *ids)

    def wait_running(self, ids, timeout=900):
        deadline = time.time() + timeout
        while time.time() < deadline:
            states = {i['InstanceId']: i['State']['Name'] for i in self.describe_instances(ids)}
            if all(s == 'running' for s in states.values()):
                return self.describe_instances(ids)
            if any(s in ('terminated', 'shutting-down', 'stopped') for s in states.values()):
                raise AwsError('instance left pending: %s' % states)
            time.sleep(10)
        raise AwsError('instances not running after %d s' % timeout)

    def wait_ssm_online(self, ids, timeout=900):
        deadline = time.time() + timeout
        while time.time() < deadline:
            out = self.call('ssm', 'describe-instance-information', '--filters',
                            'Key=InstanceIds,Values=%s' % ','.join(ids))
            online = {i['InstanceId'] for i in out.get('InstanceInformationList', []) if i.get('PingStatus') == 'Online'}
            if online >= set(ids):
                return
            time.sleep(10)
        raise AwsError('SSM agent not online on %s after %d s' % (ids, timeout))

    def vcpu_quota(self):
        out = self.call('service-quotas', 'get-service-quota', '--service-code', 'ec2', '--quota-code', 'L-1216C47A')
        return float(out['Quota']['Value'])

    def running_standard_vcpus(self):
        out = self.call('ec2', 'describe-instances', '--filters',
                        'Name=instance-state-name,Values=pending,running')
        total = 0
        for r in out.get('Reservations', []):
            for i in r['Instances']:
                family = i['InstanceType'][0]
                if family in 'acdhimrtz':
                    cpu = i.get('CpuOptions', {})
                    total += cpu.get('CoreCount', 1) * cpu.get('ThreadsPerCore', 1)
        return total

    # ---- SSM
    def send(self, ids, script, comment, timeout_s):
        """Run a bash script on instances; returns the command id."""
        # AWS-RunShellScript runs /bin/sh (dash on Ubuntu): run the script under bash.
        wrapped = ("f=$(mktemp /tmp/rvoip-bench-ssm-XXXXXX.sh)\n"
                   "cat > \"$f\" <<'RVOIP_BENCH_SSM_EOF'\n%s\nRVOIP_BENCH_SSM_EOF\n"
                   "bash \"$f\"; rc=$?; rm -f \"$f\"; exit $rc" % script)
        params = {'commands': [wrapped], 'executionTimeout': [str(int(timeout_s))]}
        out = self.call('ssm', 'send-command', '--instance-ids', *ids, '--document-name', 'AWS-RunShellScript',
                        '--comment', comment[:100], '--timeout-seconds', '600',
                        '--parameters', json.dumps(params))
        return out['Command']['CommandId']

    def wait_command(self, command_id, ids, timeout_s, poll=10):
        """Wait for every invocation; returns {instance: invocation dict}."""
        deadline = time.time() + timeout_s
        done = {}
        while time.time() < deadline and len(done) < len(ids):
            for iid in ids:
                if iid in done:
                    continue
                try:
                    inv = self.call('ssm', 'get-command-invocation', '--command-id', command_id,
                                    '--instance-id', iid, transient=TRANSIENT + ('InvocationDoesNotExist',))
                except AwsError as error:
                    if 'InvocationDoesNotExist' in str(error):
                        continue
                    raise
                if inv['Status'] not in ('Pending', 'InProgress', 'Delayed'):
                    done[iid] = inv
            if len(done) < len(ids):
                time.sleep(poll)
        if len(done) < len(ids):
            raise AwsError('SSM command %s still running after %d s on %s' % (
                command_id, timeout_s, sorted(set(ids) - set(done))))
        return done

    def run(self, ids, script, comment, timeout_s):
        """send + wait; raises AwsError on any non-success, with the tail of stderr."""
        cid = self.send(ids, script, comment, timeout_s)
        results = self.wait_command(cid, ids, timeout_s + 120)
        bad = {i: r for i, r in results.items() if r['Status'] != 'Success'}
        if bad:
            detail = '; '.join('%s %s rc=%s: %s' % (i, r['Status'], r.get('ResponseCode'),
                               (r.get('StandardErrorContent') or r.get('StandardOutputContent') or '')[-600:])
                               for i, r in bad.items())
            raise AwsError('%s failed: %s' % (comment, detail))
        return results

    # ---- S3
    def s3_cp(self, src, dst, recursive=False):
        args = ['s3', 'cp', '--only-show-errors', str(src), str(dst)]
        if recursive:
            args.insert(2, '--recursive')
        self.call(*args, parse=False)

    def s3_put_json(self, uri, value):
        tmp = Path(os.environ.get('TMPDIR', '/tmp')) / ('bench-put-%d.json' % os.getpid())
        tmp.write_text(json.dumps(value, sort_keys=True, indent=1) + '\n')
        try:
            self.s3_cp(tmp, uri)
        finally:
            tmp.unlink(missing_ok=True)

    def s3_get_json(self, uri):
        bucket, key = split_s3(uri)
        try:
            out = self.call('s3', 'cp', '--only-show-errors', uri, '-', parse=False, retries=3)
        except AwsError as error:
            if '404' in str(error) or 'Not Found' in str(error) or 'does not exist' in str(error):
                return None
            raise
        return json.loads(out)

    def s3_exists(self, uri):
        bucket, key = split_s3(uri)
        try:
            self.call('s3api', 'head-object', '--bucket', bucket, '--key', key, retries=3)
            return True
        except AwsError as error:
            if '404' in str(error) or 'Not Found' in str(error):
                return False
            raise

    def s3_create_exclusive(self, uri, value):
        """Conditional put (If-None-Match: *); False when the object already exists."""
        bucket, key = split_s3(uri)
        tmp = Path(os.environ.get('TMPDIR', '/tmp')) / ('bench-lock-%d.json' % os.getpid())
        tmp.write_text(json.dumps(value, sort_keys=True))
        try:
            self.call('s3api', 'put-object', '--bucket', bucket, '--key', key, '--body', str(tmp),
                      '--if-none-match', '*', retries=3)
            return True
        except AwsError as error:
            if 'PreconditionFailed' in str(error) or '412' in str(error) or 'ConditionalRequestConflict' in str(error):
                return False
            raise
        finally:
            tmp.unlink(missing_ok=True)

    def s3_sync(self, src, dst, include_patterns):
        args = ['s3', 'sync', '--only-show-errors', str(src), str(dst), '--exclude', '*']
        for pattern in include_patterns:
            args += ['--include', pattern]
        self.call(*args, parse=False)

    def s3_rm(self, uri):
        self.call('s3', 'rm', '--only-show-errors', uri, parse=False)

    def s3_list(self, prefix_uri):
        bucket, key = split_s3(prefix_uri)
        out = self.call('s3api', 'list-objects-v2', '--bucket', bucket, '--prefix', key)
        return [o['Key'] for o in out.get('Contents', [])]


def split_s3(uri):
    assert uri.startswith('s3://'), uri
    bucket, _, key = uri[5:].partition('/')
    return bucket, key
