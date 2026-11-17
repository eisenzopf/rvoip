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

    def test_start_skew_never_moves_a_call_out_of_its_stage(self):
        # Two caller hosts 0.5 s apart: each call stays in the stage its own host scheduled it in, so the
        # scored attempts are exactly the planned scored calls; the 1 s bins stay on one wall clock.
        d, profile = self.trial(40, skew=0.5)
        s = summarize_trial.summarize(d, 'rfc')
        self.assertEqual(s['caller_start_skew_seconds'], 0.5)
        self.assertIs(s['checks']['caller_start_skew'], True)
        for stage, row in zip(profile['stages'], s['stage_summaries']):
            planned = stage['cps'] * stage['seconds']
            self.assertEqual((row['planned_offers'], row['scheduled'], row['whole_stage']['offered']), (planned,) * 3)
        planned = sum(st['cps'] * st['seconds'] for st in profile['stages'] if st['name'].startswith('trial-'))
        attempted, succeeded, valid, _ = rfc7502.evaluate_trial(s)
        self.assertEqual((attempted, succeeded, valid), (planned, planned, True))
        self.assertEqual(s['rfc6076']['scored']['invites'], planned)
        self.assertEqual(s['stage_assignment']['own_clock'], 0)
        self.assertEqual(s['evaluation']['outcome'], 'pass')
        self.assertTrue(s['caller_validity']['caller_valid_before_backpressure'])
        self.assertEqual(s['caller_validity']['dispatch_misses'], [])
        total = sum(st['cps'] * st['seconds'] for st in profile['stages'])
        host2_tail = profile['stages'][-1]['cps'] // 2 // 2   # host 2's last 0.5 s runs past host 1's timeline
        self.assertEqual(sum(row['offered'] for row in s['bins_1s']), total - host2_tail)

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


CALLER_THREADS = ['loadgen-signaling', 'loadgen-call-0', 'loadgen-call-1', 'rvoip-media']
NET_BEFORE = """### at 1000.0
### iface ens5
### ethtool -S ens5
NIC statistics:
     rx_drops: 5
     pps_allowance_exceeded: 10
     bw_in_allowance_exceeded: 0
     bw_out_allowance_exceeded: 0
     conntrack_allowance_exceeded: 0
     linklocal_allowance_exceeded: 0
     conntrack_allowance_available: 900000
     queue_0_rx_cnt: 100
### sysfs /sys/class/net/ens5/statistics
rx_dropped: 7
tx_dropped: 0
### softnet /proc/net/softnet_stat
0000a000 00000000 00000001 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
0000b000 00000000 00000002 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000001
### snmp /proc/net/snmp
Udp: InDatagrams NoPorts InErrors OutDatagrams RcvbufErrors SndbufErrors InCsumErrors IgnoredMulti MemErrors
Udp: 1000 0 3 900 2 0 0 0 0
### snmp /proc/net/netstat
TcpExt: SyncookiesSent ListenDrops TCPTimeouts
TcpExt: 0 0 4
### conntrack
12
"""
NET_AFTER = (NET_BEFORE.replace('### at 1000.0', '### at 1250.0').replace('pps_allowance_exceeded: 10', 'pps_allowance_exceeded: 75')
             .replace('0000b000 00000000 00000002', '0000c000 00000004 00000012')
             .replace('Udp: 1000 0 3 900 2', 'Udp: 9000 0 53 900 52').replace('\n12\n', '\n40\n'))


class SoftirqSaturationTests(unittest.TestCase):
    """A caller host whose NIC IRQ CPU sits in softirq is tester evidence; the DUT side is recorded only."""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())

    def tearDown(self):
        shutil.rmtree(self.tmp)

    def trial(self, saturate=None, **kw):
        d = self.tmp / ('t-%d' % len(list(self.tmp.iterdir())))
        profile = fx.make_trial(d, 40, **kw)
        total = sum(st['seconds'] for st in profile['stages'])
        if saturate:
            # caller-1 only, sampled 0.5 s off the second: interval i covers [T0 - 3.5 + i, T0 - 2.5 + i]
            fx.write_resources(d / 'caller-1' / 'resources.jsonl', fx.T0 - 2.5, total + 4, 3.0, 0.3, CALLER_THREADS,
                               logical=96, physical=48, saturate=saturate)
            (d / 'caller-1' / 'tuning.json').write_text(json.dumps({'interface': 'ens5', 'plan': {'irq_cpus': [1, 2]}}))
        return d, profile

    def test_saturated_caller_makes_the_trial_invalid(self):
        d, profile = self.trial(saturate=(1, 8, 12))
        s = summarize_trial.summarize(d, 'rfc')
        by = s['caller_validity_by_caller']
        self.assertIs(by['caller-1']['checks']['no_irq_cpu_softirq_saturated'], False)
        self.assertIs(by['caller-2']['checks']['no_irq_cpu_softirq_saturated'], True)
        one = s['resource']['caller-1']
        self.assertEqual(one['softirq_saturated_cpus'], [1])
        self.assertAlmostEqual(one['softirq_peak_share'], 0.98)
        self.assertEqual(one['softirq']['irq_cpus'], [1, 2])
        self.assertTrue(one['softirq']['irq_cpus_source'].startswith('tuning.json plan.irq_cpus'))
        self.assertEqual(one['softirq']['first_saturated_second'], 4)
        self.assertAlmostEqual(s['resource']['caller-2']['softirq_peak_share'], 0.02)
        self.assertEqual(s['resource']['caller-2']['softirq']['cpus_judged'], 'all_cpus', 'no tuning.json')
        self.assertAlmostEqual(s['resource']['dut']['softirq_peak_share'], 0.02)
        self.assertEqual(s['resource']['dut']['softirq_saturated_cpus'], [])
        self.assertEqual(s['evaluation']['outcome'], 'invalid')
        reason = [r for r in s['evaluation']['reasons'] if 'no_irq_cpu_softirq_saturated' in r]
        self.assertEqual(reason, ['caller check no_irq_cpu_softirq_saturated failed '
                                  '(caller-1 receive path (softirq 98 % on CPUs 1, from second 4))'])
        side = s['evaluation']['detail']['host_saturation']
        self.assertEqual(side['side'], 'caller')

    def test_tester_saturation_is_never_a_dut_shortfall(self):
        # Timeouts in the trial stage are the DUT backpressure marker (second 4). Without saturation they fail the
        # trial; with caller-1 saturated from 3.5 s (before the marker) the same failures are tester evidence.
        clean, _ = self.trial(limit=30, caller_kwargs={'reject_status': None})
        s = summarize_trial.summarize(clean, 'rfc')
        self.assertEqual(s['backpressure']['first_timeout_second'], 4)
        self.assertEqual(s['evaluation']['outcome'], 'fail')
        self.assertEqual(s['evaluation']['detail']['host_saturation']['side'], None)
        d, _ = self.trial(saturate=(1, 7, 11), limit=30, caller_kwargs={'reject_status': None})
        s = summarize_trial.summarize(d, 'rfc')
        self.assertIs(s['caller_validity']['checks']['no_irq_cpu_softirq_saturated'], False)
        self.assertEqual(s['evaluation']['outcome'], 'invalid')
        # a dispatch shortfall beside it does not turn the trial into a DUT fail (the E74 exception)
        s['stage_summaries'][1]['whole_stage']['dispatched'] -= 3
        attempted, succeeded, valid, detail = rfc7502.evaluate_trial(s)
        self.assertFalse(valid)
        self.assertIsNone(detail['dut_shortfall_reasons'])
        self.assertIn('3 scheduled calls not dispatched', detail['invalid_reasons'])
        self.assertTrue(any('no_irq_cpu_softirq_saturated' in r for r in detail['invalid_reasons']))

    def test_saturation_after_the_backpressure_marker_is_recorded_not_failed(self):
        d, _ = self.trial(saturate=(1, 9, 12), limit=30, caller_kwargs={'reject_status': None})
        s = summarize_trial.summarize(d, 'rfc')
        self.assertIs(s['caller_validity']['checks']['no_irq_cpu_softirq_saturated'], True)
        self.assertEqual(s['resource']['caller-1']['softirq_saturated_cpus'], [1])
        self.assertEqual(s['evaluation']['outcome'], 'fail')
        self.assertIn('after the DUT backpressure marker', s['evaluation']['detail']['host_saturation']['text'])

    def test_saturated_dut_is_recorded_only(self):
        d, _ = self.trial(dut_kwargs={'saturate': (3, 8, 11)})
        s = summarize_trial.summarize(d, 'rfc')
        self.assertEqual(s['resource']['dut']['softirq_saturated_cpus'], [3])
        self.assertNotIn('no_irq_cpu_softirq_saturated', s['checks'])
        self.assertEqual(s['evaluation']['outcome'], 'pass')
        side = rfc7502.host_saturation(s)
        self.assertEqual(side['side'], 'dut')
        self.assertTrue(side['text'].startswith('DUT host receive path (softirq 98 % on CPUs 3'), side['text'])

    def test_net_counter_deltas(self):
        d, _ = self.trial(caller_kwargs={'net': (NET_BEFORE, NET_AFTER)})
        s = summarize_trial.summarize(d, 'rfc')
        net = s['resource']['caller-1']['net']
        self.assertEqual(net['iface'], 'ens5')
        self.assertEqual(net['seconds'], 250.0)
        self.assertEqual(net['flagged'], {'pps_allowance_exceeded': 65, 'softnet_dropped': 4, 'softnet_time_squeeze': 16,
                                          'UdpInErrors': 50, 'UdpRcvbufErrors': 50})
        self.assertEqual(net['deltas']['TcpExtTCPTimeouts'], 0)
        self.assertEqual(net['deltas']['sysfs_rx_dropped'], 0)
        self.assertEqual((net['conntrack_count_before'], net['conntrack_count_after']), (12, 40))
        self.assertEqual(net['softnet_processed'], 0x1000)
        self.assertNotIn('net', s['resource']['dut'] or {}, 'no snapshots on the DUT')
        flags = rfc7502.host_saturation(s)['net_flags']
        self.assertIn('caller-1 pps_allowance_exceeded +65', flags)
        self.assertIn('caller-2 UdpRcvbufErrors +50', flags)


if __name__ == '__main__':
    unittest.main()
