"""summarize_trial on synthetic dut/ + caller-<i>/ trial directories."""
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
sys.path.insert(0, str(HERE))
import bench_fixtures as fx  # noqa: E402
import rfc7502  # noqa: E402
import search  # noqa: E402
import summarize_trial  # noqa: E402


def quiet(fn):
    """Run a CLI entry point with its stdout captured."""
    import contextlib
    import io
    with contextlib.redirect_stdout(io.StringIO()):
        return fn()


class SummarizeTests(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())

    def tearDown(self):
        shutil.rmtree(self.tmp)

    def trial(self, rate=40, **kw):
        d = self.tmp / ('t-%d-%d' % (rate, len(list(self.tmp.iterdir()))))
        profile = fx.make_trial(d, rate, **kw)
        return d, profile

    def test_clean_signaling_trial_over_two_hosts(self):
        d, profile = self.trial(40)
        s = summarize_trial.summarize(d, 'rfc')
        self.assertEqual(s['schema'], summarize_trial.SCHEMA)
        self.assertEqual(s['verdict'], 'pass', s['checks'])
        self.assertEqual(s['checks_missing'], {})
        names = [row['name'] for row in s['stage_summaries']]
        self.assertEqual(names, ['warmup', 'trial-1'])
        trial = s['stage_summaries'][1]
        self.assertEqual(trial['whole_stage']['offered'], 40 * trial['seconds'])
        self.assertEqual(trial['planned_offers'], 40 * trial['seconds'])
        self.assertEqual(trial['whole_stage']['rfc_attempt_ok'], trial['whole_stage']['offered'])
        self.assertEqual(s['trial_window']['start_seconds'], profile['stages'][0]['seconds'])
        self.assertEqual(s['rfc6076']['scored']['invites'], trial['whole_stage']['offered'])
        self.assertEqual(s['rfc6076']['scored']['ser_pct'], 100.0)
        self.assertEqual(s['rfc6076']['all_calls']['invites'], sum(r['whole_stage']['offered'] for r in s['stage_summaries']))
        self.assertEqual(s['setup_ms']['count'], trial['whole_stage']['offered'])
        self.assertIsNotNone(s['setup_ms']['p99_9'])
        self.assertEqual(s['media'], {'applicable': False})
        self.assertTrue(s['caller_validity']['caller_valid_before_backpressure'])
        self.assertEqual(sorted(s['caller_validity_by_caller']), ['caller-1', 'caller-2'])
        self.assertEqual(s['caller_validity']['process_core_limit'], 36.0, '0.75 x 48 physical cores')
        self.assertIsNone(s['backpressure']['first_second'])
        self.assertEqual(s['dut']['true_zero'], True)
        self.assertEqual(s['dut']['peak_cpu_cores'], 4.0)
        self.assertEqual(s['dut']['responses_503'], 0)
        self.assertEqual(s['generator']['hosts'], 2)
        self.assertEqual(s['generator']['sources'], 4)
        self.assertEqual(s['evaluation']['outcome'], 'pass')
        self.assertIn('loadgen-call', s['resource']['caller-1']['scored_thread_groups'])
        self.assertIn('tokio-rt-worker', s['resource']['dut']['scored_thread_groups'])
        attempted, succeeded, valid, detail = rfc7502.evaluate_trial(s)
        self.assertEqual((attempted, succeeded, valid), (trial['whole_stage']['offered'],) * 2 + (True,))
        self.assertEqual(detail['caller']['sources'], 4)
        self.assertEqual(detail['caller']['flows'], 32, '2 hosts x 2 sources x 8 connections')
        self.assertEqual(search.halt_reasons(s), [])

    def test_dut_503s_fail_the_trial_and_mark_backpressure(self):
        d, profile = self.trial(40, limit=30)
        s = summarize_trial.summarize(d, 'rfc')
        warmup = profile['stages'][0]['seconds']
        self.assertEqual(s['backpressure']['first_503_second'], warmup)
        self.assertEqual(s['backpressure']['first_second'], warmup)
        self.assertEqual(s['final_status_counts']['503'], s['dut']['responses_503'])
        self.assertEqual(s['evaluation']['outcome'], 'fail')
        self.assertTrue(s['evaluation']['reasons'][0].startswith('%d of' % s['dut']['responses_503']))
        self.assertTrue(s['caller_validity']['caller_valid_before_backpressure'])
        self.assertEqual(s['verdict'], 'pass', 'evidence checks pass; the 503s are the trial outcome')

    def test_missing_dut_summary_is_reported_missing_and_halts_a_search(self):
        d, _ = self.trial(40, dut_kwargs={'skip': ('summary.json',)})
        s = summarize_trial.summarize(d)
        self.assertIsNone(s['checks']['dut_true_zero'])
        self.assertIn('dut_true_zero', s['checks_missing'])
        self.assertEqual(s['verdict'], 'incomplete')
        self.assertIn('summary.json', s['dut']['missing'])
        self.assertEqual(s['dut']['responses_503'], 0, 'falls back to the Prometheus snapshot')
        self.assertEqual(s['dut']['responses_503_source'], 'metrics-final.prom')
        self.assertTrue(search.halt_reasons(s))
        self.assertEqual(search.halt_reasons(s, {'halt_on_missing_dut_evidence': False}), [])

    def test_dut_not_true_zero_halts(self):
        d, _ = self.trial(40, dut_kwargs={'true_zero': False})
        s = summarize_trial.summarize(d)
        self.assertIs(s['checks']['dut_true_zero'], False)
        self.assertEqual(s['verdict'], 'fail')
        self.assertEqual(search.halt_reasons(s), ['dut_true_zero is false'])

    def test_missing_caller_events_invalidate_the_trial(self):
        d, _ = self.trial(40, skip_callers=(2,))
        s = summarize_trial.summarize(d, 'rfc')
        self.assertIs(s['checks']['caller_evidence_complete'], False)
        self.assertEqual(s['evidence']['callers_missing'], ['caller-2'])
        self.assertIn('events.jsonl', s['evidence']['missing']['caller-2'])
        self.assertEqual(s['evaluation']['outcome'], 'invalid')
        self.assertTrue(any('caller evidence missing' in r for r in s['evaluation']['reasons']))

    def test_hot_caller_thread_invalidates(self):
        d, _ = self.trial(40, caller_kwargs={'thread_cores': 0.9})
        s = summarize_trial.summarize(d, 'rfc')
        self.assertIs(s['caller_validity']['checks']['no_thread_above_0_8_core'], False)
        self.assertEqual(s['evaluation']['outcome'], 'invalid')
        self.assertIn('caller check no_thread_above_0_8_core failed', s['evaluation']['reasons'])

    def test_caller_dispatch_miss_inside_the_trial_invalidates(self):
        d, profile = self.trial(40, caller_kwargs={'drop_bins': (7,)})
        s = summarize_trial.summarize(d, 'rfc')
        self.assertIs(s['caller_validity']['checks']['dispatch_within_1pct_per_bin_before_backpressure'], False)
        self.assertTrue(all(m['caller'] in ('caller-1', 'caller-2') for m in s['caller_validity']['dispatch_misses']))
        self.assertEqual(s['evaluation']['outcome'], 'invalid')

    def test_start_skew_over_two_seconds_fails_its_check(self):
        d, _ = self.trial(40, skew=2.5)
        s = summarize_trial.summarize(d)
        self.assertEqual(s['caller_start_skew_seconds'], 2.5)
        self.assertIs(s['checks']['caller_start_skew'], False)
        self.assertEqual(s['callers']['caller-2']['start_offset_seconds'], 2.5)

    def test_media_row_reports_echo_validation(self):
        d, _ = self.trial(40, media=True, hold_ms=1000, caller_kwargs={'audio_fail_every': 50})
        s = summarize_trial.summarize(d)
        media = s['media']
        self.assertTrue(media['applicable'])
        self.assertEqual(media['established'], s['rfc6076']['scored']['established'])
        self.assertGreater(media['audio_failures'], 0)
        self.assertEqual(media['validated_audio'] + media['audio_failures'], media['established'])
        self.assertTrue(s['checks']['media_echo_observed'])
        self.assertIs(s['checks']['media_all_audio_validated'], False)
        self.assertIs(s['checks']['dut_srtp_unprotect_failures_zero'], True)
        self.assertEqual(rfc7502.trial_verdict(s, 'rfc')['outcome'], 'pass', 'media is reported, not scored, under rfc')
        self.assertEqual(rfc7502.trial_verdict(s, 'full')['outcome'], 'fail')
        self.assertEqual(s['rfc6076']['scored']['sdt_ms']['p50'], 1000.0)

    def test_disconnect_failures_are_counted_not_failed(self):
        d, _ = self.trial(40, caller_kwargs={'bye_status': 481})
        s = summarize_trial.summarize(d, 'rfc')
        self.assertEqual(s['disconnect_failures']['scored'], s['rfc6076']['scored']['invites'])
        self.assertEqual(s['evaluation']['outcome'], 'pass')
        self.assertEqual(s['evaluation']['detail']['disconnect_failures'], s['rfc6076']['scored']['invites'])

    def test_without_plan_no_stage_is_scored(self):
        d, _ = self.trial(40, plan=False)
        s = summarize_trial.summarize(d, 'rfc')
        self.assertIn('plan.json', s['evidence']['missing']['trial'])
        self.assertTrue(s['stage_problems'])
        self.assertIs(s['checks']['scored_stage_present'], False)
        self.assertEqual(s['evaluation']['outcome'], 'invalid')

    def test_testbed_baseline_skips_dut_drain_checks(self):
        d, _ = self.trial(40, dut_kwargs={'skip': ('summary.json', 'ready.json')})
        plan = json.loads((d / 'plan.json').read_text())
        (d / 'plan.json').write_text(json.dumps(dict(plan, dut_kind='ea-uas')))
        s = summarize_trial.summarize(d)
        self.assertNotIn('dut_true_zero', s['checks'])
        self.assertIn('dut_true_zero', s['checks_not_applicable'])
        self.assertEqual(search.halt_reasons(s), [])
        self.assertEqual(s['verdict'], 'pass')

    def test_cli_writes_merged_summary(self):
        d, _ = self.trial(40)
        self.assertEqual(quiet(lambda: summarize_trial.main([str(d)])), 0)
        written = json.loads((d / 'merged' / 'summary.json').read_text())
        self.assertEqual(written['evaluation']['outcome'], 'pass')
        self.assertEqual(written['schema'], summarize_trial.SCHEMA)


if __name__ == '__main__':
    unittest.main()
