#!/usr/bin/env python3
"""End-to-end probe of run_lane's capacity-reservation paths on real EC2, at the smallest shape.

  AWS_PROFILE=<admin> python3 scripts/bench/odcr_probe.py [--az us-west-2c] [--type c8i.large] [--max-minutes 20]

One reservation for 1x --type in the bench placement group in --az, driven through run_lane.Lane's own
methods: create -> targeted launch -> --max-wall hold (end date shortened, held tag) -> the resumed
leg's sweep spares it -> adopt (end date extended, tag flipped) -> second targeted launch -> release
(cancel) -> a fresh sweep finds nothing for the env tag. About $0.05; hard stop at --max-minutes;
every exit terminates the probe instances and cancels the reservation. Writes nothing to S3. Reads the
bench Terraform outputs, so run it from a checkout with infra/aws/bench applied. Exit 0 = every step
held; the last line is `RESULT {json}` with the reservation and instance ids.

Run it from an admin session: the release provisioner role trusts only GitHub OIDC (its check is
`aws iam simulate-principal-policy`, see crates/sip/rvoip-sip/docs/RFC7502_BENCHMARK.md §6).
"""
import argparse
import json
from pathlib import Path
import sys
import time
import types

BENCH = Path(__file__).resolve().parent
sys.path.insert(0, str(BENCH))
import run_lane  # noqa: E402
from aws import Aws  # noqa: E402

p = argparse.ArgumentParser()
p.add_argument('--az', default='us-west-2c')
p.add_argument('--type', default='c8i.large')
p.add_argument('--max-minutes', type=float, default=20)
a = p.parse_args()

T0 = time.time()
RUN_ID = 'odcr-probe-%s' % time.strftime('%Y%m%dT%H%M%SZ', time.gmtime())
ENV_TAG = 'odcr-probe'
results = {'run_id': RUN_ID, 'az': a.az, 'type': a.type, 'steps': []}


def log(*parts):
    print(time.strftime('%H:%M:%S'), *parts, flush=True)


def step(name, **facts):
    facts['t_s'] = round(time.time() - T0)
    results['steps'].append(dict(step=name, **facts))
    log('STEP', name, json.dumps(facts))


def deadline_check():
    if time.time() - T0 > a.max_minutes * 60:
        raise RuntimeError('hard stop: %.0f min' % a.max_minutes)


aws = Aws('us-west-2', log=log)
state = {'run_id': RUN_ID}
args = types.SimpleNamespace(
    run_id=RUN_ID, profile='smoke', reserve_capacity=True, az=a.az, env_tag=ENV_TAG, resume=False,
    dut_type=a.type, caller_type=a.type, ea_host_type=None, callers=0, lane_config=None,
    save_state=lambda s: None)
lane = run_lane.Lane(args, aws, log, state)
tf = lane.tf_outputs()
instances = []


def reservation(cid):
    r = aws.describe_capacity_reservations([cid])[0]
    return {'state': r['State'], 'end': r.get('EndDate'), 'total': r['TotalInstanceCount'],
            'available': r['AvailableInstanceCount'], 'pg': r.get('PlacementGroupArn'),
            'tags': {t['Key']: t['Value'] for t in r.get('Tags', [])}}


def launch_into(cid, leg):
    expires = run_lane.utc(T0 + a.max_minutes * 60 + 600)
    moved = aws.template_in_az(tf['dut_launch_template'], a.az)
    ids = aws.run_from_template(
        tf['dut_launch_template'], 1,
        {'rvoip-bench': 'true', 'managed-by': 'github-actions', 'rvoip-run-id': RUN_ID,
         'rvoip-expires-at': expires, 'Name': 'rvoip-bench-odcr-probe', 'rvoip-role': 'probe'},
        instance_type=a.type, capacity_reservation=cid, **moved)
    instances.extend(ids)
    inst = aws.wait_running(ids, timeout=300)[0]
    used = (inst.get('CapacityReservationId'), inst['Placement'].get('AvailabilityZone'),
            inst['Placement'].get('GroupName'))
    step('launch-%d' % leg, instance=ids[0], reservation_used=used[0], az=used[1], placement_group=used[2],
         reservation=reservation(cid))
    assert used[0] == cid, 'instance not in the reservation: %s' % (used,)
    aws.terminate(ids)
    for _ in range(60):
        deadline_check()
        if aws.describe_instances(ids)[0]['State']['Name'] == 'terminated' and reservation(cid)['available'] == 1:
            break
        time.sleep(5)
    step('terminated-%d' % leg, reservation=reservation(cid))


code = 1
try:
    # Leg 1: reserve, launch into it.
    exp1 = run_lane.utc(T0 + 30 * 60)      # the reservation's own end: a safety bound past the hard stop
    t1 = lane.reserve(tf, exp1)
    cid = t1['dut']
    assert t1['callers'] == cid
    step('created', id=cid, reservation=reservation(cid))
    deadline_check()
    launch_into(cid, 1)
    # The --max-wall stop: hold, end date shortened to min(now + HOLD, lane expiry).
    state['lane'] = {'expires_at': exp1}
    run_lane.HOLD_SECONDS = 15 * 60
    lane.hold_reservations()
    held = reservation(cid)
    step('held', id=cid, state_entry=state['reservations'][0], reservation=held)
    assert held['state'] == 'active' and held['tags'].get('rvoip-held-for-resume') == 'true'
    assert state['reservations'][0]['held_for_resume'] and state['reservations'][0]['ends_at'] < exp1
    deadline_check()
    # Leg 2: resume -> the sweep spares the hold, reserve() adopts it with a later end date.
    args.resume = True
    lane.sweep_reservations()
    assert reservation(cid)['state'] == 'active', 'the sweep cancelled the hold'
    exp2 = run_lane.utc(time.time() + 35 * 60)
    t2 = lane.reserve(tf, exp2)
    adopted = reservation(cid)
    step('adopted', id=t2['dut'], reservation=adopted)
    assert t2['dut'] == cid and adopted['tags'].get('rvoip-held-for-resume') == 'false'
    assert len(state['reservations']) == 1, 'adoption created a new reservation'
    deadline_check()
    launch_into(cid, 2)
    # Teardown: cancel; then a fresh run's sweep for the env tag finds nothing.
    lane.release_reservations()
    step('released', reservation=reservation(cid))
    assert reservation(cid)['state'] == 'cancelled'
    fresh = run_lane.Lane(types.SimpleNamespace(**dict(vars(args), resume=False, run_id=RUN_ID + '-sweep')),
                          aws, log, {'run_id': RUN_ID + '-sweep'})
    left = aws.describe_capacity_reservations(filters=(('tag:rvoip-bench', 'true'), ('tag:rvoip-env-id', ENV_TAG),
                                                       ('state', 'active,pending')))
    fresh.sweep_reservations()
    step('sweep', active_for_env=[r['CapacityReservationId'] for r in left])
    assert not left
    code = 0
except Exception as error:  # noqa: BLE001
    log('PROBE FAILED:', repr(error))
    results['error'] = repr(error)
finally:
    if instances:
        try:
            aws.terminate(instances)
        except Exception as error:  # noqa: BLE001
            log('WARNING terminate:', error)
    lane.release_reservations()
    for r in state.get('reservations', []):
        try:
            r['final'] = reservation(r['id'])['state']
        except Exception as error:  # noqa: BLE001
            r['final'] = 'unknown: %s' % error
    results['reservations'] = state.get('reservations', [])
    results['instances'] = instances
    results['minutes'] = round((time.time() - T0) / 60, 1)
    results['ok'] = code == 0
    print('RESULT ' + json.dumps(results))
sys.exit(code)
