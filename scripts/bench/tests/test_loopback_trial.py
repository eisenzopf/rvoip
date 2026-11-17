"""loopback_trial.build: run_lane's plan on the fake loopback lane, with only ports and paths changed."""
import argparse
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import loopback_trial  # noqa: E402
import rfc7502  # noqa: E402


def args(row, rate=400, admission=None):
    return argparse.Namespace(row=row, rate=rate, seconds=20, warmup=10, admission_max_cps=admission, linger=None,
                              run_id='lb-test', rel='%s/t00-r%d' % (row, rate), out=Path('/tmp/lb'))


class BuildTest(unittest.TestCase):
    def test_plan_matches_run_lane_with_loopback_ports(self):
        plan = loopback_trial.build(args('R2', 200), Path('/tmp/lb/R2/t00-r200'))
        self.assertEqual([(s['name'], s['cps'], s['seconds']) for s in plan['profile']['stages']],
                         [('warmup', 100, 10), ('trial-1', 200, 20)])
        rfc7502.validate_shares(plan['profile'], 2)
        dut = plan['dut_config']
        self.assertEqual((dut['transport'], dut['media'], dut['bind']), ('tls', 'srtp', '127.0.0.1:5061'))
        self.assertEqual(dut['rtp_port_range'], [10000, 29999])
        self.assertEqual(dut['run_dir'], '/tmp/lb/R2/t00-r200/dut')
        self.assertEqual(dut['admission'], {'mode': 'off'})
        one, two = plan['caller_configs']
        self.assertEqual((one['bind_ips'], two['bind_ips']), (['127.0.0.2', '127.0.0.3'], ['127.0.0.4', '127.0.0.5']))
        self.assertEqual((one['media_port_start'], one['media_port_end']), (30000, 44999))
        self.assertEqual((two['media_port_start'], two['media_port_end']), (45000, 59999))
        self.assertEqual((one['metrics_listen'], two['metrics_listen']), ('127.0.0.2:9465', '127.0.0.4:9466'))
        self.assertEqual([s['cps'] for s in one['stages']], [50, 100])
        self.assertEqual(one['calls'] + two['calls'], 100 * 10 + 200 * 20)
        self.assertEqual(one['output'], '/tmp/lb/R2/t00-r200/caller-1')
        self.assertTrue(one['run_id'].endswith('-h1') and two['run_id'].endswith('-h2'))
        self.assertEqual(set(plan['caller_hosts']), {'caller-1', 'caller-2'})

    def test_admission(self):
        plan = loopback_trial.build(args('R1', 400, admission=200), Path('/tmp/lb/R1/t00-r400'))
        self.assertEqual(plan['dut_config']['admission'], {'mode': 'max-cps', 'max_cps': 200, 'retry_after_secs': 2})
        self.assertEqual(plan['admission'], plan['dut_config']['admission'])
        self.assertTrue(plan['caller_configs'][0]['signaling_only'])


if __name__ == '__main__':
    unittest.main()
