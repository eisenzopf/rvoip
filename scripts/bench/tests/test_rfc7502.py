"""RFC 7502 §4.10 search, trial plans, RFC 6076 metrics and trial scoring (ported from the E74 test-rfc7502.py)."""
import json
from pathlib import Path
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import rfc6076  # noqa: E402
import rfc7502  # noqa: E402
import summarize_trial  # noqa: E402


def run_search(search, limit):
    """Drive a search against a deterministic DUT that succeeds at r <= limit."""
    while not search.done:
        rate = search.next_rate()
        n = 50000
        search.record(rate, n, n if rate <= limit else n - 1)
    return search


def call(status=200, outcome='pass', model=None, srd=None, kind='successful', sdd=None, sdt=None, completed=True, first=None,
         complete=True):
    row = {'sip_status': status, 'outcome': outcome}
    if model:
        row['call_model'] = model
    if srd is not None or sdd is not None or sdt is not None or first is not None:
        row['rfc6076'] = {'final_status': status, 'first_response_status': first or status, 'srd_ms': srd,
                          'srd_kind': kind if srd is not None else None, 'sdd_ms': sdd, 'sdt_ms': sdt,
                          'established': status == 200, 'completed': completed and status == 200,
                          'trace_complete': complete, 'invite_transmissions': 1}
    return row


def rows_of(ends, start=0):
    return [{'id': str(i), 'scheduled_ms': start + i, 'send_ms': start + i, 'dispatched': True, 'scheduler_lateness_ms': 0,
             'end': end} for i, end in enumerate(ends)]


class SearchTests(unittest.TestCase):
    def test_rfc7502_appendix_a_converges_to_458(self):
        # RFC 7502 Appendix A: a DUT limited at 460 sps, find_R(100) -> 458.
        search = run_search(rfc7502.Search(100), 460)
        self.assertEqual(search.result, 458)
        self.assertIn('converged', search.s['stop_reason'])
        self.assertFalse(search.s['tester_limited'])

    def test_weights_halve_with_a_floor_and_rates_floor(self):
        search = rfc7502.Search(1000, w=0.4)
        self.assertEqual(search.s['d'], 0.2, 'd = max(0.10, w/2)')
        search.record(1000, 10, 10)
        self.assertEqual(search.next_rate(), 1400)
        search.record(1400, 10, 9)
        self.assertEqual((search.next_rate(), search.s['w'], search.s['d']), (1120, 0.2, 0.1))
        search.record(1120, 10, 9)
        self.assertEqual((search.next_rate(), search.s['w'], search.s['d']), (1008, 0.1, 0.1))

    def test_quantum_and_tester_cap(self):
        search = rfc7502.Search(1001, quantum=2, r_max=1200)
        self.assertEqual(search.next_rate(), 1000)
        run_search(search, 10 ** 9)
        self.assertEqual(search.result, 1200)
        self.assertTrue(search.s['tester_limited'])
        self.assertIn('R >= 1200', search.s['stop_reason'])
        self.assertTrue(all(t['rate'] % 2 == 0 for t in search.s['trials']))

    def test_quantum_4_gives_every_host_a_whole_share(self):
        search = rfc7502.new_search({'r0': 4003, 'baseline_sps': 9426})
        self.assertEqual(search.s['quantum'], 4)
        self.assertEqual(search.s['r_max'], rfc7502.tester_cap(9426))
        self.assertEqual(search.s['r_max'], 6312, '0.67 x 9,426 = 6,315.4, floored to 4')
        self.assertEqual(search.next_rate(), 4000)
        run_search(search, 10 ** 9)
        self.assertEqual(search.result, 6312)
        self.assertTrue(search.s['tester_limited'])
        self.assertTrue(all(t['rate'] % 4 == 0 for t in search.s['trials']))
        for t in search.s['trials']:
            for hosts in (1, 2, 4):
                rfc7502.validate_shares(rfc7502.trial_profile(t['rate'], hosts=hosts), hosts)
        bounded = run_search(rfc7502.new_search({'r0': 4000, 'r_max': 6312}), 5001)
        self.assertLessEqual(bounded.result, 5001)
        self.assertEqual(bounded.result % 4, 0)

    def test_r0_defaults_to_the_cap_and_is_kept_below_it(self):
        self.assertEqual(rfc7502.new_search({'baseline_sps': 1000}).next_rate(), 668)
        self.assertEqual(rfc7502.new_search({'r0': 5000, 'r_max': 1000}).next_rate(), 1000)
        with self.assertRaises(ValueError):
            rfc7502.new_search({})

    def test_invalid_trials_repeat_then_stop_at_three(self):
        search = rfc7502.Search(500)
        self.assertEqual(search.record(500, 0, 0), 'invalid')
        self.assertEqual(search.next_rate(), 500, 'an invalid trial repeats the same rate')
        self.assertEqual(search.record(500, 100, 100, valid=False), 'invalid')
        self.assertFalse(search.done, 'a second invalid trial is retried')
        self.assertEqual(search.record(500, 100, 100, valid=False), 'invalid')
        self.assertTrue(search.done)
        self.assertIsNone(search.result)
        search = rfc7502.Search(500)
        search.record(500, 0, 0)
        self.assertEqual(search.record(500, 100, 100), 'pass')
        self.assertEqual(search.s['invalid_streak'], 0)

    def test_state_round_trips_and_max_trials_stops(self):
        search = rfc7502.Search(100, max_trials=3)
        search.record(100, 5, 5)
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'state.json'
            search.save(path)
            again = rfc7502.Search.load(path)
        self.assertEqual(again.next_rate(), 110)
        again.record(110, 5, 5)
        again.record(121, 5, 5)
        self.assertTrue(again.done)
        self.assertIsNone(again.result)
        self.assertIn('max_trials', again.s['stop_reason'])
        again.stop('ignored once done')
        self.assertIn('max_trials', again.s['stop_reason'])
        with self.assertRaises(RuntimeError):
            again.record(121, 5, 5)

    def test_bisect_accelerator_brackets_and_confirms(self):
        search = rfc7502.Search(1000, algorithm='bisect', confirmations=3, resolution=0.02, quantum=4)
        run_search(search, 1333)
        self.assertLessEqual(search.result, 1333)
        self.assertGreaterEqual(search.result, 1333 * 0.97)
        self.assertLess(len(search.s['trials']), 20, 'bisect needs far fewer trials than §4.10')
        self.assertIn('confirmation', search.s['stop_reason'])


class ProfileTests(unittest.TestCase):
    def test_sustained_rule_is_the_default(self):
        profile = rfc7502.trial_profile(2000)
        self.assertEqual(profile['stages'], [{'name': 'warmup', 'cps': 1000, 'seconds': 30},
                                             {'name': 'trial-1', 'cps': 2000, 'seconds': 180}])
        self.assertEqual(profile['transient_stages'], ['warmup'])
        # N / r wins over 180 s at low rates
        self.assertEqual(rfc7502.trial_profile(200)['stages'][-1]['seconds'], 250)

    def test_burst_trials_keep_n_attempts_in_the_window(self):
        self.assertEqual(rfc7502.trial_profile(10152, 50000, minimum_seconds=0)['stages'][-1]['seconds'], 6)
        self.assertEqual(rfc7502.trial_profile(4000, 50000, minimum_seconds=0)['stages'][-1]['seconds'], 13)

    def test_long_trials_split_at_the_caller_stage_bound(self):
        long = rfc7502.trial_profile(50, 50000, minimum_seconds=0)
        self.assertEqual([s['seconds'] for s in long['stages'][1:]], [600, 400])
        self.assertEqual(rfc7502.trial_profile(400, 10000, 0, 120)['stages'], [{'name': 'trial-1', 'cps': 400, 'seconds': 120}])

    def test_ramped_warmup_for_admission_rows(self):
        profile = rfc7502.trial_profile(3600, 648000, warmup_seconds=120, warmup_ramp=True)
        self.assertEqual([(s['name'], s['cps'], s['seconds']) for s in profile['stages']],
                         [('warmup-1', 900, 30), ('warmup-2', 1800, 30), ('warmup-3', 2700, 30), ('warmup-4', 3600, 30),
                          ('trial-1', 3600, 180)])
        self.assertEqual(profile['transient_stages'], ['warmup-1', 'warmup-2', 'warmup-3', 'warmup-4'])
        self.assertIn('ramped', profile['basis'])
        odd = rfc7502.trial_profile(1066, 191880, warmup_seconds=120, warmup_ramp=True)
        self.assertEqual([s['cps'] for s in odd['stages'][:4]], [268, 536, 800, 1066])
        for r in (4356, 3960, 1068, 880, 5200, 636):
            for hosts in (1, 2, 4):
                for st in rfc7502.trial_profile(r, warmup_seconds=120, warmup_ramp=True, hosts=hosts)['stages']:
                    self.assertEqual((st['cps'] * st['seconds']) % hosts, 0, (r, st, hosts))
        with self.assertRaises(ValueError):
            rfc7502.trial_profile(3600, 648000, warmup_seconds=30, warmup_ramp=True)
        spec = {'r0': 3600, 'warmup_ramp': True}
        self.assertEqual(rfc7502.profile_for_spec(spec, 3600)['stages'][3]['name'], 'warmup-4')
        self.assertEqual(rfc7502.profile_for_spec(spec, 3600)['stages'][0]['seconds'], 30, '120 s ramp by default')

    def test_three_hosts_extend_a_stage_to_whole_shares(self):
        profile = rfc7502.trial_profile(1004, 50000, minimum_seconds=181, hosts=3)
        trial = profile['stages'][-1]
        self.assertEqual((trial['cps'] * trial['seconds']) % 3, 0)
        self.assertGreaterEqual(trial['seconds'], 181)
        shares = rfc7502.host_shares(trial['cps'], trial['seconds'], 3)
        self.assertEqual(sum(s['calls'] for s in shares), trial['cps'] * trial['seconds'])
        with self.assertRaises(ValueError):
            rfc7502.host_shares(1001, 1, 2)

    def test_scored_window_starts_after_the_ramp(self):
        profile = rfc7502.trial_profile(3600, 648000, warmup_seconds=120, warmup_ramp=True)
        summary = {'stage_summaries': [dict(s) for s in profile['stages']],
                   'caller_validity': {'caller_valid_before_backpressure': False,
                                       'checks': {'dispatch_within_1pct_per_bin_before_backpressure': False},
                                       'dispatch_misses': [{'t': 5, 'requested': 900, 'dispatched': 700},
                                                           {'t': 100, 'requested': 3600, 'dispatched': 3000}]}}
        self.assertEqual(rfc7502.trial_windows(summary), [(120, 300)])
        self.assertEqual(rfc7502.caller_invalid_in_trial(summary), [])


class MetricsTests(unittest.TestCase):
    def test_rfc6076_ratios_follow_the_rfc_formulas(self):
        calls = ([call(200, srd=5.0, sdd=2.0, sdt=10000.0)] * 90 + [call(486, 'fail', srd=4.0, kind='failed')] * 2
                 + [call(503, 'fail', srd=1.0, kind='failed')] * 3 + [call(302, 'fail')] * 4 + [call(None, 'fail')])
        m = rfc6076.metrics(calls)
        self.assertEqual(m['invites'], 100)
        self.assertEqual(m['invites_with_3xx'], 4)
        self.assertAlmostEqual(m['ser_pct'], 100 * 90 / 96)
        self.assertAlmostEqual(m['seer_pct'], 100 * 92 / 96, msg='486 counts toward SEER')
        self.assertEqual(m['isa_count'], 4, '503 x3 + one call without a final response (408 class)')
        self.assertAlmostEqual(m['isa_pct'], 4.0)
        self.assertAlmostEqual(m['scr_pct'], 90.0)
        self.assertEqual(m['srd_successful_ms']['count'], 90)
        self.assertEqual(m['srd_failed_ms']['count'], 5, 'failed SRD is never combined with successful SRD')
        self.assertEqual(m['sdd_ms']['p99_9'], 2.0)
        self.assertEqual(m['sdt_ms']['mean'], 10000.0)
        self.assertEqual(m['final_status_counts']['no_final_response'], 1)
        self.assertTrue(m['trace']['available'])

    def test_rfc6076_without_wire_trace_reports_unavailable_intervals(self):
        m = rfc6076.metrics([{'sip_status': 200, 'outcome': 'pass'}, {'sip_status': 503, 'outcome': 'fail'}])
        self.assertEqual((m['ser_pct'], m['isa_pct'], m['scr_pct']), (50.0, 50.0, 50.0))
        self.assertEqual(m['srd_successful_ms']['count'], 0)
        self.assertIsNone(m['srd_successful_ms']['p99'])
        self.assertFalse(m['trace']['available'])
        self.assertEqual(rfc6076.metrics([])['ser_pct'], None, 'a zero denominator is undefined (RFC 6076 §4)')

    def test_distribution_reports_p99_9_and_max(self):
        d = rfc6076.distribution(list(range(1, 10001)))
        self.assertEqual((d['p50'], d['p99'], d['p99_9'], d['max'], d['count']), (5000, 9900, 9990, 10000, 10000))
        self.assertEqual(summarize_trial.percentiles(list(range(1, 1001)))['p99_9'], 999)

    def test_complete_call_is_the_call_models_pass(self):
        hold = {'sip_status': 200, 'outcome': 'pass', 'call_model': 'caller-hold'}
        self.assertTrue(summarize_trial.complete_call(hold))
        self.assertFalse(summarize_trial.complete_call(dict(hold, outcome='fail')))
        self.assertTrue(rfc6076.rfc_session_ok(call(200, model='signaling-only', srd=3.0, completed=True)))
        self.assertFalse(rfc6076.rfc_session_ok(call(200, srd=3.0, completed=False)), 'a failed BYE is not a completed session')
        self.assertFalse(rfc6076.rfc_session_ok(call(503, 'fail')))

    def test_rfc_criterion_counts_attempt_failures_only_and_records_disconnect_failures(self):
        bye_481 = call(200, srd=4.0, completed=False)
        bye_481['rfc6076']['bye_final_status'] = 481
        self.assertTrue(rfc6076.rfc_attempt_ok(bye_481))
        self.assertTrue(rfc6076.disconnect_failure(bye_481))
        self.assertFalse(rfc6076.rfc_session_ok(bye_481))
        self.assertFalse(rfc6076.rfc_attempt_ok(call(503, 'fail', srd=1.0, kind='failed')))
        self.assertFalse(rfc6076.disconnect_failure(call(503, 'fail', srd=1.0, kind='failed')))
        ends = [dict(call(200, srd=4.0, sdd=1.0, sdt=0.0), setup_ms=4.0)] * 9 + [dict(bye_481, setup_ms=4.0)]
        aggregate = summarize_trial.aggregate(rows_of(ends), 1)
        self.assertEqual((aggregate['rfc_attempt_ok'], aggregate['rfc_session_ok'], aggregate['rfc_disconnect_failures']), (10, 9, 1))
        summary = {'stage_summaries': [{'name': 'trial-1', 'whole_stage': aggregate}],
                   'caller_validity': {'caller_valid_before_backpressure': True}}
        attempted, succeeded, valid, detail = rfc7502.evaluate_trial(summary, 'rfc')
        self.assertEqual((attempted, succeeded, valid, detail['disconnect_failures'], detail['scored_by']), (10, 10, True, 1, 'rfc_attempt_ok'))
        self.assertEqual(rfc7502.evaluate_trial(summary, 'full')[1], aggregate['full'], 'criterion full keeps its own rule')


class EvaluationTests(unittest.TestCase):
    def test_warmup_backpressure_leaves_no_window_for_the_write_check(self):
        summary = {'stage_summaries': [{'name': 'warmup', 'requested_cps': 1458, 'seconds': 30},
                                       {'name': 'trial-1', 'requested_cps': 2916, 'seconds': 180,
                                        'whole_stage': {'offered': 524510, 'dispatched': 524510, 'missing_terminal_observations': 0,
                                                        'rfc_attempt_ok': 524510, 'rejected_503': 0}}],
                   'caller_validity': {'caller_valid_before_backpressure': False, 'backpressure_second': 0,
                                       'scheduler_lateness_p99_ms': 1.96438,
                                       'checks': {'schedule_to_write_p99_at_most_5ms': False,
                                                  'dispatch_within_1pct_per_bin_before_backpressure': False}}}
        attempted, succeeded, valid, detail = rfc7502.evaluate_trial(summary)
        self.assertEqual((attempted, succeeded, valid, detail['invalid_reasons']), (524510, 524510, True, []))
        summary['caller_validity']['scheduler_lateness_p99_ms'] = 7.5
        self.assertFalse(rfc7502.evaluate_trial(summary)[2], 'a real late write path is still tester evidence')

    def test_dut_503s_with_a_slow_caller_write_path_fail_not_invalid(self):
        summary = {'stage_summaries': [{'name': 'warmup', 'requested_cps': 1800, 'seconds': 30},
                                       {'name': 'trial-1', 'requested_cps': 3600, 'seconds': 180,
                                        'whole_stage': {'offered': 647789, 'dispatched': 647789, 'missing_terminal_observations': 0,
                                                        'rfc_attempt_ok': 458058, 'rejected_503': 189731}}],
                   'caller_validity': {'caller_valid_before_backpressure': False,
                                       'checks': {'schedule_to_write_p99_at_most_5ms': False}}}
        attempted, succeeded, valid, detail = rfc7502.evaluate_trial(summary)
        self.assertEqual((attempted, succeeded, valid), (647789, 458058, True))
        self.assertEqual(detail['dut_shortfall_reasons'], ['caller check schedule_to_write_p99_at_most_5ms failed'])
        summary['stage_summaries'][1]['whole_stage'].update(rfc_attempt_ok=647789, rejected_503=0)
        self.assertFalse(rfc7502.evaluate_trial(summary)[2])

    def test_short_burst_trial_stage_is_judged_on_its_total(self):
        def run(misses):
            return {'stage_summaries': [{'name': 'warmup', 'requested_cps': 4248, 'seconds': 30},
                                        {'name': 'trial-1', 'requested_cps': 8496, 'seconds': 7}],
                    'caller_validity': {'caller_valid_before_backpressure': False,
                                        'checks': {'dispatch_within_1pct_per_bin_before_backpressure': False},
                                        'dispatch_misses': misses}}
        observed = [{'t': 0, 'requested': 4248, 'dispatched': 4329}, {'t': 29, 'requested': 4248, 'dispatched': 5021},
                    {'t': 30, 'requested': 8496, 'dispatched': 8390}, {'t': 32, 'requested': 8496, 'dispatched': 8609}]
        self.assertEqual(rfc7502.caller_invalid_in_trial(run(observed)), [])
        self.assertEqual(rfc7502.caller_invalid_in_trial(run(observed[:3])), [], 'first-bin under-dispatch only')
        short = [{'t': t, 'requested': 8496, 'dispatched': 8200} for t in range(30, 37)]
        self.assertEqual(rfc7502.caller_invalid_in_trial(run(short)), ['caller dispatch outside 1% in 7 trial bins'])
        long_run = run([{'t': 100, 'requested': 8496, 'dispatched': 8000}])
        long_run['stage_summaries'][1]['seconds'] = 180
        self.assertTrue(rfc7502.caller_invalid_in_trial(long_run))

    def test_a_short_last_trial_bin_is_an_edge_not_a_caller_miss(self):
        summary = {'stage_summaries': [{'name': 'warmup', 'seconds': 30, 'whole_stage': {}},
                                       {'name': 'trial-1', 'seconds': 19, 'whole_stage': {}}],
                   'caller_validity': {'caller_valid_before_backpressure': False,
                                       'checks': {'dispatch_within_1pct_per_bin_before_backpressure': False},
                                       'dispatch_misses': [{'t': 29, 'requested': 1320, 'dispatched': 1595},
                                                           {'t': 48, 'requested': 2640, 'dispatched': 2088}]}}
        self.assertEqual(rfc7502.caller_invalid_in_trial(summary), [])
        summary['caller_validity']['dispatch_misses'].append({'t': 30, 'requested': 2640, 'dispatched': 2805})
        self.assertEqual(rfc7502.caller_invalid_in_trial(summary), [], 'the stage-switch catch-up in the first trial second')
        summary['caller_validity']['dispatch_misses'].append({'t': 40, 'requested': 2640, 'dispatched': 2400})
        self.assertTrue(rfc7502.caller_invalid_in_trial(summary), 'a short bin inside the trial is still a miss')

    def test_neighbour_make_up_is_judged_per_caller_host(self):
        summary = {'stage_summaries': [{'name': 'warmup', 'seconds': 30}, {'name': 'trial-1', 'seconds': 180}],
                   'caller_validity': {'caller_valid_before_backpressure': False,
                                       'checks': {'dispatch_within_1pct_per_bin_before_backpressure': False},
                                       'dispatch_misses': [{'t': 40, 'requested': 880, 'dispatched': 850, 'caller': 'caller-1'},
                                                           {'t': 41, 'requested': 880, 'dispatched': 910, 'caller': 'caller-2'}]}}
        self.assertEqual(rfc7502.caller_invalid_in_trial(summary), ['caller dispatch outside 1% in 2 trial bins'],
                         'another host\'s surplus does not make up a host\'s shortfall')
        summary['caller_validity']['dispatch_misses'][1]['caller'] = 'caller-1'
        self.assertEqual(rfc7502.caller_invalid_in_trial(summary), [])

    def test_dut_driven_dispatch_shortfall_is_a_failed_trial(self):
        ends = [dict(call(200, srd=4.0, sdd=1.0, sdt=0.0), setup_ms=4.0)] * 6 + \
               [{'sip_status': None, 'outcome': 'fail', 'failure_class': 'timeout'}] * 2
        rows = rows_of(ends)
        rows += [{'id': str(i), 'scheduled_ms': i, 'send_ms': i, 'dispatched': False, 'scheduler_lateness_ms': 0,
                  'end': {'outcome': 'inconclusive', 'reason': 'generator_capacity'}} for i in range(8, 10)]
        aggregate = summarize_trial.aggregate(rows, 1)
        base = {'stage_summaries': [{'name': 'trial-1', 'whole_stage': aggregate}]}
        shortfall = dict(base, caller_validity={'caller_valid_before_backpressure': False,
                                                'checks': {'clean_shutdown': False, 'source_pools_balanced': False}})
        attempted, succeeded, valid, detail = rfc7502.evaluate_trial(shortfall, 'rfc')
        self.assertTrue(valid, detail['invalid_reasons'])
        self.assertLess(succeeded, attempted)
        self.assertTrue(detail['dut_shortfall_reasons'])
        hot = dict(base, caller_validity={'caller_valid_before_backpressure': False,
                                          'checks': {'no_thread_above_0_8_core': False, 'clean_shutdown': False}})
        self.assertFalse(rfc7502.evaluate_trial(hot, 'rfc')[2], 'a hot caller thread keeps the trial invalid')

    def test_a_dropped_trace_event_never_reads_as_a_failed_session(self):
        partial = call(200, srd=4.0, completed=False, complete=False)
        self.assertTrue(rfc6076.rfc_session_ok(partial), 'the caller passed the call, BYE included')
        self.assertFalse(rfc6076.rfc_session_ok(dict(partial, outcome='fail')))
        m = rfc6076.metrics([partial, call(200, srd=4.0, sdd=1.0, sdt=10.0)])
        self.assertEqual((m['scr_pct'], m['trace']['incomplete_records']), (100.0, 1))
        summary = {'stage_summaries': [{'name': 'trial-1', 'whole_stage': summarize_trial.aggregate(
            rows_of([dict(partial, setup_ms=4.0)]), 1)}]}
        attempted, succeeded, valid, detail = rfc7502.evaluate_trial(summary, 'rfc')
        self.assertEqual((attempted, succeeded, valid), (1, 1, True))
        self.assertTrue(detail['rfc6076_trace_coverage_low'])

    def test_missing_evidence_and_failed_checks(self):
        ends = [dict(call(200, model='signaling-only', srd=4.0, sdd=1.0, sdt=0.0), setup_ms=4.0)] * 99 + \
               [call(503, 'fail', srd=1.0, kind='failed')]
        aggregate = summarize_trial.aggregate(rows_of(ends, 1000), 1)
        self.assertEqual((aggregate['rfc_session_ok'], aggregate['full'], aggregate['rejected_503']), (99, 99, 1))
        summary = {'stage_summaries': [{'name': 'warmup', 'whole_stage': aggregate},
                                       {'name': 'trial-1', 'whole_stage': aggregate}],
                   'caller_validity': {'caller_valid_before_backpressure': True},
                   'checks': {'dut_srtp_unprotect_failures_zero': False, 'dut_true_zero': None}}
        attempted, succeeded, valid, detail = rfc7502.evaluate_trial(summary, 'rfc')
        self.assertEqual((attempted, succeeded, valid), (100, 99, True))
        self.assertEqual(detail['rejected_503'], 1)
        self.assertEqual(detail['run_failed_checks'], ['dut_srtp_unprotect_failures_zero'], 'recorded, never deciding')
        self.assertEqual(detail['run_missing_checks'], ['dut_true_zero'])
        self.assertEqual(rfc7502.trial_verdict(summary)['outcome'], 'fail')
        missing = json.loads(json.dumps(summary))
        missing['stage_summaries'][1]['whole_stage']['missing_terminal_observations'] = 2
        self.assertFalse(rfc7502.evaluate_trial(missing, 'rfc')[2], 'missing terminal evidence invalidates a trial')
        self.assertFalse(rfc7502.evaluate_trial({'stage_summaries': []})[2])
        self.assertEqual(rfc7502.trial_verdict(None)['outcome'], 'invalid')
        no_caller = dict(summary, checks={'caller_evidence_complete': False}, evidence={'callers_missing': ['caller-2']})
        attempted, succeeded, valid, detail = rfc7502.evaluate_trial(no_caller)
        self.assertFalse(valid)
        self.assertIn('caller evidence missing: caller-2', detail['invalid_reasons'])


if __name__ == '__main__':
    unittest.main()
