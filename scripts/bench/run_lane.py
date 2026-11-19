#!/usr/bin/env python3
"""rvoip RFC 7501/7502 bench lane orchestrator (the `bench-lane` gate).

  run_lane.py --profile release|minor|full|baseline --candidate <sha> --receipt <path>
              --dut-bin <rvoip-bench-uas> --caller-bin <rvoip-loadgen> [--ea-uas-bin <ea-uas>]
              [--run-id ID] [--resume] [--max-wall MIN] [--dry-run] [--record]

One lane = one DUT host + N caller hosts launched from the infra/aws/bench
launch templates, used for one profile, then terminated. Every trial is a
fresh DUT process: start, ready.json, S3 start barrier for the callers,
generate, drain (finish file), collect evidence with sha256, merge, score.

Contract: docs/BENCH_GATE_INTERFACE.md (CLI, exit codes, receipt).
Design:   crates/sip/rvoip-sip/docs/RFC7502_BENCHMARK.md.

Exit codes: 0 PASS/WARN, 1 FAIL, 2 usage, 3 INFRA, 4 stopped at --max-wall.
Standard library + the AWS CLI v2 + openssl.
"""
import argparse
import datetime
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import threading
import time
import traceback

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
sys.path.insert(0, str(HERE))
from aws import Aws, AwsError  # noqa: E402
import caller_policy  # noqa: E402
import evidence  # noqa: E402
import profiles  # noqa: E402
import rfc7502  # noqa: E402
import score  # noqa: E402
import search  # noqa: E402

RECEIPT_SCHEMA = 'rvoip-bench-lane-receipt-v1'
BASELINES = REPO / 'crates/sip/rvoip-sip/perf-baselines/rfc7502'
EXIT_PASS, EXIT_FAIL, EXIT_USAGE, EXIT_INFRA, EXIT_WALL = 0, 1, 2, 3, 4
DUT_SIP_PORT = {'tls': 5061, 'udp': 5060}
CALLER_CONNECTIONS = 8
METRICS_PORT = 9464
WORKDIR = Path(os.environ.get('RVOIP_BENCH_WORKDIR', Path.home() / '.cache/rvoip-bench'))


class Infra(RuntimeError):
    """Exit 3: retry later."""


class Usage(RuntimeError):
    """Exit 2: bad input."""


class WallClock(RuntimeError):
    """Exit 4: --max-wall reached; resume later."""


class Interrupted(Infra):
    """SIGINT/SIGTERM: unwind through the normal exit path (teardown, reservations, lock, receipt)."""


# Set once the exit path starts: a signal then no longer interrupts the cleanup.
SIGNALS = {'unwinding': False}


# How long a lane waits for a cluster placement group to let go of another AZ (a previous lane's
# instances still shutting down), and how long teardown waits for its own instances to terminate.
PLACEMENT_WAIT_SECONDS = 300


def parse_utc(text):
    return datetime.datetime.strptime(text, '%Y-%m-%dT%H:%M:%SZ').replace(tzinfo=datetime.timezone.utc).timestamp()


# A reservation held across a --max-wall stop ends by itself this long after the stop at the latest.
HOLD_SECONDS = 2 * 3600


def utc(ts=None):
    return datetime.datetime.fromtimestamp(ts or time.time(), datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')


def stamp():
    return datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ')


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for block in iter(lambda: f.read(1 << 20), b''):
            h.update(block)
    return h.hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':')).encode()


class Log:
    def __init__(self, path):
        self.path = path
        path.parent.mkdir(parents=True, exist_ok=True)

    def __call__(self, *args):
        line = '%s %s' % (utc(), ' '.join(str(a) for a in args))
        print(line, flush=True)
        with self.path.open('a') as f:
            f.write(line + '\n')


# ---------------------------------------------------------------------------
# Environment and recorded rates
# ---------------------------------------------------------------------------

def env_id_for(lane, caller_sha):
    """aws-<region>-<dut>-<n>x<caller>-<ami8>-<caller8>, from the launch templates' types
    (a baseline lane swaps the DUT host for a larger ea-uas host but keeps the env-id)."""
    region = lane['region'].replace('us-west-', 'usw')
    dut = lane['tf']['dut_instance_type'].replace('.', '').replace('xlarge', 'xl')
    caller = lane['tf']['caller_instance_type'].replace('.', '').replace('xlarge', 'xl')
    return 'aws-%s-%s-%dx%s-%s-%s' % (region, dut, len(lane['callers']), caller,
                                       lane['ami_id'].replace('ami-', '')[:8], caller_sha[:8])


def load_rates(env_id):
    path = BASELINES / env_id / 'rates.json'
    if not path.exists():
        return None, path
    rates = json.loads(path.read_text())
    assert rates.get('schema') == 'rvoip-rfc7502-rates-v1', path
    return rates, path


# ---------------------------------------------------------------------------
# TLS material: a throwaway CA per lane run, the DUT and caller certificates
# carry the lane's private IPs as SANs.
# ---------------------------------------------------------------------------

def make_tls(out, dut_ips, caller_ips):
    out.mkdir(parents=True, exist_ok=True)

    def ossl(*args):
        subprocess.run(['openssl', *args], check=True, capture_output=True)

    ossl('req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '7', '-subj', '/CN=rvoip-bench-ca',
         '-keyout', str(out / 'ca.key'), '-out', str(out / 'ca.crt'))
    for name, ips in (('dut', dut_ips), ('caller', caller_ips)):
        san = ','.join(['IP:%s' % ip for ip in ips] + ['DNS:rvoip-bench-%s' % name])
        ext = out / (name + '.ext')
        ext.write_text('subjectAltName=%s\nextendedKeyUsage=serverAuth,clientAuth\n' % san)
        ossl('req', '-newkey', 'rsa:2048', '-nodes', '-subj', '/CN=rvoip-bench-%s' % name,
             '-keyout', str(out / (name + '.key')), '-out', str(out / (name + '.csr')))
        ossl('x509', '-req', '-in', str(out / (name + '.csr')), '-CA', str(out / 'ca.crt'), '-CAkey', str(out / 'ca.key'),
             '-CAcreateserial', '-days', '7', '-extfile', str(ext), '-out', str(out / (name + '.crt')))
    for leftover in out.glob('*.csr'):
        leftover.unlink()
    (out / 'ca.key').unlink()


# ---------------------------------------------------------------------------
# The lane
# ---------------------------------------------------------------------------

class Lane:
    def __init__(self, args, aws, log, state):
        self.args, self.aws, self.log, self.state = args, aws, log, state

    @property
    def info(self):
        return self.state['lane']

    def tf_outputs(self):
        if self.args.lane_config:
            return json.loads(Path(self.args.lane_config).read_text())
        try:
            out = subprocess.run(['terraform', '-chdir=%s' % (REPO / 'infra/aws/bench'), 'output', '-json'],
                                 capture_output=True, text=True)
        except FileNotFoundError:
            raise Usage('terraform is not installed (pass --lane-config)')
        if out.returncode:
            raise Usage('terraform output failed (pass --lane-config): %s' % out.stderr.strip()[-500:])
        return {k: v['value'] for k, v in json.loads(out.stdout).items()}

    def preflight(self, tf):
        # The baseline swaps the DUT for a larger ea-uas host (at most 32 vCPU more).
        need = tf['lane_vcpus'] + (32 if self.ea_uas else 0)
        quota = self.aws.vcpu_quota()
        used = self.aws.running_standard_vcpus()
        self.log('quota: %d vCPU, in use %d, lane needs %d' % (quota, used, need))
        if used + need > quota:
            raise Infra('vCPU quota: %d in use + %d for the lane > %d' % (used, need, quota))

    def lane_placement(self, tf):
        """(placement group, AZ) the lane launches into: --az (or the pinned AZ), else the templates'."""
        placement = self.aws.template_data(tf['dut_launch_template']).get('Placement') or {}
        return placement.get('GroupName'), (self.args.az or tf.get('availability_zone')
                                            or placement.get('AvailabilityZone'))

    def wait_for_placement_group(self, tf):
        """A cluster placement group is bound to the AZ of its members until the last one is terminated, so
        a launch into another AZ right after a lane (or a fallback to another AZ) is refused by EC2. Wait for
        members elsewhere to finish terminating; fail INFRA, naming them, when they do not."""
        group, az = self.lane_placement(tf)
        if not group or not az:
            return
        deadline = time.time() + PLACEMENT_WAIT_SECONDS
        while True:
            elsewhere = [m for m in self.aws.placement_group_members(group) if m[1] != az]
            if not elsewhere:
                return
            if time.time() >= deadline:
                raise Infra('placement group %s is bound to %s by %s; launch in %s when they have terminated' % (
                    group, elsewhere[0][1], ', '.join('%s (%s)' % (m[0], m[2]) for m in elsewhere), az))
            self.log('waiting for placement group %s: %s still in %s' % (
                group, ', '.join('%s %s' % (m[0], m[2]) for m in elsewhere), elsewhere[0][1]))
            time.sleep(15)

    def launch(self, budget_hours):
        tf = self.tf_outputs()
        self.preflight(tf)
        self.wait_for_placement_group(tf)
        expires = utc(time.time() + (budget_hours + 1) * 3600)
        base = {'rvoip-bench': 'true', 'managed-by': 'github-actions', 'rvoip-run-id': self.args.run_id,
                'rvoip-expires-at': expires}
        ids = {}
        moved = {}
        targets = self.reserve(tf, expires)
        if self.args.az and self.args.az != tf.get('availability_zone'):
            # Launch templates name one subnet; --az moves both templates to the same-named public subnet of
            # another AZ (no Terraform change). The whole lane shares the AZ, and so does every later leg.
            self.log('launching in %s (the templates name %s)' % (self.args.az, tf.get('availability_zone')))
            for key in ('dut_launch_template', 'caller_launch_template'):
                moved[key] = self.aws.template_in_az(tf[key], self.args.az)
        try:
            ids['dut'] = self.aws.run_from_template(
                tf['dut_launch_template'], 1, dict(base, Name='rvoip-bench-dut', **{'rvoip-role': 'dut'}),
                instance_type=self.args.ea_host_type if self.ea_uas else self.args.dut_type,
                capacity_reservation=targets.get('dut'), **moved.get('dut_launch_template', {}))
            ids['callers'] = self.aws.run_from_template(
                tf['caller_launch_template'], self.args.callers,
                dict(base, Name='rvoip-bench-caller', **{'rvoip-role': 'caller'}),
                instance_type=self.args.caller_type, capacity_reservation=targets.get('callers'),
                **moved.get('caller_launch_template', {}))
        except AwsError as error:
            self.aws.terminate(ids.get('dut', []) + ids.get('callers', []))
            if 'InsufficientInstanceCapacity' in str(error) or 'VcpuLimitExceeded' in str(error):
                raise Infra('launch: %s' % error)
            raise
        every = ids['dut'] + ids['callers']
        if (self.state.get('lane') or {}).get('launched_at'):   # an earlier leg's lane: still billed in cost()
            self.state.setdefault('earlier_lanes', []).append(
                {k: self.state['lane'].get(k) for k in ('launched_at', 'terminated_at', 'dut_type', 'callers')})
        self.state['lane'] = {'instances': every, 'region': self.args.region, 'launched_at': utc(),
                              'expires_at': expires, 'tf': tf}
        self.save()
        self.log('launched', every)
        described = self.aws.wait_running(every)
        self.aws.wait_ssm_online(every)
        by_id = {i['InstanceId']: i for i in described}

        def ips(iid):
            eni = by_id[iid]['NetworkInterfaces'][0]
            addrs = sorted(eni['PrivateIpAddresses'], key=lambda a: not a['Primary'])
            return [a['PrivateIpAddress'] for a in addrs]

        dut = ids['dut'][0]
        self.state['lane'].update({
            'dut': {'id': dut, 'ip': ips(dut)[0], 'type': by_id[dut]['InstanceType']},
            'callers': [{'id': c, 'ips': ips(c), 'type': by_id[c]['InstanceType']} for c in ids['callers']],
            'ami_id': by_id[dut]['ImageId'], 'az': by_id[dut]['Placement']['AvailabilityZone'],
            'placement_group': by_id[dut]['Placement'].get('GroupName'),
            'dut_type': by_id[dut]['InstanceType'], 'caller_type': by_id[ids['callers'][0]]['InstanceType'],
        })
        if self.args.az and self.state['lane']['az'] != self.args.az:
            raise Infra('lane launched in %s, not the requested %s' % (self.state['lane']['az'], self.args.az))
        for c in self.state['lane']['callers']:
            if len(c['ips']) < 2:
                raise Infra('caller %s has %d source IPs; need >= 2' % (c['id'], len(c['ips'])))
        self.save()

    @property
    def ea_uas(self):
        return bool(profiles.PROFILES[self.args.profile].get('ea_uas'))

    def alive(self):
        if not self.state.get('lane', {}).get('instances'):
            return False
        try:
            described = self.aws.describe_instances(self.info['instances'])
        except AwsError:
            return False
        return all(i['State']['Name'] == 'running' for i in described)

    def install(self, s3_run):
        lane = self.info
        tls = self.args.local / 'tls'
        if lane.get('tls_for') != lane['instances']:
            shutil.rmtree(tls, ignore_errors=True)
            lane['tls_for'] = lane['instances']
            make_tls(tls, [lane['dut']['ip']], [ip for c in lane['callers'] for ip in c['ips']])
        for f in tls.iterdir():
            self.aws.s3_cp(f, s3_run + 'tls/' + f.name)
        for path in sorted(HERE.glob('*.py')):
            self.aws.s3_cp(path, s3_run + 'py/' + path.name)
        self.aws.s3_cp(HERE / 'agent/bench-agent.sh', s3_run + 'agent/bench-agent.sh')
        for role, path in self.binaries().items():
            self.aws.s3_cp(path, s3_run + 'bin/' + role)
            sha = Path(self.args.local / ('%s.sha256' % role))
            sha.write_text('%s  %s\n' % (sha256_file(path), role))
            self.aws.s3_cp(sha, s3_run + 'bin/%s.sha256' % role)
        results = self.aws.run(lane['instances'], self.agent(s3_run, 'install', s3_run), 'rvoip bench install', 900)
        facts = {}
        for iid, inv in results.items():
            facts[iid] = json.loads(inv['StandardOutputContent'].strip().splitlines()[-1])
        lane['facts'] = facts
        for caller in lane['callers']:
            n = facts.get(caller['id'], {}).get('nproc')
            if n:
                caller['logical_cpus'], caller['physical_cores'] = n, n // 2
        for iid, f in facts.items():
            tuning = f.get('tuning') or {}
            if tuning.get('errors'):
                self.log('WARNING host tuning errors on', iid, tuning['errors'])
            if not tuning:
                self.log('WARNING no tuning record on', iid, '(stock AMI?)')
        self.save()

    def binaries(self):
        b = {'rvoip-loadgen': Path(self.args.caller_bin)}
        if self.ea_uas:
            b['ea-uas'] = Path(self.args.ea_uas_bin)
        else:
            b['rvoip-bench-uas'] = Path(self.args.dut_bin)
        return b

    def agent(self, s3_run, *argv):
        quoted = ' '.join("'%s'" % a for a in argv)
        return '\n'.join([
            'set -euo pipefail', 'export PATH=$PATH:/snap/bin', 'mkdir -p /opt/rvoip-bench',
            'aws s3 cp --only-show-errors %sagent/bench-agent.sh /opt/rvoip-bench/bench-agent.sh' % s3_run,
            'METRICS_PORT=%d bash /opt/rvoip-bench/bench-agent.sh %s' % (METRICS_PORT, quoted)])

    # ---- On-Demand Capacity Reservations
    # minor/full hold one targeted reservation per instance type for the lane (1 DUT + N callers) in the
    # lane's AZ and cluster placement group, so a long run cannot lose its hosts to a capacity miss. A leg
    # creates them before it launches (a capacity miss here is the same INFRA as a launch miss: nothing is
    # recorded yet), launches only into them (never unreserved), and cancels them on every exit path but
    # one: a --max-wall stop (exit 4) holds them for the next leg, ending by themselves at the earlier of
    # the stop + HOLD_SECONDS and the lane expiry; the resumed leg adopts them (same run-id and AZ) or
    # reserves again when they have ended. Each also ends by itself at the lane's rvoip-expires-at, so a
    # crashed orchestrator stops billing then at the latest; the next lane on the env-id cancels it sooner
    # (sweep_reservations).

    @property
    def env_tag(self):
        return self.args.env_tag or 'unpinned'

    def reserve(self, tf, expires):
        """Role -> reservation id for this leg's launch ({} when --no-reserve-capacity)."""
        if not self.args.reserve_capacity:
            return {}
        dut_template = self.aws.template_data(tf['dut_launch_template'])
        placement = dut_template.get('Placement') or {}
        az = self.args.az or tf.get('availability_zone') or placement.get('AvailabilityZone')
        group = placement.get('GroupName')
        group_arn = self.aws.placement_group_arn(group) if group else None
        need = {'dut': (self.args.ea_host_type if self.ea_uas else self.args.dut_type) or tf['dut_instance_type'],
                'callers': self.args.caller_type or tf['caller_instance_type']}
        counts = {}
        for role, itype in need.items():
            counts[itype] = counts.get(itype, 0) + (1 if role == 'dut' else self.args.callers)
        reservations = self.state.setdefault('reservations', [])
        usable = self.adoptable(az, counts, expires)
        by_type = {}
        for itype, count in counts.items():
            if itype in usable:
                by_type[itype] = usable[itype]
                continue
            tags = {'rvoip-bench': 'true', 'Name': 'rvoip-bench-lane', 'rvoip-run-id': self.args.run_id,
                    'rvoip-env-id': self.env_tag, 'rvoip-expires-at': expires}
            token = '%s-%d' % (self.args.run_id[:56], len(reservations))
            try:
                cid = self.aws.create_capacity_reservation(itype, az, count, expires, tags, token, group_arn)
            except AwsError as error:
                if 'InsufficientInstanceCapacity' in str(error) or 'ReservationCapacityExceeded' in str(error):
                    raise Infra('reserve %d x %s in %s: %s' % (count, itype, az, error))
                raise
            reservations.append({'id': cid, 'instance_type': itype, 'count': count, 'az': az,
                                 'placement_group': group, 'created_at': utc(), 'ends_at': expires})
            self.save()
            self.log('reserved %d x %s in %s: %s (ends %s)' % (count, itype, az, cid, expires))
            by_type[itype] = cid
        self.aws.wait_capacity_reservations(sorted(set(by_type.values())))
        return {role: by_type[itype] for role, itype in need.items()}

    @staticmethod
    def live(r):
        return not r.get('cancelled_at') and not r.get('ended_state')

    def adoptable(self, az, counts, expires):
        """instance type -> id of an active reservation of this run in `az` with room for the lane (a hold
        from the previous leg), its end moved back to this leg's expiry. Reservations of this run that have
        ended are marked so; none is cancelled here."""
        reservations = self.state.setdefault('reservations', [])
        found = {r['CapacityReservationId']: r for r in self.aws.describe_capacity_reservations(filters=(
            ('tag:rvoip-bench', 'true'), ('tag:rvoip-run-id', self.args.run_id)))}
        for r in reservations:
            if self.live(r) and (found.get(r['id']) or {}).get('State') != 'active':
                r['ended_state'] = (found.get(r['id']) or {}).get('State', 'gone')
                self.log('capacity reservation %s has ended (%s): reserving again' % (r['id'], r['ended_state']))
        usable = {}
        for cid, f in sorted(found.items()):
            itype = f['InstanceType']
            if (f['State'] != 'active' or f['AvailabilityZone'] != az or itype not in counts or itype in usable
                    or f.get('AvailableInstanceCount', 0) < counts[itype]):
                continue
            try:    # the hold ends soon after the stop: this leg needs it until its own expiry
                self.aws.end_capacity_reservation_at(cid, expires)
                self.aws.tag(cid, {'rvoip-held-for-resume': 'false'})
            except AwsError as error:
                self.log('WARNING could not adopt %s; reserving again:' % cid, error)
                continue
            usable[itype] = cid
            entry = next((r for r in reservations if r['id'] == cid), None)
            if entry is None:    # held by a leg whose state was lost: adopt by tag
                entry = {'id': cid, 'instance_type': itype, 'count': f.get('TotalInstanceCount'), 'az': az,
                         'created_at': None, 'ends_at': None}
                reservations.append(entry)
            entry.update(adopted_at=utc(), ends_at=expires)
            self.log('adopting held capacity reservation %s (%s x %s in %s)' % (
                cid, f.get('TotalInstanceCount'), itype, az))
        self.save()
        return usable

    def hold_reservations(self):
        """--max-wall stop: keep this run's reservations for the next leg, ending by themselves at the earlier
        of now + HOLD_SECONDS and the lane expiry. One that cannot be shortened is cancelled instead."""
        held = [r for r in self.state.get('reservations', []) if self.live(r)]
        expires = (self.state.get('lane') or {}).get('expires_at')
        for r in held:
            end = time.time() + HOLD_SECONDS
            for bound in (r.get('ends_at'), expires):
                if bound:
                    end = min(end, parse_utc(bound))
            if end <= time.time() + 600:
                self.cancel_one(r)
                continue
            try:
                self.aws.end_capacity_reservation_at(r['id'], utc(end))
                self.aws.tag(r['id'], {'rvoip-held-for-resume': 'true'})
            except AwsError as error:
                self.log('WARNING could not hold %s for the next leg:' % r['id'], error)
                self.cancel_one(r)
                continue
            r.update(held_for_resume=True, held_at=utc(), ends_at=utc(end))
            self.log('holding capacity reservation %s for the next leg until %s' % (r['id'], r['ends_at']))
        if held:
            self.save()

    def cancel_one(self, r):
        try:
            self.aws.cancel_capacity_reservation(r['id'])
            r['cancelled_at'] = utc()
            self.log('cancelled capacity reservation', r['id'])
        except AwsError as error:
            self.log('WARNING cancel %s failed; it ends by itself at %s:' % (r['id'], r.get('ends_at')), error)

    def release_reservations(self):
        """Cancel every reservation this run still holds (every exit path but a --max-wall stop)."""
        held = [r for r in self.state.get('reservations', []) if self.live(r)]
        for r in held:
            self.cancel_one(r)
        if held:
            self.save()

    def sweep_reservations(self):
        """Cancel bench reservations tagged for this env-id that no live leg holds (a crashed leg's). A resumed
        run spares exactly what its previous leg held for it: its own run-id, tagged held-for-resume."""
        keep = set()
        if self.args.resume:
            keep = {r['id'] for r in self.state.get('reservations', []) if self.live(r) and r.get('held_for_resume')}
        found = self.aws.describe_capacity_reservations(filters=(
            ('tag:rvoip-bench', 'true'), ('tag:rvoip-env-id', self.env_tag), ('state', 'active,pending')))
        for r in found:
            cid = r['CapacityReservationId']
            tags = {t['Key']: t['Value'] for t in r.get('Tags', [])}
            if cid in keep or (self.args.resume and tags.get('rvoip-run-id') == self.args.run_id
                               and tags.get('rvoip-held-for-resume') == 'true'):
                self.log('keeping capacity reservation %s held for this run' % cid)
                continue
            self.log('cancelling orphaned capacity reservation %s (run %s, %s x %s)' % (
                cid, tags.get('rvoip-run-id'), r.get('TotalInstanceCount'), r.get('InstanceType')))
            self.aws.cancel_capacity_reservation(cid)
            for mine in self.state.get('reservations', []):
                if mine['id'] == cid:
                    mine['cancelled_at'] = utc()
        self.save()

    def teardown(self, wait=True):
        """Terminate the lane; with `wait`, until EC2 reports every instance terminated, so the placement
        group is free for the next lane in any AZ (a signal skips the wait: the process is being stopped)."""
        ids = self.state.get('lane', {}).get('instances', [])
        if ids and not self.args.keep_lane:
            self.log('terminating', ids)
            self.aws.terminate(ids)
            self.state['lane']['terminated_at'] = utc()
            self.save()
            if wait and not self.aws.wait_terminated(ids, timeout=PLACEMENT_WAIT_SECONDS):
                self.log('WARNING lane instances not terminated after %d s: %s' % (PLACEMENT_WAIT_SECONDS, ids))

    def save(self):
        self.args.save_state(self.state)


# ---------------------------------------------------------------------------
# Trials
# ---------------------------------------------------------------------------

class Trials:
    """One trial = fresh DUT process, barrier, load, drain, evidence, remote summary, verdict."""

    def __init__(self, args, aws, lane, log, s3_run):
        self.args, self.aws, self.lane, self.log, self.s3_run = args, aws, lane, log, s3_run

    def dut_config(self, row, admission):
        lane = self.lane.info
        transport = row['transport']
        if self.lane.ea_uas:
            return score.ea_uas_config(row, lane['dut']['ip'], DUT_SIP_PORT[transport])
        return {
            'run_dir': '/var/lib/rvoip-bench/run',
            'transport': transport,
            'bind': '%s:%d' % (lane['dut']['ip'], DUT_SIP_PORT[transport]),
            'advertise_ip': lane['dut']['ip'],
            'tls_cert': '/opt/rvoip-bench/tls/dut.crt',
            'tls_key': '/opt/rvoip-bench/tls/dut.key',
            'media': row['media'],
            'rtcp_mux': True,
            'rtp_port_range': [10000, 64999],
            'admission': admission or {'mode': 'off'},
            'max_concurrent_calls': 200000,
            'metrics_listen': '0.0.0.0:%d' % METRICS_PORT,
            'stats_interval_ms': 1000,
            'drain_timeout_secs': 120,
        }

    def caller_configs(self, row, run_id, profile):
        """One config per caller host; host i runs rfc7502.host_shares(...)[i] of every stage."""
        lane = self.lane.info
        hosts = len(lane['callers'])
        rfc7502.validate_shares(profile, hosts)
        configs = []
        for i, caller in enumerate(lane['callers']):
            stages = [rfc7502.host_shares(s['cps'], s['seconds'], hosts)[i] for s in profile['stages']]
            configs.append(score.caller_config(
                row=row, run_id='%s-h%d' % (run_id, i + 1), sources=caller['ips'], connections=CALLER_CONNECTIONS,
                dut_ip=lane['dut']['ip'], dut_port=DUT_SIP_PORT[row['transport']],
                stages=[{'cps': s['cps'], 'seconds': s['seconds']} for s in stages],
                output='/var/lib/rvoip-bench/caller-%d' % (i + 1), media=self.args.caller_media))
        return configs

    def plan(self, row_id, row, rate, index, profile, rel, admission=None):
        """The trial's plan.json: what summarize_trial scores against, with the DUT and caller configs."""
        lane = self.lane.info
        run_id = re.sub(r'[^A-Za-z0-9_.-]', '-', '%s-%s' % (self.args.run_id[-24:], rel))[-80:]
        return {'row': row_id, 'rate': rate, 'index': index, 'criterion': 'rfc', 'media': row['media'],
                'transport': row['transport'], 'hold_ms': row['hold_ms'],
                'dut_kind': 'ea-uas' if self.lane.ea_uas else 'dut', 'profile': profile, 'admission': admission,
                'caller_hosts': {'caller-%d' % (i + 1): {'physical_cores': c.get('physical_cores', 48),
                                                         'instance_id': c['id'], 'sources': c['ips']}
                                 for i, c in enumerate(lane['callers'])},
                'dut_config': self.dut_config(row, admission),
                'caller_configs': self.caller_configs(row, run_id, profile)}

    def run(self, row_id, row, rate, index, profile, admission=None, rel=None):
        """Run one trial. Returns (summary or None, trial_rel, local_dir, note)."""
        rel = rel or evidence.trial_path(row_id, index, rate)
        trial_prefix = self.s3_run + rel + '/'
        local = self.args.local / rel
        cached = local / 'merged' / 'summary.json'
        if cached.exists():
            reused = json.loads(cached.read_text())
            if rfc7502.evaluate_trial(reused, 'rfc')[0]:
                return reused, rel, local, None   # reused on --resume
            # A void trial (no scored attempt) is never reused: a retry at the same rate reruns it.
        if local.exists():
            shutil.rmtree(local)
        local.mkdir(parents=True)
        self.args.check_wall(sum(st['seconds'] for st in profile['stages']) + row['hold_ms'] / 1000 + 600)
        lane = self.lane.info
        callers = [c['id'] for c in lane['callers']]
        dut = lane['dut']['id']
        dut_bin = 'ea-uas' if self.lane.ea_uas else 'rvoip-bench-uas'
        plan = self.plan(row_id, row, rate, index, profile, rel, admission)
        (local / 'plan.json').write_text(json.dumps(plan, indent=1, sort_keys=True))
        self.aws.s3_cp(local / 'plan.json', trial_prefix + 'plan.json')
        self.aws.s3_put_json(trial_prefix + 'dut.json', plan['dut_config'])
        for i, config in enumerate(plan['caller_configs']):
            self.aws.s3_put_json(trial_prefix + 'caller-%d.json' % (i + 1), config)
        env = caller_policy.policy_for(lane['callers'][0].get('logical_cpus', 96))
        env_file = local / 'caller-env.sh'
        env_file.write_text(''.join('export %s=%s\n' % kv for kv in sorted(env.items())))
        self.aws.s3_cp(env_file, trial_prefix + 'caller-env.sh')
        self.log('trial', rel, 'stages', [(s['name'], s['cps'], s['seconds']) for s in profile['stages']])
        # 1. Fresh DUT process.
        try:
            ready = self.aws.run([dut], self.lane.agent(self.s3_run, 'dut-start', trial_prefix, dut_bin),
                                 'rvoip bench dut-start', 180)
            out = ready[dut]['StandardOutputContent']
            ready_line = json.dumps(json.loads(out[out.index('{'):out.rindex('}') + 1]))   # ready.json may be pretty-printed
            (local / 'ready.json').write_text(ready_line + '\n')
        except AwsError as error:
            self.finish_dut(trial_prefix, dut_bin)
            return None, rel, local, 'DUT did not start: %s' % str(error)[-400:]
        sha = (json.loads(ready_line).get('git_sha') or '') if not self.lane.ea_uas else None
        if sha is not None and not self.args.candidate.startswith(sha[:12] or '-'):
            self.finish_dut(trial_prefix, dut_bin)
            raise Usage('the DUT binary is built at %r, not the candidate %s' % (sha, self.args.candidate))
        # 2. Callers, behind the S3 start barrier (one SSM command per caller: each has its own index).
        total = sum(s['seconds'] for s in profile['stages'])
        caller_timeout = int(total + row['hold_ms'] / 1000 + 900)
        commands = {cid: self.aws.send([cid], self.lane.agent(self.s3_run, 'caller-run', trial_prefix, str(i + 1)),
                                       'rvoip bench caller-run %d' % (i + 1), caller_timeout)
                    for i, cid in enumerate(callers)}
        deadline, markers = time.time() + 300, []
        while time.time() < deadline:
            markers = [k for k in self.aws.s3_list(trial_prefix + 'barrier/') if '/ready-' in k]
            if len(markers) >= len(callers):
                break
            time.sleep(2)
        else:
            for cmd in commands.values():
                self.aws.call('ssm', 'cancel-command', '--command-id', cmd)
            self.finish_dut(trial_prefix, dut_bin)
            return None, rel, local, 'start barrier: %d of %d callers ready' % (len(markers), len(callers))
        go = {'start_at': round(time.time() + 5.0, 3), 'callers': len(callers)}
        self.aws.s3_put_json(trial_prefix + 'barrier/go.json', go)
        # 3. Generate: callers exit after their stages, the hold and the linger.
        results = {}
        for cid, cmd in commands.items():
            results.update(self.aws.wait_command(cmd, [cid], caller_timeout + 300, poll=20))
        caller_rc = {cid: [r['Status'], r.get('ResponseCode')] for cid, r in results.items()}
        (local / 'caller-exit.json').write_text(json.dumps(caller_rc, indent=1))
        if any(rc == 75 for _, rc in caller_rc.values()):
            self.finish_dut(trial_prefix, dut_bin)
            raise Infra('disk guard refused a caller: %s' % caller_rc)
        # 4. Drain the DUT, pack its evidence.
        self.finish_dut(trial_prefix, dut_bin)
        # 5. Merge and summarize on caller 1 (about 6 GB of RAM for 1M calls), before the next trial.
        try:
            self.aws.run([callers[0]], self.lane.agent(self.s3_run, 'summarize', trial_prefix, str(len(callers))),
                         'rvoip bench summarize', 1800)
            (local / 'merged').mkdir(exist_ok=True)
            self.aws.s3_cp(trial_prefix + 'merged/summary.json', cached)
            self.aws.s3_cp(trial_prefix + 'evidence-manifest.json', local / 'evidence-manifest.json')
        except AwsError as error:
            return None, rel, local, 'evidence/summary: %s' % str(error)[-400:]
        return json.loads(cached.read_text()), rel, local, None

    def finish_dut(self, trial_prefix, dut_bin):
        dut = self.lane.info['dut']['id']
        try:
            self.aws.run([dut], self.lane.agent(self.s3_run, 'dut-finish', trial_prefix, dut_bin),
                         'rvoip bench dut-finish', 420)
        except AwsError as error:
            self.log('WARNING dut-finish:', str(error)[-300:])

    def verdict(self, summary, rel, local, note):
        """rfc7502.trial_verdict plus the search halt rules; written to verdict.json."""
        verdict = rfc7502.trial_verdict(summary, 'rfc')
        if note:
            verdict.setdefault('reasons', []).insert(0, note)
        verdict['halt'] = search.halt_reasons(summary) if summary else []
        verdict['run'] = rel
        verdict['evidence'] = self.s3_run + rel + '/'
        (local / 'verdict.json').write_text(json.dumps(verdict, indent=1, sort_keys=True))
        try:
            self.aws.s3_cp(local / 'verdict.json', self.s3_run + rel + '/verdict.json')
        except AwsError as error:
            self.log('WARNING verdict upload:', error)
        self.log('trial', rel, verdict['outcome'], 'attempted', verdict.get('attempted'),
                 'succeeded', verdict.get('succeeded'), '; '.join(verdict.get('reasons') or [])[:300])
        return verdict


# ---------------------------------------------------------------------------
# Profiles
# ---------------------------------------------------------------------------

def trial_profile(ctx, rate, ramp=False):
    t = profiles.TRIAL
    return rfc7502.trial_profile(rate, t['n'], 120 if ramp else t['warmup_seconds'], t['minimum_trial_seconds'],
                                 warmup_ramp=ramp, hosts=len(ctx.lane.info['callers']))


def run_one(ctx, row_id, row, rate, index, admission=None, rel=None, ramp=False):
    summary, rel, local, note = ctx.trials.run(row_id, row, rate, index, trial_profile(ctx, rate, ramp),
                                               admission=admission, rel=rel)
    verdict = ctx.trials.verdict(summary, rel, local, note)
    return verdict, summary


def compact(verdict, summary, **extra):
    """The receipt's per-trial record."""
    detail = verdict.get('detail') or {}
    out = {'rate': verdict.get('detail', {}).get('rate') or extra.get('rate'), 'run': verdict.get('run'),
           'outcome': verdict['outcome'], 'attempted': verdict.get('attempted'),
           'established': verdict.get('succeeded'), 'attempt_failures': verdict.get('failed', 0),
           'disconnect_failures': detail.get('disconnect_failures'),
           'status_counts': detail.get('final_status_counts'),
           'invalid_reason': '; '.join(verdict.get('reasons') or []) if verdict['outcome'] == 'invalid' else None,
           'halt': verdict.get('halt') or None, 'evidence': verdict.get('evidence')}
    if summary:
        out['srd_ms'] = score.srd(summary)
        out['sdd_ms'] = score.sdd(summary)
        out['caller_valid'] = (summary.get('caller_validity') or {}).get('caller_valid_before_backpressure')
        out['dut_peak_cores'] = (summary.get('dut') or {}).get('peak_cpu_cores')
    out.update(extra)
    return out


def search_record(trial, s3_run):
    """The receipt's record of one search.json trial (the compact() fields)."""
    detail = trial.get('detail') or {}
    run = detail.get('run')
    return {'rate': trial['rate'], 'run': run, 'outcome': trial['outcome'], 'kind': 'search',
            'attempted': trial.get('attempted'), 'established': trial.get('succeeded'),
            'attempt_failures': trial.get('failed', 0), 'disconnect_failures': detail.get('disconnect_failures'),
            'status_counts': detail.get('final_status_counts'),
            'invalid_reason': '; '.join(detail.get('invalid_reasons') or []) if trial['outcome'] == 'invalid' else None,
            'halt': detail.get('halt_reasons') or None, 'evidence': s3_run + run + '/' if run else None}


def run_smoke(ctx, row_id):
    """smoke: one short trial per row at --r0 (functional check of a lane; never a gate)."""
    p = profiles.PROFILES[ctx.args.profile]
    row = profiles.ROWS[row_id]
    rate = profiles.quantize(ctx.args.r0.get(row_id) or p['rates'][row_id], profiles.TRIAL['quantum'] * 2)
    seconds = ctx.args.smoke_seconds or p['seconds']
    profile = rfc7502.trial_profile(rate, rate * seconds, p['warmup_seconds'], seconds,
                                    hosts=len(ctx.lane.info['callers']))
    result = {'id': row_id, 'transport': row['transport'], 'media': row['media'], 'hold_ms': row['hold_ms'],
              'trials': []}
    callers = [c['id'] for c in ctx.lane.info['callers']]
    for variant in ctx.args.diag_variants:
        # Diagnostic variants (--diag-variant): a caller host tweak before the trial; 'none' is the testbed as pinned.
        if ctx.args.diag_variants != ['none']:
            tweak = ctx.aws.run(callers, ctx.lane.agent(ctx.s3_run, 'host-tweak', variant), 'rvoip bench host-tweak', 120)
            ctx.log('host-tweak', variant, {i: r['StandardOutputContent'].strip()[-200:] for i, r in tweak.items()})
        rel = '%s/t00-r%d' % (row_id, rate) if ctx.args.diag_variants == ['none'] else '%s/diag-%s-r%d' % (row_id, variant, rate)
        summary, rel, local, note = ctx.trials.run(row_id, row, rate, 0, profile, rel=rel)
        verdict = ctx.trials.verdict(summary, rel, local, note)
        result['trials'].append(compact(verdict, summary, rate=rate, kind='smoke', variant=variant,
                                        softirq={k: (v or {}).get('softirq_peak_share')
                                                 for k, v in ((summary or {}).get('resource') or {}).items()},
                                        net_flagged={k: ((v or {}).get('net') or {}).get('flagged')
                                                     for k, v in ((summary or {}).get('resource') or {}).items()}))
    checks = (summary or {}).get('checks') or {}
    bad = sorted(k for k, v in checks.items() if v is False)
    if verdict['outcome'] == 'pass' and not verdict['halt'] and not bad:
        result['verdict'] = 'pass'
    else:
        result['verdict'] = 'fail' if verdict['outcome'] == 'fail' else 'infra'
        result['reason'] = '%s smoke at %d sps: %s; failed checks %s; halt %s' % (
            row_id, rate, verdict['outcome'], bad, verdict['halt'])
    return result


def run_confirm(ctx, row_id, recorded):
    """release: 3 x 0.95R (must pass), 1 x 1.05R (warn if it passes), RFC 6076 latency at 0.9R."""
    p = profiles.PROFILES[ctx.args.profile]
    row = profiles.ROWS[row_id]
    r = recorded['r']
    result = {'id': row_id, 'transport': row['transport'], 'media': row['media'], 'hold_ms': row['hold_ms'],
              'recorded_r': r, 'tester_cap': recorded.get('tester_cap'),
              'tester_cap_lower_bound': bool(recorded.get('tester_cap_lower_bound')), 'trials': []}
    plan = [('confirm', f) for f in p['confirm']] + [('probe_up', p['probe_up']), ('latency', p['latency_at'])]
    index = len(result['trials'])
    for kind, factor in plan:
        rate = profiles.quantize(r * factor)
        invalid = 0
        while True:
            verdict, summary = run_one(ctx, row_id, row, rate, index)
            index += 1
            result['trials'].append(compact(verdict, summary, rate=rate, factor=factor, kind=kind))
            ctx.persist_row(row_id, result)
            if verdict['halt']:
                result['verdict'] = 'fail'
                result['reason'] = '%s at %d sps: DUT did not drain cleanly (%s)' % (
                    row_id, rate, '; '.join(verdict['halt']))
                return result
            if verdict['outcome'] != 'invalid':
                break
            invalid += 1
            if invalid >= profiles.TRIAL['invalid_streak_stop']:
                result['verdict'] = 'infra'
                result['reason'] = '%s: %d invalid trials in a row at %d sps' % (row_id, invalid, rate)
                return result
        if kind == 'confirm' and verdict['outcome'] == 'fail':
            result['verdict'] = 'fail'
            result['reason'] = '%s confirmation at %.2fxR = %d sps failed: %s' % (
                row_id, factor, rate, '; '.join(verdict.get('reasons') or []))
            return result
        if kind == 'latency':
            result['latency'] = score.latency_verdict(summary, recorded, p['latency_tolerance'], rate)
    up = [t for t in result['trials'] if t['kind'] == 'probe_up' and t['outcome'] != 'invalid']
    latency = result.get('latency') or {}
    if latency.get('verdict') == 'fail':
        result['verdict'], result['reason'] = 'fail', '%s latency: %s' % (row_id, latency.get('reason'))
    elif up and up[-1]['outcome'] == 'pass':
        result['verdict'] = 'warn'
        result['proposed_r'] = up[-1]['rate']
        result['reason'] = '%s 1.05xR trial (%d sps) passed: likely improvement, record a new R after review' % (
            row_id, up[-1]['rate'])
    else:
        result['verdict'] = 'pass'
    return result


def baseline_lower_bound(search_outcome):
    """The §6.1 baseline value of a search that ran out of trials or time before it saw the tester's ceiling
    (every trial at or below its highest passing rate passed), else None. Recorded as a lower bound, flagged
    `lower_bound` wherever rates are read, never as a measured ceiling. ea-uas baselines only: a DUT search
    that stops this way has not converged and fails."""
    stop = (search_outcome or {}).get('stop_reason') or ''
    if (search_outcome or {}).get('result_sps') is None and stop.startswith(('max_trials', 'max_hours')):
        return (search_outcome or {}).get('highest_passing_sps') or None
    return None


def run_search_row(ctx, row_id, recorded, caps):
    """minor/full/baseline: bisect or exact §4.10 search of one row (scripts/bench/search.py)."""
    p = profiles.PROFILES[ctx.args.profile]
    row = profiles.ROWS[row_id]
    hosts = len(ctx.lane.info['callers'])
    cap = caps.get(row_id)
    r0 = (ctx.args.r0.get(row_id) or (recorded or {}).get('r') or p.get('r0', {}).get(row_id)
          or (int(cap * 0.8) if cap else None))
    if not r0:
        raise Usage('no r0 for %s: pass --r0 %s=<sps>, a recorded R, or a tester cap' % (row_id, row_id))
    spec = {'row': row_id, 'criterion': 'rfc', 'r0': profiles.quantize(r0), 'n': profiles.TRIAL['n'],
            'algorithm': p['algorithm'], 'confirmations': p['confirmations'],
            'resolution': p.get('resolution', 0.02), 'r_max': cap, 'quantum': profiles.TRIAL['quantum'],
            'max_trials': p['max_trials'], 'max_hours': p['budget_hours'], 'hosts': hosts,
            'hold_ms': row['hold_ms'], 'minimum_trial_seconds': profiles.TRIAL['minimum_trial_seconds'],
            'warmup_seconds': profiles.TRIAL['warmup_seconds'],
            'halt_on_missing_dut_evidence': not ctx.lane.ea_uas}
    folder = ctx.args.local / row_id
    folder.mkdir(parents=True, exist_ok=True)

    def run_trial(rate, index, profile):
        summary, rel, local, note = ctx.trials.run(row_id, row, rate, index, profile)
        ctx.aws.s3_cp(folder / 'search.json', ctx.s3_run + row_id + '/search.json') if (folder / 'search.json').exists() else None
        return search.TrialResult(summary=summary, run=rel, trial_dir=local, note=note)

    outcome = search.run_search(spec, folder / 'search.json', folder / 'trials.jsonl', run_trial, log=ctx.log,
                                deadline=ctx.args.deadline)
    for name in ('search.json', 'trials.jsonl'):
        if (folder / name).exists():
            ctx.aws.s3_cp(folder / name, ctx.s3_run + row_id + '/' + name)
    if outcome.get('paused') and outcome.get('halted'):
        raise Infra('search %s paused: %s' % (row_id, outcome['halted']))   # void trials, not the wall clock
    if outcome.get('paused'):
        raise WallClock('search %s paused: %s' % (row_id, outcome.get('halted')))
    trials = []
    if (folder / 'search.json').exists():
        trials = [search_record(t, ctx.s3_run) for t in json.loads((folder / 'search.json').read_text()).get('trials', [])]
    result = {'id': row_id, 'transport': row['transport'], 'media': row['media'], 'hold_ms': row['hold_ms'],
              'recorded_r': (recorded or {}).get('r'), 'tester_cap': cap,
              'tester_cap_lower_bound': row_id in ctx.args.cap_lower_bound,
              'search': {k: outcome.get(k) for k in ('algorithm', 'result_sps', 'stop_reason', 'tester_limited',
                                                      'highest_passing_sps', 'lowest_failing_sps', 'halted',
                                                      'trials', 'valid_trials', 'parameters')},
              'trials': trials}
    r = outcome.get('result_sps')
    stop = outcome.get('stop_reason') or ''
    if r is None and ctx.lane.ea_uas and 'invalid' in stop and outcome.get('highest_passing_sps'):
        # §6.1 testbed baseline: the tester itself ran out (caller validity) above every passing rate.
        # The baseline is then >= the highest passing rate (E74: "stopped: 3 consecutive invalid trials").
        r = outcome['highest_passing_sps']
        outcome['tester_limited'] = True
    if r is None and ctx.lane.ea_uas and baseline_lower_bound(outcome):
        r = baseline_lower_bound(outcome)
        result['lower_bound'] = True
        result['lower_bound_reason'] = outcome.get('stop_reason')
    if r is None:
        reason = outcome.get('halted') or stop or 'no result'
        result['verdict'] = 'infra' if 'invalid' in reason else 'fail'
        result['reason'] = '%s search: %s' % (row_id, reason)
        return result
    result['r'] = r
    result['tester_limited'] = bool(outcome.get('tester_limited'))
    if p.get('latency_at') and not ctx.lane.ea_uas:
        rate = profiles.quantize(r * p['latency_at'])
        verdict, summary = run_one(ctx, row_id, row, rate, 0, rel='%s/latency-r%d' % (row_id, rate))
        result['latency_trial'] = compact(verdict, summary, rate=rate, kind='latency')
        result['latency'] = score.latency_verdict(summary, recorded or {}, {'relative': 0.25, 'floor_ms': 10.0}, rate)
    floor = (recorded or {}).get('r')
    if floor and r < 0.95 * floor and not ctx.lane.ea_uas:
        result['verdict'], result['reason'] = 'fail', '%s R = %d < 0.95 x recorded %d' % (row_id, r, floor)
    elif (result.get('latency') or {}).get('verdict') == 'fail':
        result['verdict'], result['reason'] = 'fail', '%s latency: %s' % (row_id, result['latency'].get('reason'))
    else:
        result['verdict'] = 'pass'
    return result


def run_overload(ctx, ceiling):
    """R4: admission max-cps = C; stages 0.5/1/1.5/2 x C then recovery 0.5/1 x C, after a 120 s ramp."""
    row = profiles.R4
    hosts = len(ctx.lane.info['callers'])
    q = profiles.TRIAL['quantum'] * hosts
    admission = {'mode': 'max-cps', 'max_cps': ceiling, 'retry_after_secs': 2}
    stages = [{'name': 'warmup-%d' % (k + 1), 'cps': profiles.quantize(ceiling * f / 2, q), 'seconds': 30}
              for k, f in enumerate((0.25, 0.5, 0.75, 1.0))]
    stages += [{'name': 'trial-%d' % (k + 1), 'cps': profiles.quantize(ceiling * f, q), 'seconds': s}
               for k, (f, s) in enumerate(row['stages'])]
    profile = {'name': 'r4-overload', 'stages': stages, 'transient_stages': [s['name'] for s in stages[:4]],
               'hosts': hosts, 'basis': 'overload 0.5/1/1.5/2 x C, recovery 0.5/1 x C'}
    summary, rel, local, note = ctx.trials.run('R4', row, ceiling, 0, profile, admission=admission,
                                               rel='R4/t00-overload-c%d' % ceiling)
    verdict = ctx.trials.verdict(summary, rel, local, note)
    result = score.overload_verdict(summary, ceiling, profile)
    result.update(id='R4', transport=row['transport'], media=row['media'], hold_ms=row['hold_ms'],
                  ceiling=ceiling, trials=[compact(verdict, summary, rate=ceiling, kind='overload')])
    return result


# ---------------------------------------------------------------------------
# Context, receipt, main
# ---------------------------------------------------------------------------

class Context:
    def __init__(self, args, aws, log):
        self.args, self.aws, self.log = args, aws, log
        self.s3_run = 's3://%s/bench/%s/' % (args.bucket, args.run_id)
        self.state = args.load_state()

        def mirror(path):
            try:
                aws.s3_cp(path, self.s3_run + 'state.json')
            except AwsError as error:
                log('WARNING state mirror:', error)

        args.state_mirror = None if args.dry_run else mirror
        self.lane = Lane(args, aws, log, self.state)
        self.trials = Trials(args, aws, self.lane, log, self.s3_run)

    def restore(self):
        """Pull the run's small state (state, search states, trial summaries and verdicts) from S3,
        so any leg on any machine resumes where the last one stopped. Unknown run-ids start fresh."""
        if not self.aws.s3_list(self.s3_run + 'state.json'):
            self.log('resume: no state for %s in S3; starting a new run' % self.args.run_id)
            return
        self.aws.s3_sync(self.s3_run, self.args.local, RESTORE_PATTERNS)
        self.state = self.args.load_state()
        self.lane.state = self.state
        self.log('resume: restored %s (rows done: %s)' % (
            self.args.run_id, sorted(k for k, v in self.state.get('rows', {}).items() if v.get('verdict'))))

    def persist_row(self, row_id, result):
        self.state.setdefault('rows', {})[row_id] = result
        self.args.save_state(self.state)


def cost(state, ended):
    by_type = {}
    for lane in (state.get('earlier_lanes') or []) + [state.get('lane') or {}]:   # every leg's lane
        if not lane.get('launched_at'):
            continue
        start = datetime.datetime.strptime(lane['launched_at'], '%Y-%m-%dT%H:%M:%SZ').replace(tzinfo=datetime.timezone.utc).timestamp()
        end = lane.get('terminated_at')
        end = datetime.datetime.strptime(end, '%Y-%m-%dT%H:%M:%SZ').replace(tzinfo=datetime.timezone.utc).timestamp() if end else ended
        hours = (end - start) / 3600
        for t in [lane.get('dut_type')] + [c['type'] for c in lane.get('callers') or []]:
            if t:
                by_type[t] = round(by_type.get(t, 0) + hours, 3)
    usd = sum(profiles.PRICE_PER_HOUR.get(t, 0) * h for t, h in by_type.items())
    return {'instance_hours': by_type, 'estimated_usd': round(usd, 2)}


def receipt_for(ctx, rows, started, status, reasons, env_id, rates_path, report):
    args, state = ctx.args, ctx.state
    lane = state.get('lane') or {}
    facts = next(iter((lane.get('facts') or {}).values()), {})
    bins = {}
    for role, path in (('dut_sha256', args.dut_bin), ('caller_sha256', args.caller_bin), ('ea_uas_sha256', args.ea_uas_bin)):
        bins[role] = sha256_file(path) if path and Path(path).exists() else None
    ended = time.time()
    receipt = {
        'schema': RECEIPT_SCHEMA, 'profile': args.profile, 'candidate_sha': args.candidate, 'run_id': args.run_id,
        'status': status, 'reasons': reasons, 'started_at': utc(started), 'ended_at': utc(ended),
        'duration_seconds': round(ended - started, 1),
        'environment': dict({
            'env_id': env_id, 'region': args.region, 'availability_zone': lane.get('az'),
            'dut_instance_type': lane.get('dut_type'), 'caller_instance_type': lane.get('caller_type'),
            'caller_count': len(lane.get('callers', [])),
            'sources_per_caller': min((len(c['ips']) for c in lane.get('callers', [])), default=None),
            'connections_per_source': CALLER_CONNECTIONS, 'ami_id': lane.get('ami_id'),
            'caller_media': getattr(args, 'caller_media', None),
            'kernel': facts.get('kernel'),
            'capacity_reservations': [{k: r.get(k) for k in ('id', 'instance_type', 'count', 'az', 'created_at',
                                                              'cancelled_at', 'held_for_resume', 'ends_at')}
                                      for r in state.get('reservations', [])]}, **bins),
        'baseline': {'path': str(rates_path.relative_to(REPO)) if rates_path else None,
                     'sha256': sha256_file(rates_path) if rates_path and rates_path.exists() else None},
        'rows': rows,
        'recommendation': {r['id']: {'new_r': r['proposed_r'], 'basis': '1.05xR passed'}
                           for r in rows if r.get('proposed_r')},
        'evidence': {'s3_prefix': ctx.s3_run, 'manifest_sha256': None, 'report': report},
        'cost': cost(state, ended),
        'tool': {'run_lane_sha256': sha256_file(Path(__file__)), 'repo_sha': git_sha()},
    }
    return receipt


def git_sha():
    out = subprocess.run(['git', '-C', str(REPO), 'rev-parse', 'HEAD'], capture_output=True, text=True)
    return out.stdout.strip() or None


def write_manifest(ctx, receipt):
    """Every S3 object of the run with the sha256 recorded beside it."""
    keys = ctx.aws.s3_list(ctx.s3_run)
    shas = {k[:-len('.sha256')]: k for k in keys if k.endswith('.sha256')}
    manifest = {'s3_prefix': ctx.s3_run, 'objects': sorted(k for k in keys if not k.endswith('.sha256')),
                'archives': {}}
    for obj, sha_key in sorted(shas.items()):
        local = ctx.args.local / 'manifest-tmp.sha256'
        bucket = ctx.s3_run.split('/')[2]
        ctx.aws.s3_cp('s3://%s/%s' % (bucket, sha_key), local)
        manifest['archives'][obj] = local.read_text().split()[0]
    path = ctx.args.local / 'manifest.json'
    path.write_bytes(canonical(manifest))
    ctx.aws.s3_cp(path, ctx.s3_run + 'manifest.json')
    receipt['evidence']['manifest_sha256'] = sha256_file(path)


def parse_args(argv):
    p = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    p.add_argument('--profile', required=True, choices=sorted(profiles.PROFILES))
    p.add_argument('--candidate', required=True)
    p.add_argument('--receipt', required=True)
    p.add_argument('--dut-bin', help='rvoip-bench-uas built at --candidate (required except for baseline)')
    p.add_argument('--caller-bin', help='rvoip-loadgen; default: the env-id-pinned build from S3 (env.json)')
    p.add_argument('--ea-uas-bin', help='ea-uas for baseline; default: the env-id-pinned build from S3')
    p.add_argument('--env-id', help='default: perf-baselines/rfc7502/ENVIRONMENT')
    p.add_argument('--region', default=os.environ.get('RVOIP_AWS_REGION', 'us-west-2'))
    p.add_argument('--bucket', default=os.environ.get('RVOIP_AWS_EVIDENCE_BUCKET'))
    p.add_argument('--lane-config', help='JSON of the infra/aws/bench outputs (default: terraform output)')
    p.add_argument('--callers', type=int, help='caller hosts (default: the env-id\'s pinned caller_count, else 2)')
    p.add_argument('--az', help='launch the lane in this AZ instead of the templates\' (same-named public subnet, '
                                'same VPC); default: the env-id\'s pinned AZ, else the templates\'')
    p.add_argument('--ea-host-type', default='c8i.12xlarge', help='baseline: ea-uas host type (E74 used c8i.12xlarge)')
    p.add_argument('--run-id')
    p.add_argument('--resume', action='store_true',
                   help='continue --run-id from its S3 state; a finished run rewrites its receipt and exits at once; '
                        'an unknown run-id starts a new run')
    p.add_argument('--max-wall', type=float, help='minutes from process start; no trial starts that would not '
                                                   'finish before it (exit 4, resume later)')
    p.add_argument('--dry-run', action='store_true')
    p.add_argument('--record', action='store_true', help='minor/full/baseline: write rates.json for the env-id')
    p.add_argument('--record-out', help='with --record: write rates.json to this path instead of into the checkout '
                                        '(also uploaded beside the receipt)')
    p.add_argument('--rows', help='comma list overriding the profile rows (debug)')
    p.add_argument('--r0', action='append', default=[], help='ROW=sps initial rate for a search row')
    p.add_argument('--cap', action='append', default=[], help='ROW=sps tester cap override')
    p.add_argument('--smoke-seconds', type=int, help='smoke: scored stage length (default: the profile\'s)')
    p.add_argument('--diag-variant', dest='diag_variants', action='append',
                   help='smoke only: run each row once per caller host tweak (none, notrack); diagnostic, never recordable')
    p.add_argument('--dut-type', help='override the DUT instance type (functional smoke on small hosts; never a gate run)')
    p.add_argument('--caller-type', help='override the caller instance type (smoke only)')
    p.add_argument('--keep-lane', action='store_true', help='do not terminate the lane (debug; costs money)')
    p.add_argument('--reserve-capacity', action=argparse.BooleanOptionalAction, default=None,
                   help='hold an On-Demand Capacity Reservation for the lane while each leg runs '
                        '(default: on for minor and full, off otherwise)')
    p.add_argument('--keep-local', action='store_true')
    p.add_argument('--break-lock', action='store_true')
    args = p.parse_args(argv)
    args.started = time.time()
    args.diag_variants = args.diag_variants or ['none']
    if args.diag_variants != ['none'] and (args.profile not in ('smoke', 'diag') or args.record):
        raise Usage('--diag-variant is a smoke-only diagnostic and is never recorded')
    if not re.fullmatch(r'[0-9a-f]{40}', args.candidate):
        raise Usage('--candidate must be a 40-hex sha')
    if not args.bucket:
        raise Usage('--bucket or RVOIP_AWS_EVIDENCE_BUCKET is required')
    if args.record and profiles.PROFILES[args.profile]['method'] != 'search':
        raise Usage('--record is for minor, full and baseline')
    ea = profiles.PROFILES[args.profile].get('ea_uas')
    if not ea and not args.dry_run and not (args.dut_bin and Path(args.dut_bin).is_file()):
        raise Usage('--dut-bin: no such file %r' % args.dut_bin)
    for name in ('caller_bin', 'ea_uas_bin', 'dut_bin'):
        path = getattr(args, name)
        if path and not Path(path).is_file():
            raise Usage('--%s: no such file %r' % (name.replace('_', '-'), path))
    if args.reserve_capacity is None:
        args.reserve_capacity = bool(profiles.PROFILES[args.profile].get('reserve_capacity'))
    args.env_tag = None    # the env-id the lane's reservations are tagged with (set once resolved)
    args.caller_media = None    # env.json caller_media (set once resolved)
    args.cap_lower_bound = set()    # rows whose tester cap comes from a lower-bound baseline
    args.r0 = {k: int(v) for k, v in (kv.split('=', 1) for kv in args.r0)}
    args.cap = {k: int(v) for k, v in (kv.split('=', 1) for kv in args.cap)}
    args.run_id = args.run_id or 'bench-%s-%s-%s' % (args.profile, args.candidate[:12], stamp())
    if not re.fullmatch(r'[A-Za-z0-9._-]{1,96}', args.run_id):
        raise Usage('bad --run-id')
    args.local = WORKDIR / args.run_id
    args.local.mkdir(parents=True, exist_ok=True)
    state_path = args.local / 'state.json'
    args.deadline = args.started + args.max_wall * 60 if args.max_wall else None

    def load_state():
        return json.loads(state_path.read_text()) if state_path.exists() else {'run_id': args.run_id}

    def save_state(state):
        tmp = state_path.with_suffix('.tmp')
        tmp.write_text(json.dumps(state, indent=1, sort_keys=True))
        tmp.replace(state_path)
        if args.state_mirror:
            args.state_mirror(state_path)

    def check_wall(need_seconds=TRIAL_ALLOWANCE_SECONDS):
        """Raise WallClock when `need_seconds` more work would not finish before --max-wall."""
        if args.deadline and time.time() + need_seconds > args.deadline:
            raise WallClock('--max-wall: %.0f s left, the next step needs about %.0f s' % (
                args.deadline - time.time(), need_seconds))

    args.load_state, args.save_state, args.check_wall, args.state_mirror = load_state, save_state, check_wall, None
    return args


# Launch, SSM-online and install of a lane; and a typical trial with its overhead.
LAUNCH_ALLOWANCE_SECONDS = 20 * 60
TRIAL_ALLOWANCE_SECONDS = 15 * 60
RESTORE_PATTERNS = ('state.json', '*/search.json', '*/trials.jsonl', '*/merged/summary.json', '*/verdict.json',
                    '*/plan.json', '*/evidence-manifest.json', 'report/*')


def resolve_environment(args, aws):
    """env-id and pinned tester builds (perf-baselines/rfc7502/ENVIRONMENT and <env-id>/env.json).

    A missing --caller-bin / --ea-uas-bin is fetched from env.json's S3 copy and checked against its sha256."""
    env_id = args.env_id
    if not env_id and (BASELINES / 'ENVIRONMENT').exists():
        env_id = (BASELINES / 'ENVIRONMENT').read_text().strip()
    pinned = None
    if env_id and (BASELINES / env_id / 'env.json').exists():
        pinned = json.loads((BASELINES / env_id / 'env.json').read_text())
        assert pinned.get('schema') == 'rvoip-rfc7502-env-v1', 'bad env.json for %s' % env_id
    ea = profiles.PROFILES[args.profile].get('ea_uas')
    for attr, key in (('caller_bin', 'rvoip-loadgen'), ('ea_uas_bin', 'ea-uas')):
        if getattr(args, attr) or (attr == 'ea_uas_bin' and not ea):
            continue
        build = (pinned or {}).get('builds', {}).get(key)
        if not build:
            raise Usage('no --%s and no pinned %s build for env-id %r' % (attr.replace('_', '-'), key, env_id))
        local = args.local / 'pinned' / key
        if not local.exists() or sha256_file(local) != build['sha256']:
            local.parent.mkdir(parents=True, exist_ok=True)
            aws.s3_cp(build['s3_uri'], local)
        if sha256_file(local) != build['sha256']:
            raise Infra('pinned %s at %s does not match sha256 %s' % (key, build['s3_uri'], build['sha256']))
        local.chmod(0o755)
        setattr(args, attr, str(local))
    return env_id, pinned


def resolve_az(wanted, pinned, leg):
    """The lane's AZ: --az, else env.json's pinned AZ, else an earlier leg's. A baseline and the runs gated on
    it share one AZ, and so do the legs of one run: a conflict is a usage error, never a fallback."""
    for source, az in (('the env-id pins', pinned), ("this run's earlier leg ran in", leg)):
        if az and wanted and az != wanted:
            raise Usage('--az %s, but %s %s' % (wanted, source, az))
        wanted = wanted or az
    return wanted


# Profiles whose results gate a release or are recorded: they run on exactly the pinned testbed.
PINNED_PROFILES = ('release', 'minor', 'full', 'baseline')


def resolve_callers(wanted, pinned, leg, profile):
    """The lane's caller count: --callers, else env.json's caller_count, else an earlier leg's, else 2. The
    env-id names the count, so a run that gates or records cannot use another; smoke/diag may (never recorded).
    The legs of one run always share it."""
    if leg and wanted and wanted != leg:
        raise Usage("--callers %d, but this run's earlier leg ran %d" % (wanted, leg))
    if pinned and wanted and wanted != pinned and profile in PINNED_PROFILES:
        raise Usage('--callers %d, but the env-id pins %d callers' % (wanted, pinned))
    return wanted or leg or pinned or 2


def outcome_code(status):
    return {'PASS': EXIT_PASS, 'WARN': EXIT_PASS, 'FAIL': EXIT_FAIL}.get(status, EXIT_INFRA)


def main(argv=None):
    """run_lane with SIGINT/SIGTERM turned into Interrupted, so a stopped run still tears its lane down,
    cancels its reservations, releases the lock and writes an INFRA receipt."""
    SIGNALS['unwinding'] = False

    def on_signal(signum, frame):
        if SIGNALS['unwinding']:
            return    # already on the exit path: let the cleanup finish
        SIGNALS['unwinding'] = True
        raise Interrupted('interrupted by %s' % signal.Signals(signum).name)

    previous = {}
    if threading.current_thread() is threading.main_thread():
        for sig in (signal.SIGINT, signal.SIGTERM):
            previous[sig] = signal.signal(sig, on_signal)
    try:
        return run(argv)
    finally:
        for sig, handler in previous.items():
            signal.signal(sig, handler)


def run(argv=None):
    try:
        args = parse_args(argv if argv is not None else sys.argv[1:])
    except Usage as error:
        print('run_lane: %s' % error, file=sys.stderr)
        return EXIT_USAGE
    log = Log(args.local / 'run_lane.log')
    aws = Aws(args.region, log=log)
    ctx = Context(args, aws, log)
    started = args.started
    profile = profiles.PROFILES[args.profile]
    rows_wanted = args.rows.split(',') if args.rows else profile['rows']
    lock = 's3://%s/bench/lock.json' % args.bucket
    have_lock = False
    status, reasons, rows, report = 'INFRA', [], [], None
    env_id, rates_path = args.env_id, None
    code = EXIT_INFRA
    stopped_for_wall = False
    interrupted = False
    try:
        if args.dry_run:
            log('dry run: quota', aws.vcpu_quota(), 'in use', aws.running_standard_vcpus(),
                'outputs', ctx.lane.tf_outputs())
            return EXIT_PASS
        if args.resume:
            ctx.restore()
            final = ctx.state.get('final')
            if final:
                log('run %s already finished (%s): rewriting the receipt' % (args.run_id, final['status']))
                receipt = final['receipt']
                Path(args.receipt).parent.mkdir(parents=True, exist_ok=True)
                Path(args.receipt).write_bytes(canonical(receipt))
                if args.record and args.record_out and final['status'] in ('PASS', 'WARN'):
                    # The leg that finished recorded the rates; hand the same file to this call's artifact.
                    env = (receipt.get('environment') or {}).get('env_id')
                    Path(args.record_out).parent.mkdir(parents=True, exist_ok=True)
                    aws.s3_cp(ctx.s3_run + 'record/%s/rates.json' % env, args.record_out)
                return outcome_code(final['status'])
        env_id, pinned = resolve_environment(args, aws)
        args.az = resolve_az(args.az, (pinned or {}).get('az'), (ctx.state.get('lane') or {}).get('az'))
        args.callers = resolve_callers(args.callers, (pinned or {}).get('caller_count'),
                                       len((ctx.state.get('lane') or {}).get('callers') or []) or None, args.profile)
        args.env_tag = env_id
        # The callers' media executor is part of the testbed: the env-id's pinned layout, or rvoip's default.
        args.caller_media = (pinned or {}).get('caller_media')
        args.check_wall(LAUNCH_ALLOWANCE_SECONDS + TRIAL_ALLOWANCE_SECONDS)
        if not aws.s3_create_exclusive(lock, {'run_id': args.run_id, 'at': utc(), 'profile': args.profile}):
            held = aws.s3_get_json(lock)
            if held and held.get('run_id') == args.run_id:
                pass
            elif args.break_lock:
                log('breaking lock held by', held)
                aws.s3_put_json(lock, {'run_id': args.run_id, 'at': utc(), 'profile': args.profile})
            else:
                raise Infra('lane lock held by %s' % held)
        have_lock = True
        ctx.lane.sweep_reservations()
        if not (args.resume and ctx.lane.alive()):
            ctx.lane.launch(profile['budget_hours'])
            ctx.lane.install(ctx.s3_run)
        lane = ctx.lane.info
        if pinned and pinned.get('ami_id') and lane.get('ami_id') != pinned['ami_id'] and not args.dut_type:
            raise Usage('lane AMI %s differs from the env-id %s AMI %s: re-pin the environment (pin_env.py)' % (
                lane.get('ami_id'), env_id, pinned['ami_id']))
        env_id = env_id or env_id_for(lane, sha256_file(args.caller_bin))
        if args.dut_type or args.caller_type:
            # Overridden types: label the env-id with what actually ran; never recordable.
            env_id = 'smoke-' + env_id_for(dict(lane, tf=dict(lane['tf'], dut_instance_type=lane['dut_type'],
                                                              caller_instance_type=lane['caller_type'])),
                                         sha256_file(args.caller_bin))
            if args.record:
                raise Usage('--record is not allowed with --dut-type/--caller-type (smoke lane)')
        rates, rates_path = load_rates(env_id)
        aws.s3_put_json(ctx.s3_run + 'lane.json', {k: v for k, v in lane.items() if k != 'facts'})
        log('env-id', env_id, 'rates', rates_path if rates else 'none recorded')
        # A baseline measures the testbed itself: never capped by, or seeded from, what it recorded before.
        caps = dict({} if profile.get('ea_uas') else (rates or {}).get('tester_caps', {}), **args.cap)
        # Caps from a lower-bound baseline row (the tester's ceiling never observed) are flagged in every row.
        args.cap_lower_bound = {row for row, b in ((rates or {}).get('baseline_sps') or {}).items()
                                if b.get('lower_bound') and row not in args.cap}
        if args.profile == 'release' and not rates:
            raise Usage('no recorded rates for %s (%s): run baseline and minor --record first' % (env_id, rates_path))
        for row_id in rows_wanted:
            done = ctx.state.get('rows', {}).get(row_id)
            if done and done.get('verdict'):
                rows.append(done)
                continue
            recorded = (rates or {}).get('baseline_sps' if profile.get('ea_uas') else 'rows', {}).get(row_id)
            if profile['method'] == 'smoke':
                result = run_smoke(ctx, row_id)
            elif profile['method'] == 'confirm':
                if not recorded:
                    raise Usage('no recorded R for %s in %s' % (row_id, rates_path))
                result = run_confirm(ctx, row_id, recorded)
            else:
                result = run_search_row(ctx, row_id, recorded, caps)
            ctx.persist_row(row_id, result)
            rows.append(result)
        if profile.get('overload'):
            r2 = next((r for r in rows if r['id'] == 'R2' and r.get('r')), None)
            done = ctx.state.get('rows', {}).get('R4')
            if done and done.get('verdict'):
                rows.append(done)
            elif r2:
                result = run_overload(ctx, profiles.quantize(r2['r']))
                ctx.persist_row('R4', result)
                rows.append(result)
        verdicts = [r['verdict'] for r in rows]
        reasons = [r['reason'] for r in rows if r.get('reason')]
        if 'fail' in verdicts:
            status, code = 'FAIL', EXIT_FAIL
        elif 'infra' in verdicts:
            status, code = 'INFRA', EXIT_INFRA
        elif 'warn' in verdicts:
            status, code = 'WARN', EXIT_PASS
        else:
            status, code = 'PASS', EXIT_PASS
        if profile.get('report'):
            report = score.render_report(ctx, rows, env_id, (rates or {}).get('baseline_sps'))
        if args.record and status in ('PASS', 'WARN'):
            score.record_rates(rows, env_id, BASELINES, ctx, args.profile, out=args.record_out)
    except WallClock as error:
        log('stopping (exit 4, resume later):', error)
        stopped_for_wall = True
        return EXIT_WALL
    except Usage as error:
        log('usage:', error)
        status, reasons, code = 'INFRA', [str(error)], EXIT_USAGE
    except (Infra, AwsError) as error:
        log('infra:', error)
        status, reasons, code = 'INFRA', [str(error)[-800:]], EXIT_INFRA
        interrupted = isinstance(error, Interrupted)
    except Exception as error:  # noqa: BLE001  -- a harness bug is infra, never a DUT verdict
        log('error:', ''.join(traceback.format_exception(error))[-3000:])
        status, reasons, code = 'INFRA', ['harness error: %s' % error], EXIT_INFRA
    finally:
        SIGNALS['unwinding'] = True
        if not args.dry_run and (have_lock or ctx.state.get('lane')):
            try:
                ctx.lane.teardown(wait=not interrupted)
            except Exception as error:  # noqa: BLE001
                log('WARNING teardown failed; the janitor reaps by rvoip-expires-at:', error)
        if not args.dry_run:
            try:
                if stopped_for_wall and args.reserve_capacity:
                    ctx.lane.hold_reservations()
                else:
                    ctx.lane.release_reservations()
            except Exception as error:  # noqa: BLE001
                log('WARNING reservation release failed; each ends by itself at the lane expiry:', error)
            if have_lock:
                try:
                    aws.s3_rm(lock)
                except AwsError as error:
                    log('WARNING lock release:', error)
    if args.dry_run:
        return code   # a dry run leaves no receipt and nothing in S3
    receipt = receipt_for(ctx, rows, started, status, reasons, env_id, rates_path, report)
    try:
        write_manifest(ctx, receipt)
    except Exception as error:  # noqa: BLE001
        log('WARNING manifest:', error)
    if code in (EXIT_PASS, EXIT_FAIL):
        # A verdict is final: a later --resume of this run-id only rewrites this receipt.
        ctx.state['final'] = {'status': status, 'receipt': receipt, 'at': utc()}
        args.save_state(ctx.state)
    Path(args.receipt).parent.mkdir(parents=True, exist_ok=True)
    Path(args.receipt).write_bytes(canonical(receipt))
    try:
        aws.s3_cp(args.receipt, ctx.s3_run + 'receipt.json')
    except AwsError as error:
        log('WARNING receipt upload:', error)
    log('status', status, 'exit', code, '; '.join(reasons))
    return code


if __name__ == '__main__':
    sys.exit(main())
