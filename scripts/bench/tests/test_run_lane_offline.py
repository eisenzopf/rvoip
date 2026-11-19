"""Offline tests of the bench lane orchestrator (run_lane.py): every profile driven end to end without AWS.

`FakeAws` replaces only `Aws.call` (the AWS CLI), so aws.py's own wrappers run unchanged against an
in-process cloud: EC2 instances, SSM commands answered by a fake bench-agent.sh, and a temp directory
standing in for the evidence bucket. The fake agent's `summarize` writes a real merged/summary.json:
synthetic caller and DUT evidence (bench_fixtures) for a scaled-down copy of the trial's plan.json
(every stage keeps its name; MINI_CPS stands for the trial rate), run through summarize_trial.

`World` is the simulated testbed: the DUT's true capacity per row (trial stages above it fail with
503s or timeouts), ea-uas's capacity for the baseline, the callers' own limit (trials above it are
caller-invalid: schedule-to-write p99 > 5 ms), the admission status under overload, and an SRD offset.
A fake clock (time.time) advances by each trial's stages, so --max-wall is exercised too.
"""
import contextlib
import fnmatch
import hashlib
import io
import json
from pathlib import Path
import re
import shlex
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

HERE = Path(__file__).resolve().parent
BENCH = HERE.parent
sys.path.insert(0, str(BENCH))
sys.path.insert(0, str(HERE))
import aws as aws_mod  # noqa: E402
import bench_fixtures as fx  # noqa: E402
import profiles  # noqa: E402
import record_baseline  # noqa: E402
import rfc_report  # noqa: E402
import run_lane  # noqa: E402
import summarize_trial  # noqa: E402

CANDIDATE = '0123456789abcdef' * 2 + '01234567'
BUCKET = 'bench-evidence-test'
AMI = 'ami-0d3897eb748e4b60e'
TEMPLATES = {'rvoip-bench-dut': 'c8i.4xlarge', 'rvoip-bench-caller': 'c8in.24xlarge'}
LANE_CONFIG = {'lane_vcpus': 16 + 2 * 96, 'dut_launch_template': 'rvoip-bench-dut',
               'caller_launch_template': 'rvoip-bench-caller', 'dut_instance_type': 'c8i.4xlarge',
               'caller_instance_type': 'c8in.24xlarge'}
MINI_CPS = 16          # the scaled trial rate of the synthetic evidence
MINI_SECONDS = 3       # every scaled stage
RECEIPT_STATUSES = ('PASS', 'WARN', 'FAIL', 'INFRA')

# The testbed of the chained baseline -> minor -> release runs.
TESTER = {'R1': 9000, 'R5': 8000, 'R2': 2600, 'R6': 2600}       # callers go invalid above
DUT = {'R1': 4000, 'R5': 3600, 'R2': 1200, 'R6': 1000}          # rvoip-bench-uas fails above


class Clock:
    def __init__(self, start=1_900_000_000.0):
        self.now = start

    def __call__(self):
        return self.now

    def advance(self, seconds):
        self.now += seconds


class World:
    """What the simulated hosts can do."""

    def __init__(self, **kw):
        self.dut_capacity = {}         # row -> sps: trial stages above fail
        self.ea_capacity = {}          # row -> sps: ea-uas fails above (baseline)
        self.tester_capacity = {}      # row -> sps: trials above are caller-invalid
        self.reject_status = {}        # row -> status of a failed attempt (503; None = timeout)
        self.overload_status = 503     # what the DUT answers above its admission max-cps
        self.setup_offset_ms = {}      # row -> added to every SRD
        self.dut_git_sha = CANDIDATE[:16]
        self.dut_starts = True         # False: dut-start fails (the process never writes ready.json)
        self.dut_clean_shutdown = True
        self.reserve_short = ()        # instance types create-capacity-reservation has no capacity for
        self.signal_on = None          # (agent command, signum): the orchestrator gets that signal there
        self.terminate_polls = 0       # describe-instances calls an instance stays shutting-down after terminate
        self.__dict__.update(kw)


def opt(args, name):
    return args[args.index(name) + 1] if name in args else None


def multi(args, name):
    """The values after `name` up to the next --option."""
    if name not in args:
        return []
    out = []
    for value in args[args.index(name) + 1:]:
        if value.startswith('--'):
            break
        out.append(value)
    return out


class Cloud:
    """In-process EC2 + SSM + S3 behind the AWS CLI argv that aws.py builds."""

    def __init__(self, root, world, clock):
        self.root, self.world, self.clock = Path(root), world, clock
        self.instances, self.commands = {}, {}
        self.reservations, self.tokens = {}, {}
        self.launched = []          # instance ids, in launch order
        self.calls = []             # every argv
        self.summarized = []        # trial prefixes the fake agent summarized
        self.trial_seq = 0

    # ---- S3
    def path(self, uri):
        bucket, key = aws_mod.split_s3(uri)
        return self.root / 's3' / bucket / key

    def keys(self, bucket, prefix=''):
        base = self.root / 's3' / bucket
        if not base.exists():
            return []
        return sorted(k for k in (str(p.relative_to(base)) for p in base.rglob('*') if p.is_file())
                      if k.startswith(prefix))

    def put(self, uri, data):
        p = self.path(uri)
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(data if isinstance(data, bytes) else data.encode())

    def get_json(self, uri):
        p = self.path(uri)
        return json.loads(p.read_text()) if p.is_file() else None

    def s3_cp(self, args):
        src, dst = [a for a in args if not a.startswith('--')][:2]
        if src.startswith('s3://'):
            p = self.path(src)
            if not p.is_file():
                raise aws_mod.AwsError('aws s3 cp: fatal error: An error occurred (404) when calling the HeadObject '
                                       'operation: Key "%s" does not exist' % aws_mod.split_s3(src)[1])
            if dst == '-':
                return p.read_text()
            data = p.read_bytes()
        else:
            if not Path(src).is_file():
                raise aws_mod.AwsError('aws s3 cp: The user-provided path %s does not exist.' % src)
            data = Path(src).read_bytes()
        if dst.startswith('s3://'):
            self.put(dst, data)
        else:
            Path(dst).parent.mkdir(parents=True, exist_ok=True)
            Path(dst).write_bytes(data)
        return ''

    def s3_sync(self, args):
        src, dst = args[1], args[2]
        assert src.startswith('s3://') and src.endswith('/'), src
        rules = [(args[i], args[i + 1]) for i in range(3, len(args) - 1) if args[i] in ('--exclude', '--include')]
        bucket, prefix = aws_mod.split_s3(src)
        for key in self.keys(bucket, prefix):
            rel = key[len(prefix):]
            take = True
            for kind, pattern in rules:   # the last matching filter wins (AWS CLI)
                if fnmatch.fnmatch(rel, pattern):
                    take = kind == '--include'
            if take:
                out = Path(dst) / rel
                out.parent.mkdir(parents=True, exist_ok=True)
                out.write_bytes(self.path(src + rel).read_bytes())
        return ''

    def s3_rm(self, args):
        uri = [a for a in args if not a.startswith('--')][0]
        self.path(uri).unlink(missing_ok=True)
        return ''

    def s3api_list_objects_v2(self, args):
        found = self.keys(opt(args, '--bucket'), opt(args, '--prefix') or '')
        return {'Contents': [{'Key': k} for k in found]} if found else {}

    def s3api_head_object(self, args):
        if not (self.root / 's3' / opt(args, '--bucket') / opt(args, '--key')).is_file():
            raise aws_mod.AwsError('An error occurred (404) when calling the HeadObject operation: Not Found')
        return {}

    def s3api_put_object(self, args):
        uri = 's3://%s/%s' % (opt(args, '--bucket'), opt(args, '--key'))
        if opt(args, '--if-none-match') == '*' and self.path(uri).is_file():
            raise aws_mod.AwsError('An error occurred (PreconditionFailed) when calling the PutObject operation')
        self.put(uri, Path(opt(args, '--body')).read_bytes())
        return {}

    # ---- EC2
    def ec2_run_instances(self, args):
        template = re.match(r'LaunchTemplateName=([^,]+)', opt(args, '--launch-template')).group(1)
        itype = opt(args, '--instance-type') or TEMPLATES[template]
        tags = {t['Key']: t['Value'] for t in json.loads(opt(args, '--tag-specifications'))[0]['Tags']}
        out = []
        for _ in range(int(opt(args, '--count'))):
            n = len(self.instances) + 1
            iid = 'i-%017x' % n
            target = json.loads(opt(args, '--capacity-reservation-specification') or '{}')
            reserved = target.get('CapacityReservationTarget', {}).get('CapacityReservationId')
            if reserved:
                r = self.reservations[reserved]
                assert r['State'] == 'active' and r['InstanceType'] == itype and r['AvailableInstanceCount'] > 0, r
                r['AvailableInstanceCount'] -= 1
            self.instances[iid] = {
                'InstanceId': iid, 'InstanceType': itype, 'ImageId': AMI, 'State': {'Name': 'running'},
                'CapacityReservationId': reserved,
                'Placement': {'AvailabilityZone': json.loads(opt(args, '--placement') or '{}').get(
                    'AvailabilityZone', 'us-west-2b'), 'GroupName': 'rvoip-bench'},
                'NetworkInterfaces': [{'PrivateIpAddresses': [
                    {'Primary': False, 'PrivateIpAddress': '10.0.%d.11' % n},
                    {'Primary': True, 'PrivateIpAddress': '10.0.%d.10' % n}]}],
                'Tags': [{'Key': k, 'Value': v} for k, v in tags.items()], 'role': tags['rvoip-role']}
            self.launched.append(iid)
            out.append({'InstanceId': iid})
        self.clock.advance(60)
        return {'Instances': out}

    SUBNETS = {'subnet-' + az[-1] * 2: az for az in ('us-west-2a', 'us-west-2b', 'us-west-2c')}

    def ec2_describe_launch_template_versions(self, args):
        eni = {'DeviceIndex': 0, 'Groups': ['sg-bench'], 'SubnetId': 'subnet-bb', 'AssociatePublicIpAddress': True}
        if opt(args, '--launch-template-name') == 'rvoip-bench-caller':
            eni['SecondaryPrivateIpAddressCount'] = 1
        return {'LaunchTemplateVersions': [{'LaunchTemplateData': {
            'NetworkInterfaces': [eni], 'Placement': {'AvailabilityZone': 'us-west-2b', 'GroupName': 'rvoip-bench'}}}]}

    def ec2_describe_subnets(self, args):
        def subnet(sid):
            az = self.SUBNETS[sid]
            return {'SubnetId': sid, 'VpcId': 'vpc-release', 'AvailabilityZone': az,
                    'Tags': [{'Key': 'Name', 'Value': 'rvoip-release-public-%s' % az}]}
        if '--subnet-ids' in args:
            return {'Subnets': [subnet(s) for s in multi(args, '--subnet-ids')]}
        want = dict(f.split('=', 1)[1].split(',Values=') for f in multi(args, '--filters'))
        return {'Subnets': [subnet(sid) for sid, az in self.SUBNETS.items()
                            if az == want['availability-zone'] and want['tag:Name'].endswith(az)]}

    def ec2_describe_instances(self, args):
        ids = multi(args, '--instance-ids')
        filters = dict(f[len('Name='):].split(',Values=') for f in multi(args, '--filters'))
        for inst in self.instances.values():
            if inst['State']['Name'] == 'shutting-down':
                inst['polls_left'] -= 1
                if inst['polls_left'] < 0:
                    inst['State'] = {'Name': 'terminated'}
        if 'placement-group-name' in filters:
            states = filters.get('instance-state-name', '').split(',')
            return {'Reservations': [{'Instances': [i for i in self.instances.values()
                                                    if i['Placement']['GroupName'] == filters['placement-group-name']
                                                    and i['State']['Name'] in states]}]}
        if not ids:
            return {'Reservations': []}    # the quota preflight: nothing else running
        return {'Reservations': [{'Instances': [self.instances[i] for i in ids]}]}

    def ec2_terminate_instances(self, args):
        for iid in multi(args, '--instance-ids'):
            if self.world.terminate_polls:
                self.instances[iid]['State'] = {'Name': 'shutting-down'}
                self.instances[iid]['polls_left'] = self.world.terminate_polls
            else:
                self.instances[iid]['State'] = {'Name': 'terminated'}
            reserved = self.reservations.get(self.instances[iid].get('CapacityReservationId'))
            if reserved and reserved['State'] == 'active':
                reserved['AvailableInstanceCount'] += 1
        return {}

    def ec2_describe_placement_groups(self, args):
        name = opt(args, '--group-names')
        return {'PlacementGroups': [{'GroupName': name, 'GroupArn': 'arn:aws:ec2:us-west-2:1:placement-group/' + name}]}

    def add_reservation(self, itype, count, tags, az='us-west-2b', pg=None, end=None):
        cid = 'cr-%017x' % (len(self.reservations) + 1)
        self.reservations[cid] = {
            'CapacityReservationId': cid, 'InstanceType': itype, 'AvailabilityZone': az, 'State': 'active',
            'TotalInstanceCount': count, 'AvailableInstanceCount': count, 'PlacementGroupArn': pg, 'EndDate': end,
            'Tags': [{'Key': k, 'Value': v} for k, v in tags.items()]}
        return cid

    def ec2_create_capacity_reservation(self, args):
        itype = opt(args, '--instance-type')
        if itype in self.world.reserve_short:
            raise aws_mod.AwsError('aws ec2 create-capacity-reservation: An error occurred (InsufficientInstanceCapacity) '
                                   'when calling the CreateCapacityReservation operation: no %s capacity' % itype)
        assert opt(args, '--instance-match-criteria') == 'targeted' and opt(args, '--end-date-type') == 'limited'
        token = opt(args, '--client-token')
        if token not in self.tokens:
            tags = {t['Key']: t['Value'] for t in json.loads(opt(args, '--tag-specifications'))[0]['Tags']}
            self.tokens[token] = self.add_reservation(itype, int(opt(args, '--instance-count')), tags,
                                                      opt(args, '--availability-zone'),
                                                      opt(args, '--placement-group-arn'), opt(args, '--end-date'))
        return {'CapacityReservation': dict(self.reservations[self.tokens[token]])}

    def ec2_describe_capacity_reservations(self, args):
        found = list(self.reservations.values())
        ids = multi(args, '--capacity-reservation-ids')
        if ids:
            found = [r for r in found if r['CapacityReservationId'] in ids]
        for f in multi(args, '--filters'):
            name, values = f[len('Name='):].split(',Values=')
            values = values.split(',')
            if name == 'state':
                found = [r for r in found if r['State'] in values]
            elif name.startswith('tag:'):
                found = [r for r in found if any(t['Key'] == name[4:] and t['Value'] in values for t in r['Tags'])]
        return {'CapacityReservations': [dict(r) for r in found]}

    def ec2_modify_capacity_reservation(self, args):
        r = self.reservations[opt(args, '--capacity-reservation-id')]
        assert r['State'] == 'active' and opt(args, '--end-date-type') == 'limited', r
        r['EndDate'] = opt(args, '--end-date')
        return {'Return': True}

    def ec2_create_tags(self, args):
        for rid in multi(args, '--resources'):
            r = self.reservations[rid]
            for kv in multi(args, '--tags'):
                k, v = re.match(r'Key=([^,]+),Value=(.*)', kv).groups()
                r['Tags'] = [t for t in r['Tags'] if t['Key'] != k] + [{'Key': k, 'Value': v}]
        return {}

    def tags_of(self, cid):
        return {t['Key']: t['Value'] for t in self.reservations[cid]['Tags']}

    def ec2_cancel_capacity_reservation(self, args):
        r = self.reservations[opt(args, '--capacity-reservation-id')]
        if r['State'] != 'active':
            raise aws_mod.AwsError('IncorrectState: %s is %s' % (r['CapacityReservationId'], r['State']))
        r['State'] = 'cancelled'
        return {'Return': True}

    def held_reservations(self):
        return sorted(cid for cid, r in self.reservations.items() if r['State'] == 'active')

    def ssm_describe_instance_information(self, args):
        return {'InstanceInformationList': [{'InstanceId': i, 'PingStatus': 'Online'} for i in self.instances]}

    def service_quotas_get_service_quota(self, args):
        return {'Quota': {'Value': 1920.0}}

    # ---- SSM and the fake bench-agent.sh
    def ssm_send_command(self, args):
        ids = multi(args, '--instance-ids')
        script = json.loads(opt(args, '--parameters'))['commands'][0]
        fetch = re.search(r'aws s3 cp --only-show-errors (\S+agent/bench-agent\.sh) ', script)
        line = re.search(r'^METRICS_PORT=\d+ bash /opt/rvoip-bench/bench-agent\.sh (.+)$', script, re.M)
        assert fetch and line, script
        argv = shlex.split(line.group(1))
        cid = 'cmd-%04d' % (len(self.commands) + 1)
        results = {}
        for iid in ids:
            if not self.path(fetch.group(1)).is_file():
                results[iid] = self.invocation('Failed', 1, err='bench-agent.sh not uploaded')
            else:
                results[iid] = self.agent(self.instances[iid], argv)
        self.commands[cid] = results
        return {'Command': {'CommandId': cid}}

    def ssm_get_command_invocation(self, args):
        return dict(self.commands[opt(args, '--command-id')][opt(args, '--instance-id')])

    def ssm_cancel_command(self, args):
        return {}

    @staticmethod
    def invocation(status='Success', rc=0, out='', err=''):
        return {'Status': status, 'ResponseCode': rc, 'StandardOutputContent': out, 'StandardErrorContent': err}

    def agent(self, inst, argv):
        cmd, rest = argv[0], argv[1:]
        if self.world.signal_on and self.world.signal_on[0] == cmd:
            signum = self.world.signal_on[1]
            self.world.signal_on = None
            os.kill(os.getpid(), signum)
        if cmd == 'install':
            self.clock.advance(60)
            facts = {'instance_id': inst['InstanceId'], 'kernel': '6.8.0-1015-aws', 'tuning': {'applied': ['sysctl']},
                     'nproc': 96 if inst['role'] == 'caller' else 16}
            return self.invocation(out='installing\n' + json.dumps(facts) + '\n')
        if cmd == 'dut-start':
            self.clock.advance(10)
            if not self.world.dut_starts:
                return self.invocation('Failed', 1, err='rvoip-bench-uas exited before ready.json')
            ready = {'pid': 4242, 'listen': '10.0.1.10:5061', 'git_sha': self.world.dut_git_sha, 'version': '0.4.1'}
            return self.invocation(out='starting %s\n%s\n' % (rest[1], json.dumps(ready, indent=2)))
        if cmd == 'caller-run':
            prefix, index = rest
            self.put(prefix + 'barrier/ready-%s' % index, '{}')
            self.put(prefix + 'caller-%s.tar.gz' % index, 'caller evidence')
            self.put(prefix + 'caller-%s.tar.gz.sha256' % index, '%s  caller-%s.tar.gz\n' % ('c' * 64, index))
            if index == '1':
                plan = self.get_json(prefix + 'plan.json')
                self.clock.advance(sum(s['seconds'] for s in plan['profile']['stages']) + plan['hold_ms'] / 1000 + 30)
            return self.invocation()
        if cmd == 'dut-finish':
            self.clock.advance(30)
            self.put(rest[0] + 'dut.tar.gz', 'dut evidence')
            self.put(rest[0] + 'dut.tar.gz.sha256', '%s  dut.tar.gz\n' % ('d' * 64))
            return self.invocation()
        if cmd == 'summarize':
            self.clock.advance(60)
            self.summarize(rest[0])
            return self.invocation()
        return self.invocation('Failed', 64, err='unknown command %s' % cmd)

    def summarize(self, prefix):
        """merged/summary.json of a scaled-down synthetic run of the trial's plan.json."""
        plan = self.get_json(prefix + 'plan.json')
        w = self.world
        row, hosts, rate = plan['row'], len(plan['caller_hosts']), plan['rate']
        k = MINI_CPS / float(rate)
        stages = plan['profile']['stages']
        minis = [max(hosts, int(round(s['cps'] * k / hosts)) * hosts) for s in stages]
        mini_profile = dict(plan['profile'], stages=[{'name': s['name'], 'cps': m, 'seconds': MINI_SECONDS}
                                                     for s, m in zip(stages, minis)])
        admission = plan.get('admission') or {}
        if admission.get('mode') == 'max-cps':
            cap, status = admission['max_cps'], w.overload_status
        elif plan['dut_kind'] == 'ea-uas':
            cap, status = w.ea_capacity.get(row), w.reject_status.get(row, 503)
        else:
            cap, status = w.dut_capacity.get(row), w.reject_status.get(row, 503)
        over = [m for s, m in zip(stages, minis) if s['name'].startswith('trial-') and cap is not None and s['cps'] > cap]
        limit = min(cap * k, min(over) - 1) if over else None
        trial_cps = max(s['cps'] for s in stages if s['name'].startswith('trial-'))
        tester = w.tester_capacity.get(row)
        media = plan['media'] != 'none'
        self.trial_seq += 1
        work = self.root / 'agent' / ('%04d' % self.trial_seq)
        start = fx.T0 + 1000 * self.trial_seq
        (work).mkdir(parents=True)
        (work / 'plan.json').write_text(json.dumps(dict(plan, profile=mini_profile)))
        for i in range(hosts):
            fx.write_caller(work / ('caller-%d' % (i + 1)), mini_profile, i, hosts, start, MINI_CPS, limit=limit,
                            media=media, hold_ms=plan['hold_ms'], process_offset_ms=400 + 100 * i,
                            transport='tls_srtp' if plan['transport'] == 'tls' else 'udp_rtp',
                            lateness_ms=20.0 if tester is not None and trial_cps > tester else 0.4,
                            reject_status=status, setup_offset_ms=w.setup_offset_ms.get(row, 0.0))
        fx.write_dut(work / 'dut', start, mini_profile, limit=limit, media=('srtp' if media else None),
                     clean=w.dut_clean_shutdown)
        path, _ = summarize_trial.write(work, criterion='rfc')
        self.put(prefix + 'merged/summary.json', path.read_bytes())
        self.put(prefix + 'evidence-manifest.json', json.dumps({'dut.tar.gz': 'd' * 64}))
        self.summarized.append(prefix)
        shutil.rmtree(work)


class FakeAws(aws_mod.Aws):
    """aws.Aws with the AWS CLI replaced by the in-process Cloud."""

    def __init__(self, cloud, region='us-west-2', log=print):
        super().__init__(region, log=log)
        self.cloud = cloud

    def call(self, *args, retries=6, parse=True, input_text=None, transient=aws_mod.TRANSIENT):
        self.cloud.calls.append(args)
        handler = getattr(self.cloud, ('%s_%s' % (args[0], args[1])).replace('-', '_'))
        out = handler(list(args[2:]))
        if not parse:
            return out if isinstance(out, str) else ''
        return out if isinstance(out, dict) else {}


def fake_tls(out, dut_ips, caller_ips):
    out.mkdir(parents=True, exist_ok=True)
    for name in ('ca.crt', 'dut.crt', 'dut.key', 'caller.crt', 'caller.key'):
        (out / name).write_text('%s for %s %s\n' % (name, dut_ips, caller_ips))


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


class Bed:
    """A temp testbed: bucket, workdir, perf-baselines, binaries, lane config, the patches."""

    def __init__(self):
        self.tmp = Path(tempfile.mkdtemp(prefix='run-lane-offline-'))
        self.clock = Clock()
        self.cloud = Cloud(self.tmp, World(), self.clock)
        self.repo = self.tmp / 'repo'     # stands in for the checkout: rates.json paths are receipt-relative to it
        self.baselines = self.repo / 'crates/sip/rvoip-sip/perf-baselines/rfc7502'
        self.baselines.mkdir(parents=True)
        self.bins = {}
        for name in ('rvoip-bench-uas', 'rvoip-loadgen', 'ea-uas'):
            self.bins[name] = self.tmp / 'bin' / name
            self.bins[name].parent.mkdir(exist_ok=True)
            self.bins[name].write_text('#!/bin/sh\n# fake %s\n' % name)
        self.lane_config = self.tmp / 'lane.json'
        self.lane_config.write_text(json.dumps(LANE_CONFIG))
        self.patches = [mock.patch.object(run_lane, 'WORKDIR', self.tmp / 'work'),
                        mock.patch.object(run_lane, 'REPO', self.repo),
                        mock.patch.object(run_lane, 'BASELINES', self.baselines),
                        mock.patch.object(run_lane, 'make_tls', fake_tls),
                        mock.patch.object(run_lane, 'Aws', lambda region, log=print: FakeAws(self.cloud, region, log)),
                        mock.patch('time.time', self.clock)]
        for p in self.patches:
            p.start()

    def close(self):
        for p in reversed(self.patches):
            p.stop()
        shutil.rmtree(self.tmp, ignore_errors=True)

    def argv(self, profile, run_id, *extra):
        argv = ['--profile', profile, '--candidate', CANDIDATE, '--receipt', str(self.receipt_path(run_id)),
                '--bucket', BUCKET, '--lane-config', str(self.lane_config), '--run-id', run_id,
                '--caller-bin', str(self.bins['rvoip-loadgen'])]
        if profiles.PROFILES[profile].get('ea_uas'):
            argv += ['--ea-uas-bin', str(self.bins['ea-uas'])]
        else:
            argv += ['--dut-bin', str(self.bins['rvoip-bench-uas'])]
        return argv + list(extra)

    def receipt_path(self, run_id):
        return self.tmp / 'receipts' / (run_id + '.json')

    def run(self, profile, run_id, *extra, world=None):
        """main(); returns (exit code, receipt or None, the run's log text)."""
        if world is not None:
            self.cloud.world = world
        receipt = self.receipt_path(run_id)
        receipt.unlink(missing_ok=True)
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = run_lane.main(self.argv(profile, run_id, *extra))
        return code, (json.loads(receipt.read_text()) if receipt.exists() else None), out.getvalue()

    def local(self, run_id):
        return self.tmp / 'work' / run_id

    def s3_run(self, run_id):
        return 's3://%s/bench/%s/' % (BUCKET, run_id)

    def env_id(self):
        return 'aws-usw2-c8i4xl-2xc8in24xl-%s-%s' % (AMI[4:12], sha256(self.bins['rvoip-loadgen'])[:8])

    def rates(self):
        path = self.baselines / self.env_id() / 'rates.json'
        return json.loads(path.read_text()) if path.exists() else None

    def alive(self):
        return [i for i, v in self.cloud.instances.items() if v['State']['Name'] != 'terminated']


def world_for(dut=None, **kw):
    return World(dut_capacity=dict(DUT if dut is None else dut), **kw)


class LaneCase(unittest.TestCase):
    bed = None

    def check(self, code, receipt, log, want_code, want_status):
        tail = '\n'.join(log.splitlines()[-40:])
        self.assertEqual(code, want_code, tail)
        self.assertIsNotNone(receipt, tail)
        self.assertEqual(receipt['status'], want_status, json.dumps(receipt.get('reasons'), indent=1) + '\n' + tail)
        self.assert_receipt_schema(receipt)
        self.assertEqual(self.bed.alive(), [], 'the lane is torn down')
        self.assertFalse(self.bed.cloud.path('s3://%s/bench/lock.json' % BUCKET).exists(), 'the lane lock is released')

    def assert_receipt_schema(self, receipt):
        """docs/BENCH_GATE_INTERFACE.md receipt fields."""
        self.assertEqual(receipt['schema'], 'rvoip-bench-lane-receipt-v1')
        self.assertIn(receipt['profile'], profiles.PROFILES)
        self.assertEqual(receipt['candidate_sha'], CANDIDATE)
        self.assertIn(receipt['status'], RECEIPT_STATUSES)
        self.assertIsInstance(receipt['reasons'], list)
        for key in ('run_id', 'started_at', 'ended_at', 'duration_seconds', 'baseline', 'recommendation', 'tool'):
            self.assertIn(key, receipt)
        env = receipt['environment']
        for key in ('env_id', 'region', 'availability_zone', 'dut_instance_type', 'caller_instance_type', 'caller_count',
                    'sources_per_caller', 'connections_per_source', 'ami_id', 'kernel', 'dut_sha256', 'caller_sha256',
                    'ea_uas_sha256'):
            self.assertIn(key, env)
        self.assertGreaterEqual(env['caller_count'], 2)
        self.assertEqual(env['sources_per_caller'], 2)
        self.assertEqual(env['ami_id'], AMI)
        self.assertEqual(env['caller_sha256'], sha256(self.bed.bins['rvoip-loadgen']))
        self.assertIsInstance(receipt['rows'], list)
        for row in receipt['rows']:
            self.assertIn(row['verdict'], ('pass', 'warn', 'fail', 'infra'))
            for key in ('id', 'transport', 'media', 'hold_ms', 'trials'):
                self.assertIn(key, row)
            for t in row['trials']:
                for key in ('rate', 'outcome', 'attempted', 'established', 'attempt_failures', 'status_counts',
                            'invalid_reason', 'evidence'):
                    self.assertIn(key, t)
        ev = receipt['evidence']
        self.assertRegex(ev['manifest_sha256'] or '', r'^[0-9a-f]{64}$')
        manifest = self.bed.cloud.path(ev['s3_prefix'] + 'manifest.json')
        self.assertEqual(sha256(manifest), ev['manifest_sha256'])
        self.assertEqual(json.loads(self.bed.cloud.path(ev['s3_prefix'] + 'receipt.json').read_text()), receipt)
        self.assertIn('instance_hours', receipt['cost'])
        self.assertGreater(receipt['cost']['estimated_usd'], 0)

    def assert_report_complete(self, receipt, overload):
        """Both reports rendered to S3, with no pending cell for evidence the run has."""
        report = receipt['evidence']['report']
        self.assertEqual(sorted(report), ['rfc7502-ietf-report', 'rfc7502-report'])
        for stem, uris in report.items():
            for kind in ('md', 'html'):
                self.assertTrue(self.bed.cloud.path(uris[kind]).is_file(), uris[kind])
            text = self.bed.cloud.path(uris['md']).read_text()
            pending = [line for line in text.splitlines() if rfc_report.PENDING in line]
            self.assertEqual(pending, [], '%s has pending cells:\n%s' % (stem, '\n'.join(pending)))
            if stem == 'rfc7502-report':
                self.assertEqual('Overload and recovery (R4)' in text, overload)
        return {stem: self.bed.cloud.path(uris['md']).read_text() for stem, uris in report.items()}

    @staticmethod
    def rows(receipt):
        return {r['id']: r for r in receipt['rows']}


# ---------------------------------------------------------------------------

class SmokeAndCliTests(LaneCase):
    def setUp(self):
        self.bed = Bed()

    def tearDown(self):
        self.bed.close()

    def test_smoke_four_rows_pass(self):
        code, receipt, log = self.bed.run('smoke', 'smoke-1', world=World())
        self.check(code, receipt, log, 0, 'PASS')
        rows = self.rows(receipt)
        self.assertEqual(sorted(rows), ['R1', 'R2', 'R5', 'R6'])
        for row in rows.values():
            self.assertEqual(row['verdict'], 'pass', row)
            self.assertEqual(row['trials'][0]['outcome'], 'pass')
        self.assertEqual(rows['R1']['trials'][0]['rate'], 400)
        self.assertEqual(len(self.bed.cloud.launched), 3)
        manifest = json.loads(self.bed.cloud.path(self.bed.s3_run('smoke-1') + 'manifest.json').read_text())
        self.assertIn('bench/smoke-1/R1/t00-r400/dut.tar.gz', manifest['archives'])

    def test_smoke_on_a_failing_dut_fails(self):
        code, receipt, log = self.bed.run('smoke', 'smoke-2', '--rows', 'R1', world=World(dut_capacity={'R1': 200}))
        self.check(code, receipt, log, 1, 'FAIL')

    def test_dut_built_at_another_sha_is_refused(self):
        code, receipt, log = self.bed.run('smoke', 'smoke-3', '--rows', 'R1', world=World(dut_git_sha='feedfacecafe'))
        self.assertEqual(code, 2, log)
        self.assertEqual(receipt['status'], 'INFRA')
        self.assertEqual(self.bed.alive(), [])

    def test_release_without_rates_is_a_usage_error(self):
        code, receipt, log = self.bed.run('release', 'rel-norates', world=world_for())
        self.assertEqual(code, 2, log)
        self.assertIn('no recorded rates', ' '.join(receipt['reasons']))
        self.assertEqual(self.bed.alive(), [])

    def test_lock_held_by_another_run_is_infra(self):
        self.bed.cloud.put('s3://%s/bench/lock.json' % BUCKET, json.dumps({'run_id': 'someone-else'}))
        code, receipt, log = self.bed.run('smoke', 'smoke-lock', world=World())
        self.assertEqual(code, 3, log)
        self.assertEqual(self.bed.cloud.launched, [])
        self.assertTrue(self.bed.cloud.path('s3://%s/bench/lock.json' % BUCKET).exists(), 'the other run keeps its lock')

    def test_dry_run_launches_nothing(self):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = run_lane.main(self.bed.argv('release', 'dry-1', '--dry-run'))
        self.assertEqual(code, 0, out.getvalue())
        self.assertEqual(self.bed.cloud.launched, [])
        self.assertFalse(self.bed.receipt_path('dry-1').exists())
        self.assertEqual(self.bed.cloud.keys(BUCKET), [], 'a dry run writes nothing to the bucket')

    def test_dry_run_without_terraform_or_lane_config(self):
        argv = self.bed.argv('release', 'dry-2', '--dry-run')
        i = argv.index('--lane-config')
        del argv[i:i + 2]
        out = io.StringIO()
        with contextlib.redirect_stdout(out), mock.patch.dict('os.environ', {'PATH': str(self.bed.tmp / 'empty')}):
            code = run_lane.main(argv)
        self.assertEqual(code, 2, out.getvalue())
        self.assertEqual(self.bed.cloud.keys(BUCKET), [], 'a dry run writes nothing to the bucket')

    def test_help_and_bad_args(self):
        proc = subprocess.run([sys.executable, str(BENCH / 'run_lane.py'), '--help'], capture_output=True, text=True)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn('--profile', proc.stdout)
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            self.assertEqual(run_lane.main(['--profile', 'release', '--candidate', 'abc', '--receipt', 'x']), 2)
        self.assertIn('40-hex', err.getvalue())


class ChainTests(LaneCase):
    """baseline --record -> minor --record -> release, on one env-id."""

    @classmethod
    def setUpClass(cls):
        cls.bed = Bed()
        cls.baseline = cls.bed.run('baseline', 'base-1', '--record', world=World(tester_capacity=TESTER))
        cls.baseline_rates = cls.bed.rates()
        cls.minor = cls.bed.run('minor', 'minor-1', '--record', world=world_for(tester_capacity=TESTER))
        cls.rates_text = (cls.bed.baselines / cls.bed.env_id() / 'rates.json').read_text() if cls.bed.rates() else None

    @classmethod
    def tearDownClass(cls):
        cls.bed.close()

    def setUp(self):
        if self.rates_text is not None:
            (self.bed.baselines / self.bed.env_id() / 'rates.json').write_text(self.rates_text)

    # ---- baseline
    def test_baseline_tester_limited_rows_and_record(self):
        code, receipt, log = self.baseline
        self.check(code, receipt, log, 0, 'PASS')
        rows = self.rows(receipt)
        expect = {'R1': 8800, 'R5': 7920, 'R2': 2400, 'R6': 2400}   # r0 x 1.1^k below the tester limit
        for row_id, r in expect.items():
            self.assertEqual(rows[row_id]['r'], r, rows[row_id])
            self.assertTrue(rows[row_id]['tester_limited'], rows[row_id])
            self.assertLessEqual(rows[row_id]['r'], TESTER[row_id])
            self.assertGreater(rows[row_id]['r'], TESTER[row_id] / 1.1 - 4)
        self.assertNotIn('R4', rows, 'baseline has no overload row')
        rates = self.baseline_rates
        self.assertEqual(rates['schema'], 'rvoip-rfc7502-rates-v1')
        for row_id, r in expect.items():
            self.assertEqual(rates['baseline_sps'][row_id]['r'], r)
            self.assertEqual(rates['tester_caps'][row_id], max(4, int(r * 0.67) // 4 * 4))
        self.assertEqual(rates['rows'], {}, 'a baseline records no DUT R')
        text = self.assert_report_complete(receipt, overload=False)['rfc7502-report']
        self.assertIn('| R1 | TLS | none | 0 s | >= 8,800 sps | tester (', text, 'a caller-invalid stop is a lower bound')

    def test_baseline_converges_on_an_ea_uas_limit(self):
        code, receipt, log = self.bed.run('baseline', 'base-2', '--rows', 'R1', world=World(ea_capacity={'R1': 8500}))
        self.check(code, receipt, log, 0, 'PASS')
        row = self.rows(receipt)['R1']
        self.assertFalse(row['tester_limited'], row)
        self.assertLessEqual(row['r'], 8500)
        self.assertGreaterEqual(row['r'], 8500 * 0.95, row['search'])

    def test_rebaseline_is_not_capped_by_the_old_caps(self):
        # A second baseline on a recorded env-id searches the testbed again: the old tester caps (0.67 x the
        # last baseline) and the DUT's recorded R must neither cap nor seed it.
        code, receipt, log = self.bed.run('baseline', 'base-3', '--rows', 'R1', world=World(tester_capacity=TESTER))
        self.check(code, receipt, log, 0, 'PASS')
        row = self.rows(receipt)['R1']
        self.assertIsNone(row['tester_cap'], row)
        self.assertEqual(row['r'], 8800, row['search'])

    # ---- minor
    def test_minor_converges_and_records(self):
        code, receipt, log = self.minor
        self.check(code, receipt, log, 0, 'PASS')
        rows = self.rows(receipt)
        rates = json.loads(self.rates_text)
        for row_id, cap in DUT.items():
            row = rows[row_id]
            self.assertEqual(row['verdict'], 'pass', row)
            self.assertEqual(row['tester_cap'], self.baseline_rates['tester_caps'][row_id])
            self.assertLessEqual(row['r'], cap, row['search'])
            self.assertGreaterEqual(row['r'], cap * 0.98 - 4, row['search'])
            self.assertFalse(row['tester_limited'])
            lat = row['latency_trial']
            self.assertEqual(lat['rate'], profiles.quantize(row['r'] * 0.9))
            self.assertEqual(lat['outcome'], 'pass')
            self.assertEqual(row['latency']['verdict'], 'pass')
            rec = rates['rows'][row_id]
            self.assertEqual(rec['r'], row['r'])
            self.assertEqual(rec['tester_cap'], self.baseline_rates['tester_caps'][row_id])
            self.assertEqual(rec['latency_rate'], lat['rate'])
            for key in ('srd_ms', 'sdd_ms'):
                self.assertIsNotNone(rec[key], rec)
                self.assertIsNotNone(rec[key]['p99'], rec)
        self.assertEqual(rates['baseline_sps'], self.baseline_rates['baseline_sps'], 'minor keeps the baseline')
        r4 = rows['R4']
        self.assertEqual(r4['verdict'], 'pass', r4)
        self.assertEqual(r4['ceiling'], profiles.quantize(rows['R2']['r']))
        stages = {s['stage']: s for s in r4['stages']}
        self.assertEqual(len(stages), 6)
        self.assertGreater(stages['trial-3']['rejected_503'], 0, 'overload is shed with 503')
        self.assertGreater(stages['trial-4']['rejected_503'], 0)
        for name in ('trial-5', 'trial-6'):
            self.assertEqual(stages[name]['established'], stages[name]['offered'], 'recovery at 100 %')
            self.assertEqual(stages[name]['other_failures'], 0)

    def test_minor_report_has_no_pending_cell(self):
        code, receipt, log = self.minor
        text = self.assert_report_complete(receipt, overload=True)['rfc7502-report']
        r1 = self.rows(receipt)['R1']['r']
        self.assertIn('| R1 | TLS | none | 0 s | %s sps | DUT | >= 8,800 sps | 5,896 |' % rfc_report.fmt(r1), text,
                      'the recorded baseline and tester cap reach the report')

    def test_minor_overload_with_non_503_failures_fails(self):
        world = world_for(tester_capacity=TESTER, overload_status=500)
        code, receipt, log = self.bed.run('minor', 'minor-r4-500', '--rows', 'R2', world=world)
        self.check(code, receipt, log, 1, 'FAIL')
        r4 = self.rows(receipt)['R4']
        self.assertEqual(r4['verdict'], 'fail')
        self.assertIn('non-503 failures', r4['reason'])
        self.assertEqual(self.rows(receipt)['R2']['verdict'], 'pass')

    def test_full_profile_runs_the_rfc7502_search(self):
        code, receipt, log = self.bed.run('full', 'full-1', '--rows', 'R5', world=world_for(tester_capacity=TESTER))
        self.check(code, receipt, log, 0, 'PASS')
        row = self.rows(receipt)['R5']
        self.assertEqual(row['search']['algorithm'], 'rfc7502')
        self.assertIn('converged: 10 successes', row['search']['stop_reason'])
        self.assertLessEqual(row['r'], DUT['R5'])
        self.assertGreaterEqual(row['r'], DUT['R5'] * 0.9)
        self.assertEqual(row['latency']['verdict'], 'pass')
        self.assertEqual(self.rows(receipt)['R4']['verdict'], 'pass') if 'R4' in self.rows(receipt) else None

    def test_record_is_refused_where_nothing_is_recorded(self):
        for profile in ('release', 'smoke'):
            code, receipt, log = self.bed.run(profile, 'rec-%s' % profile, '--record', world=world_for())
            self.assertEqual(code, 2, log)
        self.assertEqual(self.bed.alive(), [])

    def test_minor_row_below_the_recorded_r_fails(self):
        world = world_for(dict(DUT, R1=int(DUT['R1'] * 0.85)), tester_capacity=TESTER, reject_status={'R1': None})
        code, receipt, log = self.bed.run('minor', 'minor-slow', '--rows', 'R1', world=world)
        self.check(code, receipt, log, 1, 'FAIL')
        self.assertIn('0.95 x recorded', self.rows(receipt)['R1']['reason'])

    # ---- release
    def release(self, run_id, world):
        return self.bed.run('release', run_id, world=world)

    def test_release_pass_when_capacity_unchanged(self):
        code, receipt, log = self.release('rel-same', world_for(tester_capacity=TESTER))
        self.check(code, receipt, log, 0, 'PASS')
        rates = json.loads(self.rates_text)
        for row_id in ('R1', 'R5', 'R2'):
            row = self.rows(receipt)[row_id]
            self.assertEqual(row['verdict'], 'pass', row)
            r = rates['rows'][row_id]['r']
            self.assertEqual(row['recorded_r'], r)
            kinds = [(t['kind'], t['rate'], t['outcome']) for t in row['trials']]
            q = profiles.quantize
            self.assertEqual(kinds, [('confirm', q(r * .95), 'pass')] * 3 + [('probe_up', q(r * 1.05), 'fail'),
                                                                            ('latency', q(r * .9), 'pass')])
            self.assertEqual(row['latency']['verdict'], 'pass')
            self.assertEqual(row['latency']['baseline_srd_ms'], rates['rows'][row_id]['srd_ms'])
        self.assertEqual(receipt['recommendation'], {})
        self.assertEqual(receipt['baseline']['sha256'], sha256(self.bed.baselines / self.bed.env_id() / 'rates.json'))

    def test_release_fails_when_capacity_drops(self):
        world = world_for({k: int(v * 0.9) for k, v in DUT.items()}, tester_capacity=TESTER)
        code, receipt, log = self.release('rel-drop', world)
        self.check(code, receipt, log, 1, 'FAIL')
        row = self.rows(receipt)['R1']
        self.assertEqual(row['verdict'], 'fail')
        self.assertIn('confirmation at 0.95xR', row['reason'])
        self.assertEqual(len(row['trials']), 1, 'the first failing confirmation decides the row')

    def test_release_warns_when_capacity_rises(self):
        world = world_for({k: int(v * 1.1) for k, v in DUT.items()}, tester_capacity=TESTER)
        code, receipt, log = self.release('rel-rise', world)
        self.check(code, receipt, log, 0, 'WARN')
        rates = json.loads(self.rates_text)
        for row_id in ('R1', 'R5', 'R2'):
            self.assertEqual(self.rows(receipt)[row_id]['verdict'], 'warn')
            self.assertEqual(receipt['recommendation'][row_id],
                             {'new_r': profiles.quantize(rates['rows'][row_id]['r'] * 1.05), 'basis': '1.05xR passed'})
        self.assertTrue(any('likely improvement' in r for r in receipt['reasons']))

    def test_release_fails_on_srd_regression(self):
        world = world_for(tester_capacity=TESTER, setup_offset_ms={'R5': 30.0})
        code, receipt, log = self.release('rel-srd', world)
        self.check(code, receipt, log, 1, 'FAIL')
        rows = self.rows(receipt)
        self.assertEqual(rows['R5']['verdict'], 'fail')
        self.assertIn('SRD p99', rows['R5']['latency']['reason'])
        self.assertEqual(rows['R1']['verdict'], 'pass')

    def test_release_srd_within_the_floor_passes(self):
        world = world_for(tester_capacity=TESTER, setup_offset_ms={'R5': 8.0})   # < max(25 %, 10 ms)
        code, receipt, log = self.release('rel-srd-ok', world)
        self.check(code, receipt, log, 0, 'PASS')

    def test_release_dut_that_does_not_drain_fails(self):
        code, receipt, log = self.release('rel-halt', world_for(tester_capacity=TESTER, dut_clean_shutdown=False))
        self.check(code, receipt, log, 1, 'FAIL')
        row = self.rows(receipt)['R1']
        self.assertIn('did not drain cleanly', row['reason'])
        self.assertEqual(len(row['trials']), 1)

    def test_search_on_a_dut_that_never_starts_is_infra_not_max_wall(self):
        # Two void trials in a row pause the search; that is not --max-wall (exit 4 would be re-invoked forever).
        code, receipt, log = self.bed.run('minor', 'minor-nostart', '--rows', 'R1',
                                          world=world_for(tester_capacity=TESTER, dut_starts=False))
        self.check(code, receipt, log, 3, 'INFRA')
        self.assertIn('offered no session', ' '.join(receipt['reasons']))

    def test_minor_dut_that_does_not_drain_halts_the_search(self):
        code, receipt, log = self.bed.run('minor', 'minor-halt', '--rows', 'R1',
                                          world=world_for(tester_capacity=TESTER, dut_clean_shutdown=False))
        self.check(code, receipt, log, 1, 'FAIL')
        self.assertIn('dut_clean_shutdown is false', self.rows(receipt)['R1']['reason'])

    def test_release_infra_after_three_invalid_trials(self):
        world = world_for(tester_capacity=dict(TESTER, R1=1000))
        code, receipt, log = self.release('rel-invalid', world)
        self.check(code, receipt, log, 3, 'INFRA')
        row = self.rows(receipt)['R1']
        self.assertEqual(row['verdict'], 'infra')
        self.assertEqual([t['outcome'] for t in row['trials']], ['invalid'] * 3)
        self.assertIn('3 invalid trials in a row', row['reason'])
        self.assertTrue(all(t['invalid_reason'] for t in row['trials']))

    # ---- resume
    def test_resume_of_a_finished_run_rewrites_the_receipt(self):
        code, first, log = self.release('rel-done', world_for(tester_capacity=TESTER))
        self.assertEqual(code, 0, log)
        launched = len(self.bed.cloud.launched)
        shutil.rmtree(self.bed.local('rel-done'))              # another machine
        code, again, log = self.bed.run('release', 'rel-done', '--resume')
        self.assertEqual(code, 0, log)
        self.assertEqual(again, first)
        self.assertEqual(len(self.bed.cloud.launched), launched, 'nothing launched')
        # a FAIL verdict is final too
        code, failed, log = self.release('rel-done-fail', world_for({k: int(v * 0.9) for k, v in DUT.items()},
                                                                    tester_capacity=TESTER))
        self.assertEqual(code, 1, log)
        code, again, log = self.bed.run('release', 'rel-done-fail', '--resume')
        self.assertEqual((code, again['status']), (1, 'FAIL'), log)

    def test_resume_after_max_wall_reruns_no_trial(self):
        world = world_for(tester_capacity=TESTER)
        before = len(self.bed.cloud.summarized)
        code, receipt, log = self.bed.run('minor', 'minor-wall', '--rows', 'R1', '--max-wall', '45', world=world)
        self.assertEqual(code, 4, log)
        self.assertIsNone(receipt)
        self.assertEqual(self.bed.alive(), [], 'the lane is torn down at --max-wall')
        self.assertFalse(self.bed.cloud.path('s3://%s/bench/lock.json' % BUCKET).exists())
        first_leg = self.bed.cloud.summarized[before:]
        self.assertGreaterEqual(len(first_leg), 2, log)
        state = json.loads(self.bed.cloud.path(self.bed.s3_run('minor-wall') + 'R1/search.json').read_text())
        self.assertFalse(state['done'])
        self.assertEqual(len(state['trials']), len(first_leg))
        shutil.rmtree(self.bed.local('minor-wall'))            # resume on another machine
        code, receipt, log = self.bed.run('minor', 'minor-wall', '--rows', 'R1', '--resume')
        self.check(code, receipt, log, 0, 'PASS')
        both = self.bed.cloud.summarized[before:]
        self.assertEqual(len(both), len(set(both)), 'a trial with a summary is not rerun')
        row = self.rows(receipt)['R1']
        self.assertEqual(row['search']['trials'], len([p for p in both if '/latency-' not in p]))
        self.assertLessEqual(row['r'], DUT['R1'])

    def test_release_resume_after_max_wall_mid_row(self):
        world = world_for(tester_capacity=TESTER)
        before = len(self.bed.cloud.summarized)
        code, receipt, log = self.bed.run('release', 'rel-wall', '--max-wall', '50', world=world)
        self.assertEqual(code, 4, log)
        self.assertIsNone(receipt)
        first_leg = self.bed.cloud.summarized[before:]
        self.assertGreater(len(first_leg), 5, 'R1 finished, R5 started')
        self.assertLess(len(first_leg), 15)
        shutil.rmtree(self.bed.local('rel-wall'))
        code, receipt, log = self.bed.run('release', 'rel-wall', '--resume')
        self.check(code, receipt, log, 0, 'PASS')
        both = self.bed.cloud.summarized[before:]
        self.assertEqual(len(both), len(set(both)), 'a trial with a summary is not rerun')
        self.assertEqual(len(both), 15)
        for row in receipt['rows']:
            self.assertEqual(len(row['trials']), 5, row)

    def test_cost_covers_every_leg(self):
        world = world_for(tester_capacity=TESTER)
        code, receipt, log = self.bed.run('release', 'rel-cost', '--rows', 'R1', '--max-wall', '40', world=world)
        self.assertEqual(code, 4, log)
        state = json.loads(self.bed.cloud.path(self.bed.s3_run('rel-cost') + 'state.json').read_text())
        lane = state['lane']
        first = run_lane.cost({'lane': lane}, 0)['instance_hours']
        self.assertGreater(first['c8i.4xlarge'], 0.3)
        code, receipt, log = self.bed.run('release', 'rel-cost', '--rows', 'R1', '--resume')
        self.check(code, receipt, log, 0, 'PASS')
        hours = receipt['cost']['instance_hours']
        self.assertGreater(hours['c8i.4xlarge'], first['c8i.4xlarge'] + 0.05, 'the first leg is billed too')
        self.assertAlmostEqual(hours['c8in.24xlarge'], 2 * hours['c8i.4xlarge'], delta=0.01)

    def test_resume_reuses_a_kept_lane(self):
        world = world_for(tester_capacity=TESTER)
        code, receipt, log = self.bed.run('release', 'rel-keep', '--rows', 'R1', '--max-wall', '40', '--keep-lane',
                                          world=world)
        self.assertEqual(code, 4, log)
        kept = self.bed.alive()
        self.assertEqual(len(kept), 3, 'the kept lane stays up')
        launched = len(self.bed.cloud.launched)
        code, receipt, log = self.bed.run('release', 'rel-keep', '--rows', 'R1', '--resume')
        self.check(code, receipt, log, 0, 'PASS')
        self.assertEqual(len(self.bed.cloud.launched), launched, 'the live lane is reused, nothing launched')

    def run_instances_calls(self, run_id):
        return [list(c) for c in self.bed.cloud.calls if c[:2] == ('ec2', 'run-instances')
                and '"rvoip-run-id", "Value": "%s"' % run_id in opt(list(c), '--tag-specifications')]

    def test_az_override_moves_the_whole_lane(self):
        code, receipt, log = self.bed.run('smoke', 'smoke-az', '--rows', 'R1', '--az', 'us-west-2c', world=World())
        self.check(code, receipt, log, 0, 'PASS')
        self.assertEqual(receipt['environment']['availability_zone'], 'us-west-2c')
        calls = self.run_instances_calls('smoke-az')
        self.assertEqual(len(calls), 2)
        for call in calls:
            self.assertEqual(json.loads(opt(call, '--placement')),
                             {'AvailabilityZone': 'us-west-2c', 'GroupName': 'rvoip-bench'})
            enis = json.loads(opt(call, '--network-interfaces'))
            self.assertEqual([e['SubnetId'] for e in enis], ['subnet-cc'])
            self.assertEqual(enis[0]['Groups'], ['sg-bench'])
        caller = json.loads(opt(calls[1], '--network-interfaces'))[0]
        self.assertEqual(caller['SecondaryPrivateIpAddressCount'], 1, 'the callers keep their second source IP')

    def test_template_az_needs_no_override(self):
        self.bed.lane_config.write_text(json.dumps(dict(LANE_CONFIG, availability_zone='us-west-2b')))
        self.addCleanup(self.bed.lane_config.write_text, json.dumps(LANE_CONFIG))
        code, receipt, log = self.bed.run('smoke', 'smoke-az-b', '--rows', 'R1', '--az', 'us-west-2b', world=World())
        self.check(code, receipt, log, 0, 'PASS')
        calls = self.run_instances_calls('smoke-az-b')
        self.assertEqual(len(calls), 2)
        for call in calls:
            self.assertNotIn('--placement', call)
            self.assertNotIn('--network-interfaces', call)

    def pin_az(self, az):
        env_id = self.bed.env_id()
        (self.bed.baselines / env_id).mkdir(parents=True, exist_ok=True)
        (self.bed.baselines / env_id / 'env.json').write_text(json.dumps(
            {'schema': 'rvoip-rfc7502-env-v1', 'env_id': env_id, 'az': az}))
        self.addCleanup((self.bed.baselines / env_id / 'env.json').unlink, missing_ok=True)
        return env_id

    def pin_callers(self, count):
        env_id = self.bed.env_id()
        (self.bed.baselines / env_id).mkdir(parents=True, exist_ok=True)
        (self.bed.baselines / env_id / 'env.json').write_text(json.dumps(
            {'schema': 'rvoip-rfc7502-env-v1', 'env_id': env_id, 'caller_count': count}))
        self.addCleanup((self.bed.baselines / env_id / 'env.json').unlink, missing_ok=True)
        return env_id

    def test_pinned_caller_media_is_what_every_trial_runs(self):
        env_id = self.bed.env_id()
        (self.bed.baselines / env_id).mkdir(parents=True, exist_ok=True)
        (self.bed.baselines / env_id / 'env.json').write_text(json.dumps(
            {'schema': 'rvoip-rfc7502-env-v1', 'env_id': env_id, 'caller_media': {'worker_threads': 24, 'lanes': 4}}))
        self.addCleanup((self.bed.baselines / env_id / 'env.json').unlink, missing_ok=True)
        code, receipt, log = self.bed.run('smoke', 'smoke-media', '--rows', 'R2', '--env-id', env_id, world=World())
        self.check(code, receipt, log, 0, 'PASS')
        self.assertEqual(receipt['environment']['caller_media'], {'worker_threads': 24, 'lanes': 4})
        plans = [k for k in self.bed.cloud.keys(BUCKET, 'bench/smoke-media/') if k.endswith('/plan.json')]
        self.assertTrue(plans)
        for key in plans:
            plan = self.bed.cloud.get_json('s3://%s/%s' % (BUCKET, key))
            for config in plan['caller_configs']:
                self.assertEqual((config['media_worker_threads'], config['media_lanes']), (24, 4), key)

    def test_unpinned_environment_sends_no_media_layout(self):
        code, receipt, log = self.bed.run('smoke', 'smoke-nomedia', '--rows', 'R2', world=World())
        self.check(code, receipt, log, 0, 'PASS')
        self.assertIsNone(receipt['environment']['caller_media'])
        for key in [k for k in self.bed.cloud.keys(BUCKET, 'bench/smoke-nomedia/') if k.endswith('/plan.json')]:
            for config in self.bed.cloud.get_json('s3://%s/%s' % (BUCKET, key))['caller_configs']:
                self.assertNotIn('media_worker_threads', config)

    def test_pinned_caller_count_is_the_default(self):
        env_id = self.pin_callers(3)
        launched = len(self.bed.cloud.launched)
        code, receipt, log = self.bed.run('smoke', 'smoke-3c', '--rows', 'R1', '--env-id', env_id, world=World())
        self.check(code, receipt, log, 0, 'PASS')
        self.assertEqual(len(self.bed.cloud.launched) - launched, 4, '1 DUT + the 3 pinned callers')
        self.assertEqual(receipt['environment']['caller_count'], 3)

    def test_gated_run_with_another_caller_count_is_refused(self):
        env_id = self.pin_callers(3)
        launched = len(self.bed.cloud.launched)
        code, receipt, log = self.bed.run('baseline', 'base-2c', '--rows', 'R1', '--env-id', env_id, '--callers', '2',
                                          world=world_for(tester_capacity=TESTER))
        self.assertEqual(code, 2, log)
        self.assertIn('the env-id pins 3 callers', ' '.join(receipt['reasons']))
        self.assertEqual(len(self.bed.cloud.launched), launched)

    def test_smoke_may_override_the_pinned_caller_count(self):
        env_id = self.pin_callers(3)
        launched = len(self.bed.cloud.launched)
        code, receipt, log = self.bed.run('smoke', 'smoke-2c', '--rows', 'R1', '--env-id', env_id, '--callers', '2',
                                          world=World())
        self.check(code, receipt, log, 0, 'PASS')
        self.assertEqual(len(self.bed.cloud.launched) - launched, 3)

    def test_pinned_az_is_where_the_lane_runs(self):
        env_id = self.pin_az('us-west-2c')
        code, receipt, log = self.bed.run('smoke', 'smoke-pin', '--rows', 'R1', '--env-id', env_id, world=World())
        self.check(code, receipt, log, 0, 'PASS')
        self.assertEqual(receipt['environment']['availability_zone'], 'us-west-2c')

    def test_az_other_than_the_pinned_one_is_refused(self):
        env_id = self.pin_az('us-west-2c')
        launched = len(self.bed.cloud.launched)
        code, receipt, log = self.bed.run('smoke', 'smoke-pin-x', '--rows', 'R1', '--env-id', env_id,
                                          '--az', 'us-west-2a', world=World())
        self.assertEqual(code, 2, log)
        self.assertIn('the env-id pins us-west-2c', ' '.join(receipt['reasons']))
        self.assertEqual(len(self.bed.cloud.launched), launched, 'nothing launched')

    def test_resume_leg_stays_in_the_first_legs_az(self):
        world = world_for(tester_capacity=TESTER)
        code, receipt, log = self.bed.run('release', 'rel-az', '--rows', 'R1', '--max-wall', '40', '--az', 'us-west-2c',
                                          world=world)
        self.assertEqual(code, 4, log)
        code, receipt, log = self.bed.run('release', 'rel-az', '--rows', 'R1', '--resume', '--az', 'us-west-2a')
        self.assertEqual(code, 2, log)
        self.assertIn("this run's earlier leg ran in us-west-2c", ' '.join(receipt['reasons']))

    def test_resume_of_an_unknown_run_id_starts_fresh(self):
        code, receipt, log = self.bed.run('release', 'rel-new', '--resume', '--rows', 'R1',
                                          world=world_for(tester_capacity=TESTER))
        self.check(code, receipt, log, 0, 'PASS')
        self.assertIn('starting a new run', log)


class ReservationTests(LaneCase):
    """On-Demand Capacity Reservations: created before a leg launches, every launch targets them, cancelled on
    every exit path, and orphans of a crashed run swept at the next lane start."""

    def setUp(self):
        self.bed = Bed()

    def tearDown(self):
        self.bed.close()

    def creates(self):
        return [list(c) for c in self.bed.cloud.calls if c[:2] == ('ec2', 'create-capacity-reservation')]

    def launches(self, run_id=None):
        return [list(c) for c in self.bed.cloud.calls if c[:2] == ('ec2', 'run-instances')
                and (run_id is None or '"rvoip-run-id", "Value": "%s"' % run_id in opt(list(c), '--tag-specifications'))]

    def assert_all_cancelled(self, receipt):
        self.assertEqual(self.bed.cloud.held_reservations(), [], 'no reservation outlives the leg')
        held = receipt['environment']['capacity_reservations']
        self.assertTrue(held, 'the receipt records the reservations')
        for r in held:
            self.assertTrue(r['cancelled_at'], r)
        self.assertFalse(self.bed.cloud.path('s3://%s/bench/lock.json' % BUCKET).exists(), 'lock released')
        self.assertEqual(self.bed.alive(), [], 'lane torn down')

    def test_minor_reserves_the_lane_targets_every_launch_and_cancels(self):
        code, receipt, log = self.bed.run('minor', 'min-cr', '--rows', 'R1', '--r0', 'R1=3000', world=world_for(tester_capacity=TESTER))
        self.check(code, receipt, log, 0, 'PASS')
        reserved = {r['instance_type']: r for r in receipt['environment']['capacity_reservations']}
        self.assertEqual({t: r['count'] for t, r in reserved.items()}, {'c8i.4xlarge': 1, 'c8in.24xlarge': 2})
        for call in self.creates():
            self.assertEqual(opt(call, '--availability-zone'), 'us-west-2b')
            self.assertEqual(opt(call, '--placement-group-arn'), 'arn:aws:ec2:us-west-2:1:placement-group/rvoip-bench')
            tags = {t['Key']: t['Value'] for t in json.loads(opt(call, '--tag-specifications'))[0]['Tags']}
            self.assertEqual(tags['rvoip-run-id'], 'min-cr')
            self.assertEqual(opt(call, '--end-date'), tags['rvoip-expires-at'], 'ends by itself at the lane expiry')
        launches = self.launches()
        self.assertEqual(len(launches), 2)
        for call in launches:
            target = json.loads(opt(call, '--capacity-reservation-specification'))
            itype = opt(call, '--instance-type') or TEMPLATES[re.match(r'LaunchTemplateName=([^,]+)',
                                                                      opt(call, '--launch-template')).group(1)]
            self.assertEqual(target['CapacityReservationTarget']['CapacityReservationId'], reserved[itype]['id'])
        self.assert_all_cancelled(receipt)

    def test_same_type_lane_shares_one_reservation(self):
        code, receipt, log = self.bed.run('smoke', 'smoke-cr-one', '--rows', 'R1', '--reserve-capacity',
                                          '--dut-type', 'c8in.24xlarge', world=World())
        self.check(code, receipt, log, 0, 'PASS')
        self.assertEqual([(r['instance_type'], r['count']) for r in receipt['environment']['capacity_reservations']],
                         [('c8in.24xlarge', 3)])
        self.assert_all_cancelled(receipt)

    def test_smoke_and_diag_do_not_reserve_by_default(self):
        code, receipt, log = self.bed.run('smoke', 'smoke-nocr', '--rows', 'R1', world=World())
        self.check(code, receipt, log, 0, 'PASS')
        code, receipt, log = self.bed.run('diag', 'diag-nocr', '--smoke-seconds', '30', world=World())
        self.check(code, receipt, log, 0, 'PASS')
        self.assertEqual(self.creates(), [])
        for call in self.launches():
            self.assertNotIn('--capacity-reservation-specification', call)
        self.assertEqual(receipt['environment']['capacity_reservations'], [])

    def test_no_reserve_capacity_turns_minor_off(self):
        code, receipt, log = self.bed.run('minor', 'min-nocr', '--rows', 'R1', '--r0', 'R1=3000', '--no-reserve-capacity',
                                          world=world_for(tester_capacity=TESTER))
        self.check(code, receipt, log, 0, 'PASS')
        self.assertEqual(self.creates(), [])

    def test_cancelled_on_a_fail_verdict(self):
        code, receipt, log = self.bed.run('smoke', 'smoke-cr-fail', '--rows', 'R1', '--reserve-capacity',
                                          world=World(dut_capacity={'R1': 200}))
        self.check(code, receipt, log, 1, 'FAIL')
        self.assert_all_cancelled(receipt)

    def test_cancelled_on_infra(self):
        code, receipt, log = self.bed.run('smoke', 'smoke-cr-infra', '--rows', 'R1', '--reserve-capacity',
                                          world=World(dut_git_sha='feedfacecafe'))
        self.assertEqual(code, 2, log)
        self.assert_all_cancelled(receipt)

    def stop_at_max_wall(self, run_id):
        """Leg 1 of a minor run, stopped by --max-wall after its lane launched: (held ids, its state)."""
        code, receipt, log = self.bed.run('minor', run_id, '--rows', 'R1', '--r0', 'R1=3000', '--max-wall', '40',
                                          world=world_for(tester_capacity=TESTER))
        self.assertEqual(code, 4, log)
        self.assertEqual(self.bed.alive(), [], 'the lane is torn down at the stop')
        state = json.loads((self.bed.local(run_id) / 'state.json').read_text())
        return self.bed.cloud.held_reservations(), state, log

    def resume(self, run_id):
        return self.bed.run('minor', run_id, '--rows', 'R1', '--r0', 'R1=3000', '--resume')

    def test_max_wall_holds_the_reservations_until_two_hours_after_the_stop(self):
        held, state, log = self.stop_at_max_wall('min-hold')
        self.assertEqual(len(held), 2, 'both reservations survive exit 4')
        for r in state['reservations']:
            self.assertTrue(r['held_for_resume'], r)
            self.assertIsNone(r.get('cancelled_at'))
            self.assertEqual(run_lane.parse_utc(r['ends_at']) - run_lane.parse_utc(r['held_at']), 2 * 3600)
            self.assertLess(r['ends_at'], state['lane']['expires_at'])
            cloud = self.bed.cloud.reservations[r['id']]
            self.assertEqual(cloud['EndDate'], r['ends_at'], 'the reservation itself ends then')
            self.assertEqual(self.bed.cloud.tags_of(r['id'])['rvoip-held-for-resume'], 'true')
        self.assertIn('holding capacity reservation', log)

    def test_hold_never_outlives_the_lane_expiry(self):
        with mock.patch.object(run_lane, 'HOLD_SECONDS', 100 * 3600):
            held, state, log = self.stop_at_max_wall('min-hold-cap')
        for r in state['reservations']:
            self.assertEqual(r['ends_at'], state['lane']['expires_at'])

    def test_resumed_leg_adopts_the_held_reservations(self):
        held, state, log = self.stop_at_max_wall('min-adopt')
        creates = len(self.creates())
        code, receipt, log = self.resume('min-adopt')
        self.check(code, receipt, log, 0, 'PASS')
        self.assertEqual(len(self.creates()), creates, 'nothing new reserved')
        self.assertIn('adopting held capacity reservation', log)
        leg2 = self.launches('min-adopt')[2:]
        self.assertEqual(len(leg2), 2)
        for call in leg2:
            target = json.loads(opt(call, '--capacity-reservation-specification'))
            self.assertIn(target['CapacityReservationTarget']['CapacityReservationId'], held)
        lane2 = json.loads((self.bed.local('min-adopt') / 'state.json').read_text())['lane']
        for cid in held:
            self.assertEqual(self.bed.cloud.reservations[cid]['EndDate'], lane2['expires_at'],
                             'adopted: extended to the new leg expiry')
        entries = receipt['environment']['capacity_reservations']
        self.assertEqual(sorted(r['id'] for r in entries), held)
        self.assertTrue(all(r['held_for_resume'] for r in entries))
        self.assert_all_cancelled(receipt)

    def test_sweep_spares_only_the_held_reservations(self):
        held, state, log = self.stop_at_max_wall('min-sweep')
        orphan = self.bed.cloud.add_reservation('c8in.24xlarge', 2, {
            'rvoip-bench': 'true', 'rvoip-env-id': 'unpinned', 'rvoip-run-id': 'crashed'})
        stray = self.bed.cloud.add_reservation('c8in.24xlarge', 2, {
            'rvoip-bench': 'true', 'rvoip-env-id': 'unpinned', 'rvoip-run-id': 'min-sweep'})
        code, receipt, log = self.resume('min-sweep')
        self.check(code, receipt, log, 0, 'PASS')
        self.assertEqual(self.bed.cloud.reservations[orphan]['State'], 'cancelled')
        self.assertEqual(self.bed.cloud.reservations[stray]['State'], 'cancelled', 'not held: swept')
        for cid in held:
            self.assertIn('keeping capacity reservation %s held for this run' % cid, log)
        self.assertNotIn(stray, [r['id'] for r in receipt['environment']['capacity_reservations']])
        self.assert_all_cancelled(receipt)

    def test_a_fresh_run_with_the_same_run_id_sweeps_the_hold(self):
        held, state, log = self.stop_at_max_wall('min-norsm')
        code, receipt, log = self.bed.run('minor', 'min-norsm', '--rows', 'R1', '--r0', 'R1=3000')
        self.check(code, receipt, log, 0, 'PASS')
        for cid in held:
            self.assertEqual(self.bed.cloud.reservations[cid]['State'], 'cancelled')

    def test_expired_hold_falls_back_to_a_new_reservation(self):
        held, state, log = self.stop_at_max_wall('min-expired')
        for cid in held:
            self.bed.cloud.reservations[cid]['State'] = 'expired'
        creates = len(self.creates())
        code, receipt, log = self.resume('min-expired')
        self.check(code, receipt, log, 0, 'PASS')
        self.assertEqual(len(self.creates()), creates + 2, 'reserved again')
        new = set(self.bed.cloud.reservations) - set(held)
        for call in self.launches('min-expired')[2:]:
            target = json.loads(opt(call, '--capacity-reservation-specification'))
            self.assertIn(target['CapacityReservationTarget']['CapacityReservationId'], new)
        ended = [r for r in receipt['environment']['capacity_reservations'] if r['id'] in held]
        self.assertEqual(len(ended), 2)
        self.assertEqual(self.bed.cloud.held_reservations(), [])
        self.assertNotIn('WARNING cancel', log, 'an ended hold is not cancelled again')

    def test_expired_hold_and_no_capacity_is_infra(self):
        held, state, log = self.stop_at_max_wall('min-expired-short')
        for cid in held:
            self.bed.cloud.reservations[cid]['State'] = 'expired'
        self.bed.cloud.world.reserve_short = ('c8i.4xlarge', 'c8in.24xlarge')
        launched = len(self.bed.cloud.launched)
        code, receipt, log = self.resume('min-expired-short')
        self.assertEqual(code, 3, log)
        self.assertIn('reserve 1 x c8i.4xlarge', ' '.join(receipt['reasons']))
        self.assertEqual(len(self.bed.cloud.launched), launched, 'no unreserved fallback')

    def test_signal_during_a_leg_cancels_instead_of_holding(self):
        code, receipt, log = self.bed.run('minor', 'min-sig', '--rows', 'R1', '--r0', 'R1=3000', '--max-wall', '40',
                                          world=world_for(tester_capacity=TESTER,
                                                          signal_on=('caller-run', signal.SIGTERM)))
        self.assertEqual(code, 3, log)
        self.assertIn('interrupted by SIGTERM', ' '.join(receipt['reasons']))
        self.assert_all_cancelled(receipt)

    def check_signal(self, signum, name):
        code, receipt, log = self.bed.run('smoke', 'smoke-cr-%s' % name, '--rows', 'R1', '--reserve-capacity',
                                          world=World(signal_on=('caller-run', signum)))
        self.assertEqual(code, 3, log)
        self.assertEqual(receipt['status'], 'INFRA')
        self.assertIn('interrupted by %s' % name, ' '.join(receipt['reasons']))
        self.assert_all_cancelled(receipt)
        self.assertIs(signal.getsignal(signum), self.saved[signum], 'main() restores the handler')

    def test_cancelled_on_sigterm(self):
        self.saved = {signal.SIGTERM: signal.getsignal(signal.SIGTERM)}
        self.check_signal(signal.SIGTERM, 'SIGTERM')

    def test_cancelled_on_sigint(self):
        self.saved = {signal.SIGINT: signal.getsignal(signal.SIGINT)}
        self.check_signal(signal.SIGINT, 'SIGINT')

    def test_capacity_miss_on_reserve_is_infra_and_launches_nothing(self):
        code, receipt, log = self.bed.run('minor', 'min-cr-short', '--rows', 'R1', '--r0', 'R1=3000',
                                          world=world_for(tester_capacity=TESTER, reserve_short=('c8in.24xlarge',)))
        self.assertEqual(code, 3, log)
        self.assertEqual(receipt['status'], 'INFRA')
        self.assertIn('reserve 2 x c8in.24xlarge', ' '.join(receipt['reasons']))
        self.assertEqual(self.bed.cloud.launched, [], 'no unreserved fallback')
        self.assertEqual(self.bed.cloud.held_reservations(), [], 'the DUT reservation made first is cancelled')
        self.assertEqual(receipt['rows'], [])

    def test_orphans_of_this_env_id_are_swept_at_lane_start(self):
        env_id = self.bed.env_id()
        orphan = self.bed.cloud.add_reservation('c8in.24xlarge', 2, {'rvoip-bench': 'true', 'rvoip-env-id': env_id,
                                                                     'rvoip-run-id': 'crashed'})
        other_env = self.bed.cloud.add_reservation('c8in.24xlarge', 2, {'rvoip-bench': 'true',
                                                                        'rvoip-env-id': 'aws-other'})
        not_bench = self.bed.cloud.add_reservation('c8in.24xlarge', 2, {'rvoip-env-id': env_id})
        code, receipt, log = self.bed.run('smoke', 'smoke-sweep', '--rows', 'R1', '--env-id', env_id, world=World())
        self.check(code, receipt, log, 0, 'PASS')
        self.assertIn('cancelling orphaned capacity reservation %s (run crashed' % orphan, log)
        self.assertEqual(self.bed.cloud.reservations[orphan]['State'], 'cancelled')
        self.assertEqual(self.bed.cloud.held_reservations(), sorted([other_env, not_bench]))



class PlacementGroupTests(LaneCase):
    """A cluster placement group is bound to the AZ of any member still shutting down."""

    def setUp(self):
        self.bed = Bed()
        sleep = mock.patch('time.sleep', self.bed.clock.advance)    # waits advance the fake clock only
        sleep.start()
        self.addCleanup(sleep.stop)

    def tearDown(self):
        self.bed.close()

    def stranger(self, az, state='shutting-down', polls=3):
        iid = 'i-%017x' % (0xfff0 + len(self.bed.cloud.instances))
        self.bed.cloud.instances[iid] = {
            'InstanceId': iid, 'InstanceType': 'c8in.24xlarge', 'ImageId': AMI, 'State': {'Name': state},
            'Placement': {'AvailabilityZone': az, 'GroupName': 'rvoip-bench'}, 'polls_left': polls,
            'NetworkInterfaces': [], 'Tags': [], 'role': 'caller'}
        return iid

    def test_teardown_waits_until_its_instances_are_terminated(self):
        world = World(terminate_polls=3)
        code, receipt, log = self.bed.run('smoke', 'smoke-pg-wait', '--rows', 'R1', world=world)
        self.check(code, receipt, log, 0, 'PASS')
        self.assertEqual({i['State']['Name'] for i in self.bed.cloud.instances.values()}, {'terminated'})
        self.assertNotIn('not terminated after', log)

    def test_launch_waits_for_a_group_still_bound_elsewhere(self):
        self.stranger('us-west-2c', polls=4)
        code, receipt, log = self.bed.run('smoke', 'smoke-pg-other', '--rows', 'R1', '--az', 'us-west-2a',
                                          world=World())
        self.check(code, receipt, log, 0, 'PASS')
        self.assertIn('waiting for placement group rvoip-bench', log)
        self.assertEqual(receipt['environment']['availability_zone'], 'us-west-2a')

    def test_a_group_that_stays_bound_is_a_clear_infra(self):
        iid = self.stranger('us-west-2c', state='running')
        code, receipt, log = self.bed.run('smoke', 'smoke-pg-bound', '--rows', 'R1', '--az', 'us-west-2a',
                                          world=World())
        self.assertEqual(code, 3, log)
        self.assertIn('placement group rvoip-bench is bound to us-west-2c by %s (running)' % iid,
                      ' '.join(receipt['reasons']))
        self.assertEqual(self.bed.cloud.launched, [], 'nothing launched')

    def test_members_in_the_same_az_do_not_block(self):
        iid = self.stranger('us-west-2a', state='running')
        code, receipt, log = self.bed.run('smoke', 'smoke-pg-same', '--rows', 'R1', '--az', 'us-west-2a',
                                          world=World())
        self.assertEqual((code, receipt['status']), (0, 'PASS'), log)
        self.assertNotIn('waiting for placement group', log)
        self.assertEqual(self.bed.alive(), [iid], 'only the stranger is left running')



class LowerBoundTests(LaneCase):
    """A §6.1 baseline row that only passed is a lower bound, flagged wherever rates are read; a DUT search that
    stops the same way still fails."""

    def setUp(self):
        self.bed = Bed()

    def tearDown(self):
        self.bed.close()

    def report_text(self, receipt):
        return self.bed.cloud.path(receipt['evidence']['report']['rfc7502-report']['md']).read_text()

    def test_baseline_row_that_only_passed_records_a_lower_bound(self):
        code, receipt, log = self.bed.run('baseline', 'base-lb', '--rows', 'R6', '--r0', 'R6=2400', '--record',
                                          world=World())
        self.check(code, receipt, log, 0, 'PASS')
        row = self.rows(receipt)['R6']
        self.assertEqual(row['verdict'], 'pass')
        self.assertTrue(row['lower_bound'])
        self.assertFalse(row['tester_limited'])
        self.assertEqual(row['r'], row['search']['highest_passing_sps'])
        self.assertTrue(row['search']['stop_reason'].startswith('max_trials'), row['search'])
        self.assertEqual(len(row['trials']), profiles.PROFILES['baseline']['max_trials'])
        base = self.bed.rates()['baseline_sps']['R6']
        self.assertEqual((base['r'], base['lower_bound']), (row['r'], True))
        self.assertTrue(base['lower_bound_reason'].startswith('max_trials'))
        self.assertEqual(self.bed.rates()['tester_caps']['R6'], int(row['r'] * 0.67) // 4 * 4)
        text = self.report_text(receipt)
        self.assertIn('LOWER BOUND, not a measured ceiling', text)
        self.assertIn('R6: baseline >= %d sps, a lower bound' % row['r'], text)

    def test_measured_baseline_is_not_flagged(self):
        code, receipt, log = self.bed.run('baseline', 'base-ceiling', '--rows', 'R1', '--record',
                                          world=World(tester_capacity=TESTER))
        self.check(code, receipt, log, 0, 'PASS')
        self.assertNotIn('lower_bound', self.rows(receipt)['R1'])
        self.assertFalse(self.bed.rates()['baseline_sps']['R1']['lower_bound'])

    def test_minor_and_full_max_trials_stops_still_fail(self):
        """The gate profiles are unchanged: a DUT search out of trials has not converged (release confirms a
        recorded R and runs no search)."""
        for profile, run_id in (('minor', 'minor-mt'), ('full', 'full-mt')):
            code, receipt, log = self.bed.run(profile, run_id, '--rows', 'R1', '--r0', 'R1=3000',
                                              world=world_for({'R1': 10 ** 7}))
            self.check(code, receipt, log, 1, 'FAIL')
            row = self.rows(receipt)['R1']
            self.assertEqual(row['verdict'], 'fail', profile)
            self.assertIn('max_trials', row['reason'])
            self.assertNotIn('lower_bound', row)
            self.assertIsNone(row.get('r'))

    def test_gate_flags_a_cap_from_a_lower_bound_baseline(self):
        env_id = self.bed.env_id()
        (self.bed.baselines / env_id).mkdir(parents=True, exist_ok=True)
        (self.bed.baselines / env_id / 'rates.json').write_text(json.dumps({
            'schema': 'rvoip-rfc7502-rates-v1', 'env_id': env_id, 'rows': {},
            'tester_caps': {'R1': 6000, 'R5': 6000},
            'baseline_sps': {'R1': {'r': 9000, 'tester_limited': False, 'lower_bound': True, 'source_run': 's3://x/'},
                             'R5': {'r': 9000, 'tester_limited': True, 'lower_bound': False, 'source_run': 's3://x/'}}}))
        code, receipt, log = self.bed.run('minor', 'minor-lbcap', '--rows', 'R1,R5', '--record',
                                          world=world_for(tester_capacity=TESTER))
        self.check(code, receipt, log, 0, 'PASS')
        rows = self.rows(receipt)
        self.assertTrue(rows['R1']['tester_cap_lower_bound'])
        self.assertFalse(rows['R5']['tester_cap_lower_bound'])
        recorded = self.bed.rates()['rows']
        self.assertTrue(recorded['R1']['tester_cap_lower_bound'])
        self.assertFalse(recorded['R5']['tester_cap_lower_bound'])
        text = self.report_text(receipt)
        self.assertIn('LOWER BOUND, not a measured ceiling', text)


class RecordBaselineTests(unittest.TestCase):
    """record_baseline.py: a finished baseline run's rows into rates.json, without re-running anything."""

    S3_RUN = 's3://%s/bench/base-done/' % BUCKET

    def setUp(self):
        self.bed = Bed()
        self.env_id = self.bed.env_id()
        (self.bed.baselines / self.env_id).mkdir(parents=True)
        (self.bed.baselines / self.env_id / 'env.json').write_text(json.dumps(
            {'schema': 'rvoip-rfc7502-env-v1', 'env_id': self.env_id}))
        (self.bed.baselines / 'ENVIRONMENT').write_text(self.env_id + '\n')
        self.bed.cloud.put(self.S3_RUN + 'state.json', json.dumps({'final': {'status': 'FAIL'}}))

    def tearDown(self):
        self.bed.close()

    def receipt(self, **over):
        receipt = {
            'profile': 'baseline', 'status': 'FAIL', 'run_id': 'base-done', 'candidate_sha': CANDIDATE,
            'environment': {'env_id': self.env_id}, 'evidence': {'s3_prefix': self.S3_RUN},
            'rows': [
                {'id': 'R1', 'r': 14168, 'tester_limited': True, 'verdict': 'pass',
                 'search': {'result_sps': None, 'highest_passing_sps': 14168,
                            'stop_reason': 'halt: 3 consecutive invalid trials'}},
                {'id': 'R2', 'r': None, 'verdict': 'infra',
                 'search': {'result_sps': None, 'highest_passing_sps': None, 'stop_reason': 'invalid'}},
                {'id': 'R6', 'r': None, 'verdict': 'fail',
                 'search': {'result_sps': None, 'highest_passing_sps': 5128,
                            'stop_reason': 'max_trials (9) without convergence; highest passing rate 5128'}}]}
        receipt.update(over)
        path = self.bed.tmp / ('receipt-%d.json' % len(list(self.bed.tmp.glob('receipt-*'))))
        path.write_text(json.dumps(receipt))
        return path

    def record(self, receipt, *extra):
        out = io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(out):
            code = record_baseline.main(['--receipt', str(receipt), '--bucket', BUCKET] + list(extra))
        return code, out.getvalue()

    def test_records_measured_and_lower_bound_rows(self):
        code, out = self.record(self.receipt())
        self.assertEqual(code, 0, out)
        rates = self.bed.rates()
        self.assertEqual(sorted(rates['baseline_sps']), ['R1', 'R6'], 'R2 has no result')
        self.assertEqual((rates['baseline_sps']['R1']['r'], rates['baseline_sps']['R1']['lower_bound']), (14168, False))
        self.assertTrue(rates['baseline_sps']['R1']['tester_limited'])
        self.assertEqual((rates['baseline_sps']['R6']['r'], rates['baseline_sps']['R6']['lower_bound']), (5128, True))
        self.assertEqual(rates['tester_caps'], {'R1': 9492, 'R6': 3432})
        self.assertEqual(rates['baseline_sps']['R1']['source_run'], self.S3_RUN)
        self.assertTrue(self.bed.cloud.path(self.S3_RUN + 'record/%s/rates.json' % self.env_id).is_file())

    def test_selected_rows_only_and_merge(self):
        self.assertEqual(self.record(self.receipt(), '--rows', 'R6')[0], 0)
        self.assertEqual(sorted(self.bed.rates()['baseline_sps']), ['R6'])
        self.assertEqual(self.record(self.receipt(), '--rows', 'R1')[0], 0)
        self.assertEqual(sorted(self.bed.rates()['baseline_sps']), ['R1', 'R6'], 'merged, R6 kept')

    def test_refusals(self):
        cases = [
            (self.receipt(profile='minor'), (), 'only a baseline run'),
            (self.receipt(environment={'env_id': 'aws-other'}), (), 'ENVIRONMENT is'),
            (self.receipt(status='INFRA'), (), 'never finished'),
            (self.receipt(), ('--rows', 'R2'), 'no recordable result for R2'),
        ]
        for receipt, extra, why in cases:
            code, out = self.record(receipt, *extra)
            self.assertEqual(code, 2, out)
            self.assertIn(why, out)
        self.bed.cloud.put(self.S3_RUN + 'state.json', json.dumps({'rows': {}}))
        code, out = self.record(self.receipt())
        self.assertEqual(code, 2, out)
        self.assertIn('no final verdict', out)
        self.assertIsNone(self.bed.rates(), 'nothing recorded by a refusal')


if __name__ == '__main__':
    unittest.main()
