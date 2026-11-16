"""search.run_search: convergence, resume, halt rules, voids, budgets, sanity gate, verdict.json."""
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import search  # noqa: E402


def summary(rate, limit, *, valid=True, true_zero=True, clean=True, missing_dut=False, offered=None):
    """A minimal merged summary: one trial-1 stage whose attempts above `limit` failed."""
    offered = rate * 10 if offered is None else offered
    ok = offered if rate <= limit else offered - 1
    checks = {'caller_evidence_complete': True}
    checks_missing = {}
    if missing_dut:
        checks.update(dut_true_zero=None, dut_clean_shutdown=None)
        checks_missing = {'dut_true_zero': 'dut/summary.json missing', 'dut_clean_shutdown': 'dut/summary.json missing'}
    else:
        checks.update(dut_true_zero=true_zero, dut_clean_shutdown=clean)
    return {'stage_summaries': [{'name': 'warmup', 'seconds': 30, 'whole_stage': {'offered': 0}},
                                {'name': 'trial-1', 'seconds': 180,
                                 'whole_stage': {'offered': offered, 'dispatched': offered, 'missing_terminal_observations': 0,
                                                 'rfc_attempt_ok': ok, 'rejected_503': offered - ok,
                                                 'rfc6076': {'final_status_counts': {'200': ok, '503': offered - ok}}}}],
            'caller_validity': {'caller_valid_before_backpressure': valid,
                                'checks': {} if valid else {'no_thread_above_0_8_core': False}},
            'checks': checks, 'checks_missing': checks_missing}


class FakeClock:
    def __init__(self, start=1_000_000.0, step=600.0):
        self.now, self.step = start, step

    def time(self):
        return self.now


class Driver:
    """run_trial for a deterministic DUT; `plan` maps trial index -> summary overrides."""

    def __init__(self, limit, clock=None, plan=None, trial_root=None, stop_after=None):
        self.limit, self.clock, self.plan = limit, clock, plan or {}
        self.calls, self.trial_root, self.stop_after = [], trial_root, stop_after

    def __call__(self, rate, index, profile):
        if self.stop_after is not None and len(self.calls) >= self.stop_after:
            raise KeyboardInterrupt('orchestrator stopped')
        self.calls.append((rate, index, profile))
        if self.clock:
            self.clock.now += self.clock.step
        kind = self.plan.get(len(self.calls) - 1, {})
        if kind == 'void':
            return search.TrialResult(summary=None, rc=3, note='guard refused')
        trial_dir = Path(self.trial_root) / ('t%02d-r%d' % (index, rate)) if self.trial_root else None
        return search.TrialResult(summary=summary(rate, self.limit, **kind), run='R1/t%02d-r%d' % (index, rate),
                                  trial_dir=trial_dir, wall_seconds=300)


def quiet(*args):
    pass


class SearchDriverTests(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.state, self.trials = self.tmp / 'R1' / 'search.json', self.tmp / 'R1' / 'trials.jsonl'

    def tearDown(self):
        shutil.rmtree(self.tmp)

    def spec(self, **kw):
        return dict({'row': 'R1', 'r0': 100, 'quantum': 1, 'minimum_trial_seconds': 180}, **kw)

    def test_converges_like_rfc7502_appendix_a(self):
        driver = Driver(460, trial_root=self.tmp / 'R1')
        result = search.run_search(self.spec(), self.state, self.trials, driver, log=quiet)
        self.assertEqual(result['result_sps'], 458)
        self.assertEqual(result['row'], 'R1')
        self.assertFalse(result['paused'])
        lines = self.trials.read_text().splitlines()
        self.assertEqual(len(lines), result['trials'])
        self.assertEqual(json.loads(lines[0])['detail']['run'], 'R1/t00-r100')
        self.assertEqual(json.loads(lines[0])['detail']['scored_seconds'], 500, 'max(N / r, 180 s) at 100 sps')
        verdict = json.loads((self.tmp / 'R1' / 't00-r100' / 'verdict.json').read_text())
        self.assertEqual((verdict['outcome'], verdict['rate'], verdict['next_rate']), ('pass', 100, 110))
        failing = [json.loads(l) for l in lines if json.loads(l)['outcome'] == 'fail'][0]
        verdict = json.loads((self.tmp / 'R1' / ('t%02d-r%d' % (failing['index'], failing['rate'])) / 'verdict.json').read_text())
        self.assertEqual(verdict['outcome'], 'fail')
        self.assertIn('"503": 1', verdict['reasons'][0])
        self.assertEqual(driver.calls[0][2]['stages'][0], {'name': 'warmup', 'cps': 50, 'seconds': 30})

    def test_resume_continues_the_saved_state(self):
        with self.assertRaises(KeyboardInterrupt):
            search.run_search(self.spec(), self.state, self.trials, Driver(460, stop_after=5), log=quiet)
        saved = json.loads(self.state.read_text())
        self.assertEqual(len(saved['trials']), 5)
        driver = Driver(460)
        result = search.run_search(self.spec(r0=999), self.state, self.trials, driver, log=quiet)
        self.assertEqual(driver.calls[0][1], 5, 'the next trial index continues')
        self.assertEqual(driver.calls[0][0], saved['r'], 'the saved next rate, not the new r0')
        self.assertEqual(result['result_sps'], 458)
        self.assertEqual(len(self.trials.read_text().splitlines()), result['trials'])

    def test_dut_not_true_zero_halts_after_recording(self):
        driver = Driver(460, plan={2: {'true_zero': False}})
        result = search.run_search(self.spec(), self.state, self.trials, driver, log=quiet)
        self.assertEqual(result['trials'], 3)
        self.assertIsNone(result['result_sps'])
        self.assertEqual(result['halted'], 'dut_true_zero is false')
        self.assertIn('halt: dut_true_zero is false', result['stop_reason'])
        self.assertTrue(result['done'])

    def test_unclean_shutdown_and_missing_dut_evidence_halt(self):
        result = search.run_search(self.spec(), self.state, self.trials, Driver(460, plan={0: {'clean': False}}), log=quiet)
        self.assertEqual(result['halted'], 'dut_clean_shutdown is false')
        shutil.rmtree(self.tmp / 'R1')
        result = search.run_search(self.spec(), self.state, self.trials, Driver(460, plan={0: {'missing_dut': True}}), log=quiet)
        self.assertIn('dut_true_zero missing', result['halted'])
        shutil.rmtree(self.tmp / 'R1')
        result = search.run_search(self.spec(halt_on_missing_dut_evidence=False), self.state, self.trials,
                                   Driver(460, plan={0: {'missing_dut': True}}), log=quiet)
        self.assertEqual(result['result_sps'], 458)

    def test_three_invalid_trials_in_a_row_stop(self):
        driver = Driver(460, plan={1: {'valid': False}, 2: {'valid': False}, 3: {'valid': False}})
        result = search.run_search(self.spec(), self.state, self.trials, driver, log=quiet)
        self.assertEqual([c[0] for c in driver.calls], [100, 110, 110, 110])
        self.assertIsNone(result['result_sps'])
        self.assertIn('3 consecutive invalid trials', result['stop_reason'])
        self.assertEqual(json.loads(self.trials.read_text().splitlines()[1])['outcome'], 'invalid')

    def test_two_void_trials_pause_without_recording(self):
        driver = Driver(460, plan={1: 'void', 2: 'void'})
        result = search.run_search(self.spec(), self.state, self.trials, driver, log=quiet)
        self.assertTrue(result['paused'])
        self.assertFalse(result['done'])
        self.assertIn('2 consecutive trials offered no session', result['halted'])
        self.assertEqual(result['trials'], 1, 'void trials are never recorded')
        one = Driver(460, plan={0: 'void'})
        resumed = search.run_search(self.spec(), self.state, self.trials, one, log=quiet)
        self.assertEqual(resumed['result_sps'], 458, 'a single void trial is retried at the same rate')
        self.assertEqual(one.calls[0][0], one.calls[1][0])

    def test_max_trials_and_max_hours(self):
        result = search.run_search(self.spec(max_trials=4), self.state, self.trials, Driver(460), log=quiet)
        self.assertIn('max_trials (4)', result['stop_reason'])
        shutil.rmtree(self.tmp / 'R1')
        clock = FakeClock(step=1800)
        result = search.run_search(self.spec(max_hours=2), self.state, self.trials, Driver(460, clock=clock), clock=clock,
                                   log=quiet)
        self.assertIn('max_hours (2) reached', result['stop_reason'])
        self.assertEqual(result['trials'], 5, 'trials start until the budget has passed')

    def test_deadline_pauses_before_a_trial_that_would_not_fit(self):
        clock = FakeClock(step=0)
        driver = Driver(460, clock=clock)
        result = search.run_search(self.spec(), self.state, self.trials, driver, clock=clock, log=quiet,
                                   deadline=clock.now + 1000)
        self.assertTrue(result['paused'])
        self.assertEqual(driver.calls, [], '30 s + 500 s + 900 s overhead does not fit in 1,000 s')
        self.assertFalse(json.loads(self.state.read_text())['done'])

    def test_sanity_gate(self):
        result = search.run_search(self.spec(expected=450), self.state, self.trials, Driver(460), log=quiet)
        self.assertFalse(result['sanity_gate']['hold'])
        self.assertEqual(result['sanity_gate']['binding'], 'DUT')
        self.assertTrue(search.sanity_gate({'result_sps': 100}, 450)['hold'])
        capped = search.sanity_gate({'result_sps': 500, 'tester_limited': True, 'parameters': {'r_max': 500}}, 450)
        self.assertTrue(capped['hold'])
        self.assertFalse(search.sanity_gate({'result_sps': 500, 'tester_limited': True}, 450, tester_cap_ok=True)['hold'])
        custom = search.run_search(self.spec(), self.tmp / 'x.json', self.tmp / 'x.jsonl', Driver(460), log=quiet,
                                   sanity=lambda r: {'hold': False, 'reason': 'custom %s' % r['result_sps']})
        self.assertEqual(custom['sanity_gate']['reason'], 'custom 458')

    def test_run_trial_must_return_a_trial_result(self):
        with self.assertRaises(TypeError):
            search.run_search(self.spec(), self.state, self.trials, lambda rate, index, profile: {}, log=quiet)


if __name__ == '__main__':
    unittest.main()
