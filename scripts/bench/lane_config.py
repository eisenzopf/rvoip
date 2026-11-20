#!/usr/bin/env python3
"""The pinned lane config: infra/aws/bench's Terraform outputs, committed as infra/aws/bench/lanes/<env-id>.json.

  lane_config.py write [--env-id <id>] [--tf-dir <dir>]   regenerate it from the Terraform state
  lane_config.py check [--env-id <id>] [--tf-dir <dir>]   exit 1 when it differs from the state (drift)
  lane_config.py show                                     print what write would write

run_lane.py reads it instead of `terraform output`, so a lane runs where there is no Terraform state (a hosted
CI runner). The config names its env-id and the sha256 of that env-id's env.json, and run_lane refuses one that
does not match; at run time it also checks the config against EC2 (the launch templates' AMI and subnet). Run
write or check where the bench Terraform state is, after any apply of infra/aws/bench or any re-pin.
Nothing here is secret: names and ids only.
"""
import argparse
import datetime
import json
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import run_lane  # noqa: E402

SCHEMA = 'rvoip-bench-lane-v1'
TF_DIR = run_lane.REPO / 'infra/aws/bench'


def terraform(*args, tf_dir=TF_DIR):
    out = subprocess.run(['terraform', '-chdir=%s' % tf_dir, *args], capture_output=True, text=True)
    if out.returncode:
        raise SystemExit('terraform %s failed: %s' % (' '.join(args), out.stderr.strip()[-500:]))
    return json.loads(out.stdout)


def build(outputs, state, env_id, env_json_sha256=None):
    """The lane config from `terraform output -json` and `terraform show -json` (pure: tested offline)."""
    config = {'schema': SCHEMA, 'env_id': env_id, 'env_json_sha256': env_json_sha256}
    config.update({k: v['value'] for k, v in sorted(outputs.items())})
    resources = (((state or {}).get('values') or {}).get('root_module') or {}).get('resources') or []
    for r in resources:
        if r.get('type') == 'aws_launch_template' and r.get('name') in ('dut', 'caller'):
            config['%s_launch_template_id' % r['name']] = r['values']['id']
        if r.get('address') == 'data.aws_subnets.release_public':
            config['release_public_subnet_ids'] = sorted(r['values'].get('ids') or [])
    return config


def comparable(config):
    return {k: v for k, v in config.items() if k != 'generated_at'}


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument('action', choices=('write', 'check', 'show'))
    p.add_argument('--env-id', help='default: perf-baselines/rfc7502/ENVIRONMENT')
    p.add_argument('--tf-dir', default=str(TF_DIR), help='the infra/aws/bench directory holding the state')
    a = p.parse_args(argv)
    env_id = a.env_id or (run_lane.BASELINES / 'ENVIRONMENT').read_text().strip()
    env_dir = run_lane.BASELINES / env_id
    if not (env_dir / 'env.json').is_file():
        raise SystemExit('no env.json for %s' % env_id)
    fresh = build(terraform('output', '-json', tf_dir=a.tf_dir), terraform('show', '-json', tf_dir=a.tf_dir), env_id,
                  run_lane.sha256_file(env_dir / 'env.json'))
    path = run_lane.lane_config_path(env_id)
    if a.action == 'show':
        print(json.dumps(fresh, indent=1, sort_keys=True))
        return 0
    if a.action == 'check':
        current = json.loads(path.read_text()) if path.is_file() else {}
        drift = sorted(k for k in set(fresh) | set(current) if k != 'generated_at'
                       and fresh.get(k) != current.get(k))
        if drift:
            print('lane config drift for %s: %s' % (env_id, ', '.join(drift)), file=sys.stderr)
            return 1
        print('lane config for %s matches the Terraform state' % env_id)
        return 0
    fresh['generated_at'] = datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(fresh, indent=1, sort_keys=True) + '\n')
    print(json.dumps({'lane_config': str(path), 'env_json_sha256': fresh['env_json_sha256']}))
    return 0


if __name__ == '__main__':
    sys.exit(main())
