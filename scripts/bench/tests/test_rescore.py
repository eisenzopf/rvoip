"""rescore_run.py against the real Day 0 bench-minor run (45065590604): its receipt and every trial's summary.json,
trimmed to what the verdicts read (fixtures/day0-bench-minor-45065590604.json.gz). The fixed scoring turns R1/R5's
"tester-invalid" stops into DUT fails (a setup-delay collapse) and R4's recovery into a pass; R6's R has 2 of 3
confirmations and is recorded as such."""
import gzip
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import rescore_run  # noqa: E402
import rfc7502  # noqa: E402
import run_lane  # noqa: E402

FIXTURE = json.loads(gzip.decompress((HERE / 'fixtures/day0-bench-minor-45065590604.json.gz').read_bytes()))
RECEIPT = FIXTURE['receipt']
S3_RUN = RECEIPT['evidence']['s3_prefix']


def fetch_summary(prefix):
    return FIXTURE['summaries'][rescore_run.rel_of(prefix, S3_RUN)]


def fetch_plan(prefix):
    return FIXTURE['plans'][rescore_run.rel_of(prefix, S3_RUN)]


class RescoreDay0Tests(unittest.TestCase):
    def setUp(self):
        self.results = rescore_run.rescore(RECEIPT, fetch_summary, fetch_plan)

    def test_the_receipt_is_the_failed_day0_run(self):
        self.assertEqual((RECEIPT['status'], RECEIPT['profile']), ('FAIL', 'minor'))
        verdicts = {r['id']: r['verdict'] for r in RECEIPT['rows']}
        self.assertEqual(verdicts, {'R1': 'infra', 'R2': 'pass', 'R5': 'infra', 'R6': 'fail', 'R4': 'fail'})

    def test_r1_and_r5_stop_at_a_dut_setup_delay_collapse_not_the_tester(self):
        for row_id, r, failing in (('R1', 8348, 9180), ('R5', 8256, 9080)):
            res = self.results[row_id]
            self.assertEqual((res['r'], res['confirmations'], res['lowest_failing_sps']), (r, 1, failing), row_id)
            self.assertEqual([t['outcome'] for t in res['trials'] if t['rate'] == failing], ['fail'] * 3)
            self.assertNotIn('invalid', [t['outcome'] for t in res['trials']])
        summary = fetch_summary(next(t['evidence'] for t in RECEIPT['rows'][0]['trials'] if t['rate'] == 9180))
        verdict = rfc7502.trial_verdict(summary)
        self.assertEqual(verdict['outcome'], 'fail')
        self.assertTrue(any('DUT setup delay collapsed' in reason for reason in verdict['reasons']))

    def test_a_saturated_caller_stays_tester_invalid_despite_slow_setup(self):
        """The collapse rule never excuses a caller whose own kernel dropped responses (softirq saturation)."""
        import copy
        summary = copy.deepcopy(fetch_summary(next(t['evidence'] for t in RECEIPT['rows'][0]['trials']
                                                   if t['rate'] == 9180)))
        summary['caller_validity']['checks']['no_irq_cpu_softirq_saturated'] = False
        self.assertEqual(rfc7502.trial_verdict(summary)['outcome'], 'invalid')

    def test_r2_is_unchanged(self):
        res = self.results['R2']
        self.assertEqual((res['r'], res['confirmations'], res['lowest_failing_sps']), (1124, 4, 1136))
        self.assertEqual(res['latency_rate'], 1008, 'the 0.9 R latency trial of the run')

    def test_r6_has_two_of_three_confirmations(self):
        res = self.results['R6']
        self.assertEqual((res['r'], res['confirmations'], res['confirmations_required']), (1204, 2, 3))
        self.assertEqual(res['lowest_failing_sps'], 1220, '1220 passed 3 times and failed once')

    def test_r4_recovery_passes_on_its_steady_window(self):
        self.assertEqual(self.results['R4']['verdict'], 'pass')
        self.assertEqual(self.results['R4']['previous_verdict'], 'fail')

    def test_record_writes_rows_naming_the_run_and_its_confirmations(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'rates.json'
            path.write_text(json.dumps({'schema': 'rvoip-rfc7502-rates-v1', 'rows': {},
                                        'tester_caps': {'R1': 9492, 'R2': 3124, 'R5': 9388, 'R6': 3432},
                                        'baseline_sps': {'R6': {'r': 5128, 'lower_bound': True}}}))
            rates = rescore_run.record(self.results, RECEIPT, path)
        rows = rates['rows']
        self.assertEqual({k: v['r'] for k, v in rows.items()}, {'R1': 8348, 'R2': 1124, 'R5': 8256, 'R6': 1204})
        self.assertEqual(rows['R6']['confirmations'], 2)
        self.assertEqual(rows['R6']['confirmations_required'], 3)
        self.assertTrue(rows['R6']['tester_cap_lower_bound'])
        for row in rows.values():
            self.assertEqual(row['source_run'], S3_RUN)
            self.assertEqual(row['candidate_sha'], RECEIPT['candidate_sha'])
            self.assertFalse(row['tester_limited'])
            self.assertIn('rescore_run.py', row['rescored']['tool'])
        self.assertEqual(rows['R2']['latency_rate'], 1008)
        self.assertIsNone(rows['R1']['srd_ms'], 'no latency trial ran for R1')


class RescoreRefusalTests(unittest.TestCase):
    def run_main(self, receipt, baselines):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'receipt.json'
            path.write_text(json.dumps(receipt))
            with mock.patch.object(run_lane, 'BASELINES', baselines), \
                    mock.patch('sys.stderr') as err, mock.patch('sys.stdout'):
                code = rescore_run.main(['--receipt', str(path), '--record'])
                text = ''.join(str(c.args[0]) for c in err.write.call_args_list)
        return code, text

    def test_refusals(self):
        with tempfile.TemporaryDirectory() as tmp:
            baselines = Path(tmp)
            env_id = RECEIPT['environment']['env_id']
            (baselines / 'ENVIRONMENT').write_text(env_id + '\n')
            (baselines / env_id).mkdir()
            for receipt, why in ((dict(RECEIPT, profile='release'), 'only a DUT search'),
                                 (dict(RECEIPT, environment={'env_id': 'aws-other'}), 'ENVIRONMENT is'),
                                 (dict(RECEIPT), 'no rates.json'),
                                 ):
                code, text = self.run_main(receipt, baselines)
                self.assertEqual(code, 2, text)
                self.assertIn(why, text)
            (baselines / env_id / 'rates.json').write_text('{}')
            code, text = self.run_main(dict(RECEIPT, status='INFRA'), baselines)
            self.assertEqual(code, 2)
            self.assertIn('never finished', text)


if __name__ == '__main__':
    unittest.main()
