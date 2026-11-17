"""caller_policy (thread policy and validity predicate) and merge_callers (N caller hosts)."""
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
import caller_policy  # noqa: E402
import merge_callers  # noqa: E402

REPORT_OK = {'shutdown_outcome': 'ok', 'interrupted': False, 'caller_watchdogs': 0, 'generator_skipped': 0, 'hold_ms': 0}


def samples(start, seconds, process_cores=2.0, thread_cores=0.2, hot_from=None, tail_report_second=None):
    rows = []
    for i in range(seconds + 1):
        hot = 0.0 if hot_from is None or i < hot_from else (i - hot_from + 1) * 0.9
        rows.append({'at': start + i, 'cpu_cores_pct': 100 * process_cores if i else None,
                     'thread_cpu_times': [{'id': 1, 'user_time': thread_cores * i + hot, 'system_time': 0.0},
                                          {'id': 2, 'user_time': thread_cores * i, 'system_time': 0.0}],
                     'thread_names': {'1': 'loadgen-call-0', '2': 'loadgen-signaling'}})
    if tail_report_second is not None:
        rows.append({'at': start + tail_report_second, 'cpu_cores_pct': 100 * 40.0,
                     'thread_cpu_times': [{'id': 1, 'user_time': 1000.0, 'system_time': 0.0}],
                     'thread_names': {'1': 'loadgen-main'}})
    return rows


def bins(seconds, rate, misses=()):
    return [{'t': t, 'requested_cps': rate, 'dispatched': rate - (5 if t in misses else 0), 'guard_skips': 0}
            for t in range(seconds)]


def quiet(fn):
    """Run a CLI entry point with its stdout captured."""
    import contextlib
    import io
    with contextlib.redirect_stdout(io.StringIO()):
        return fn()


class PolicyTests(unittest.TestCase):
    def test_policies_fill_their_hosts(self):
        for cpus in (48, 96, 192):
            env = caller_policy.policy_for(cpus)
            self.assertEqual(caller_policy.validate(env, cpus), cpus)
            self.assertTrue(all(key.startswith('RVOIP_') for key in env))
        self.assertEqual(caller_policy.expected_thread_groups(caller_policy.POLICY_96),
                         {'loadgen-signaling': 8, 'loadgen-call': 48, 'rvoip-media': 40})
        with self.assertRaises(ValueError):
            caller_policy.validate({'RVOIP_LOADGEN_LANES': '8'}, 96)
        with self.assertRaises(ValueError):
            caller_policy.validate(caller_policy.POLICY_192, 96)
        with self.assertRaises(ValueError):
            caller_policy.policy_for(64)

    def test_thread_groups(self):
        self.assertEqual(caller_policy.thread_group('loadgen-call-7'), 'loadgen-call')
        self.assertEqual(caller_policy.thread_group('loadgen-signaling'), 'loadgen-signaling')
        self.assertEqual(caller_policy.thread_group('tokio-x'), 'tokio-x')
        # /proc comm keeps 15 bytes.
        self.assertEqual(caller_policy.thread_group('loadgen-signali'), 'loadgen-signaling')
        self.assertEqual(caller_policy.thread_group('loadgen-validat'), 'loadgen-validation')
        self.assertEqual(caller_policy.thread_group('loadgen-call-12'), 'loadgen-call')
        self.assertEqual(caller_policy.thread_group('loadgen-sig'), 'loadgen-sig')
        self.assertEqual(caller_policy.observed_thread_groups(samples(0, 2)), {'loadgen-call': 1, 'loadgen-signaling': 1})


class PredicateTests(unittest.TestCase):
    def test_valid_caller(self):
        v = caller_policy.predicate(bins(10, 100), [0.5] * 100, samples(1000, 12), REPORT_OK, physical_cores=48,
                                    run_start_epoch=1000)
        self.assertTrue(v['caller_valid'], v['checks'])
        self.assertEqual(v['process_core_limit'], 36.0)
        self.assertAlmostEqual(v['thread_peak_cores'], 0.2)

    def test_each_threshold(self):
        late = caller_policy.predicate(bins(10, 100), [6.0] * 100, samples(1000, 12), REPORT_OK, physical_cores=48,
                                       run_start_epoch=1000)
        self.assertFalse(late['checks']['schedule_to_write_p99_at_most_5ms'])
        busy = caller_policy.predicate(bins(10, 100), [0.5], samples(1000, 12, process_cores=37), REPORT_OK, physical_cores=48,
                                       run_start_epoch=1000)
        self.assertFalse(busy['checks']['process_cpu_at_most_limit'])
        hot = caller_policy.predicate(bins(10, 100), [0.5], samples(1000, 12, hot_from=5), REPORT_OK, physical_cores=48,
                                      run_start_epoch=1000)
        self.assertFalse(hot['checks']['no_thread_above_0_8_core'])
        self.assertEqual(hot['hottest_thread']['name'], 'loadgen-call-0')
        missed = caller_policy.predicate(bins(10, 100, misses=(3,)), [0.5], samples(1000, 12), REPORT_OK, physical_cores=48,
                                         run_start_epoch=1000)
        self.assertFalse(missed['checks']['dispatch_within_1pct_per_bin_before_backpressure'])
        self.assertEqual(missed['dispatch_misses'], [{'t': 3, 'requested': 100, 'dispatched': 95}])
        tolerated = caller_policy.predicate(bins(10, 50, misses=()), [0.5], samples(1000, 12), REPORT_OK, physical_cores=48,
                                            run_start_epoch=1000)
        self.assertTrue(tolerated['checks']['dispatch_within_1pct_per_bin_before_backpressure'])
        unclean = caller_policy.predicate(bins(10, 100), [0.5], samples(1000, 12), dict(REPORT_OK, shutdown_outcome='timeout'),
                                          physical_cores=48, run_start_epoch=1000)
        self.assertFalse(unclean['caller_valid_before_backpressure'])

    def test_judged_before_backpressure_and_during_generation_only(self):
        # misses after the first 503 are the DUT's; the end-of-run report building is not load generation
        v = caller_policy.predicate(bins(10, 100, misses=(6, 7, 8)), [0.5], samples(1000, 12, tail_report_second=14),
                                    dict(REPORT_OK, shutdown_outcome='timeout'), backpressure_second=6, physical_cores=48,
                                    run_start_epoch=1000)
        self.assertTrue(v['checks']['dispatch_within_1pct_per_bin_before_backpressure'])
        self.assertTrue(v['caller_valid_before_backpressure'], 'an unclean shutdown after backpressure is a DUT result')
        v = caller_policy.predicate(bins(10, 100), [0.5], samples(1000, 12, tail_report_second=14), REPORT_OK,
                                    physical_cores=48, run_start_epoch=1000)
        self.assertTrue(v['checks']['process_cpu_at_most_limit'], 'the 40-core report-building sample is outside generation')

    def test_missing_inputs_are_false_never_assumed(self):
        v = caller_policy.predicate(bins(10, 100), [0.5], [], REPORT_OK, physical_cores=None, run_start_epoch=1000)
        self.assertFalse(v['checks']['process_cpu_at_most_limit'])
        self.assertFalse(v['checks']['no_thread_above_0_8_core'])
        self.assertIn('physical_cores', v['missing_inputs'])
        self.assertIn('resources.jsonl', v['missing_inputs'])

    def test_combine_tags_misses_with_their_host(self):
        a = caller_policy.predicate(bins(10, 100, misses=(3,)), [0.5], samples(1000, 12), REPORT_OK, physical_cores=48,
                                    run_start_epoch=1000)
        b = caller_policy.predicate(bins(10, 100), [0.9], samples(1000, 12, thread_cores=0.5), REPORT_OK, physical_cores=48,
                                    run_start_epoch=1000)
        combined = caller_policy.combine({'caller-1': a, 'caller-2': b})
        self.assertFalse(combined['checks']['dispatch_within_1pct_per_bin_before_backpressure'])
        self.assertEqual(combined['dispatch_misses'][0]['caller'], 'caller-1')
        self.assertAlmostEqual(combined['thread_peak_cores'], 0.5)
        self.assertEqual(combined['hottest_thread']['caller'], 'caller-2')
        self.assertFalse(combined['caller_valid'])


class MergeTests(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())

    def tearDown(self):
        shutil.rmtree(self.tmp)

    def test_three_hosts_merge_onto_the_first_clock(self):
        fx.make_trial(self.tmp, 60, hosts=3, skew=0.25)
        callers = [merge_callers.load_caller(d) for d in merge_callers.caller_dirs(self.tmp)]
        self.assertEqual([c['role'] for c in callers], ['caller-1', 'caller-2', 'caller-3'])
        merged = merge_callers.merge(callers)
        total = sum(len(c['events']['scheduled']) for c in callers)
        self.assertEqual(len(merged['scheduled']), total, 'namespaced ids never collide')
        self.assertTrue(any(cid.startswith('trial-c2-') for cid in merged['scheduled']))
        self.assertTrue(any(cid.startswith('trial-c3-') for cid in merged['scheduled']))
        self.assertEqual(merged['offsets'], {'caller-1': 0.0, 'caller-2': 0.25, 'caller-3': 0.25})
        self.assertEqual(merged['start_skew_seconds'], 0.25)
        # caller-2's first call (process offset 500 ms, run start +250 ms) lands 250 ms after caller-1's first
        first1 = min(callers[0]['events']['scheduled'].values())
        first2 = min(v for k, v in merged['scheduled'].items() if k.startswith('trial-c2-'))
        self.assertEqual(first2 - first1, 250)
        self.assertEqual(merged['config']['stages'][1]['cps'], 60)
        self.assertEqual(merged['config']['calls'], total)
        self.assertEqual(merged['report']['attempted'], total)
        self.assertEqual(merged['report']['endpoints'], 48)
        self.assertEqual(len(merged['report']['sources']), 6)
        self.assertEqual(merged['report']['rfc6076_trace']['records'], total)
        self.assertEqual(merged['report']['verdict'], 'pass')

    def test_namespace(self):
        self.assertEqual(merge_callers.namespace('run-7', 'run', 'c2'), 'run-c2-7')
        self.assertEqual(merge_callers.namespace('other-7', 'run', 'c2'), 'c2-other-7')
        self.assertEqual(merge_callers.namespace('run-7', 'run', ''), 'run-7')

    def test_missing_caller_files_are_named(self):
        fx.make_trial(self.tmp, 40, skip_callers=(2,))
        callers = [merge_callers.load_caller(d) for d in merge_callers.caller_dirs(self.tmp)]
        self.assertEqual(callers[1]['missing'], ['caller.json', 'report.json', 'events.jsonl'])
        merged = merge_callers.merge(callers)
        self.assertEqual(merged['roles'], ['caller-1'])

    def test_cli_writes_the_merged_view(self):
        fx.make_trial(self.tmp, 40)
        self.assertEqual(quiet(lambda: merge_callers.main([str(self.tmp)])), 0)
        out = self.tmp / 'merged' / 'caller'
        lines = (out / 'events.jsonl').read_text().splitlines()
        self.assertEqual(sum('"run_started"' in line for line in lines), 1, 'run-level singletons once')
        report = json.loads((out / 'report.json').read_text())
        self.assertEqual(report['merged_from_callers'], ['caller-1', 'caller-2'])
        self.assertEqual(json.loads((out / 'merge.json').read_text())['start_skew_seconds'], 0.0)


if __name__ == '__main__':
    unittest.main()
