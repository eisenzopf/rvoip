"""Report rendering: complete evidence leaves no pending cell; missing evidence renders pending, never a number."""
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
import rfc_report  # noqa: E402
import rfc_report_ietf  # noqa: E402
import search  # noqa: E402
import summarize_trial  # noqa: E402

LIMIT = 60


def lane(root, rows=('R1',), media_rows=('R2',)):
    """Run real (synthetic-evidence) searches for each row under root and return the report spec."""
    spec_rows = []
    for row in rows:
        media = row in media_rows

        def run_trial(rate, index, profile, row=row, media=media):
            rel = '%s/t%02d-r%d' % (row, index, rate)
            trial_dir = root / rel
            fx.make_trial(trial_dir, rate, limit=LIMIT, media=media, hold_ms=1000 if media else 0, row=row,
                          run_start=fx.T0 + 1000 * index)
            path, summary = summarize_trial.write(trial_dir)
            return search.TrialResult(summary=summary, run=rel, trial_dir=trial_dir)
        spec = {'row': row, 'r0': 40, 'n': 200, 'warmup_seconds': 4, 'minimum_trial_seconds': 6, 'hosts': 2,
                'confirmations': 2, 'baseline_sps': 200}
        search.run_search(spec, root / row / 'search.json', root / row / 'trials.jsonl', run_trial, log=lambda *a: None)
        spec_rows.append({'row': row, 'search': '%s/search.json' % row, 'trials': '%s/trials.jsonl' % row})
    return {'title': 'rvoip-bench-uas test report', 'report_date': '2026-11-16', 'profile': 'minor',
            'evidence_root': str(root), 'evidence_uri': 's3://example-bucket/bench/lane-1/',
            'dut': {'name': 'rvoip-bench-uas', 'version': '0.4.1', 'git_sha': 'abcdef1234567890', 'instance_type': 'c8i.4xlarge',
                    'vcpu': 16, 'physical_cores': 8},
            'testbed': {'caller_hosts': 2, 'caller_type': 'c8in.24xlarge', 'sources_per_host': 2, 'connections_per_source': 8,
                        'caller_build': 'loadgen 1234abcd'},
            'environment': {'env_id': 'c8i.4xlarge-test', 'region': 'us-west-2'},
            'rows': spec_rows,
            'baseline': {row: {'result_sps': 200, 'source': 'perf-baselines/rfc7502/test/rates.json'} for row in rows},
            'summary': ['test summary'], 'findings': ['test finding']}


class ReportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = Path(tempfile.mkdtemp())
        cls.spec = lane(cls.tmp, rows=('R1', 'R2'))

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.tmp)

    def test_search_converged_on_the_synthetic_dut(self):
        state = json.loads((self.tmp / 'R1' / 'search.json').read_text())
        self.assertTrue(state['done'], state['stop_reason'])
        self.assertLessEqual(state['result'], LIMIT)
        self.assertEqual(state['result'] % 4, 0)
        self.assertTrue(any(t['outcome'] == 'fail' for t in state['trials']))

    def test_complete_evidence_has_no_pending_cell(self):
        for module, stem in ((rfc_report, 'rfc7502-report'), (rfc_report_ietf, 'rfc7502-ietf-report')):
            md, page = module.render(self.spec, self.tmp / 'report', stem)
            text = md.read_text()
            self.assertNotIn(rfc_report.PENDING, text, '%s:\n%s' % (stem, '\n'.join(
                line for line in text.splitlines() if rfc_report.PENDING in line)))
            self.assertIn('rvoip-bench-uas (rvoip abcdef123456)', text)
            html = page.read_text()
            self.assertIn('prefers-color-scheme: dark', html)
            self.assertNotIn('class="pending"', html)

    def test_report_values_come_from_the_evidence(self):
        doc, _ = rfc_report.build(self.spec)
        md = doc.markdown()
        state = json.loads((self.tmp / 'R1' / 'search.json').read_text())
        self.assertIn('%s sps | DUT' % rfc_report.fmt(state['result']), md)
        self.assertIn('"503":', md, 'the failure breakdown lists the DUT 503s')
        for section in ('1. Headline', '2. Testbed baseline (RFC 7502 §6.1)', '3. Test setup report (RFC 7502 §5.1)',
                        '4. Device benchmarks', '5. RFC 6076', '6. Setup latency below the limit',
                        '7. Session Attempt Failure breakdown', '8. Search traces', '9. Findings',
                        '10. Setup and methodology', '11. Deviations', '12. Evidence'):
            self.assertIn('## ' + section, md)
        self.assertNotIn('Overload and recovery', md, 'R4 renders only when the spec names its trial')
        for area in ('Topology', 'Warm-up', 'Fresh process per trial', 'Trial length', 'Rate quantum', 'Tester cap',
                     'Invalid trials', 'Establishment threshold', '503 with Retry-After', 'Percentiles', 'Algorithm variant'):
            self.assertIn('| %s |' % area, md)
        self.assertIn('100.0000%', md, 'SER of the final passing trial')

    def test_tester_limited_reading(self):
        spec = json.loads(json.dumps(self.spec))
        spec['baseline']['R1'] = {'result_sps': 60, 'source': 'low baseline'}
        md = rfc_report.build(spec)[0].markdown()
        self.assertIn('tester-limited (R > 0.67 x baseline 60)', md)

    def test_missing_evidence_renders_pending(self):
        spec = dict(self.spec, rows=[{'row': 'R5', 'search': 'R5/search.json'}], baseline={})
        md = rfc_report.build(spec)[0].markdown()
        self.assertIn('| R5 | UDP | none | 0 s | pending | pending | pending | pending | pending |', md)
        doc, title = rfc_report.build(rfc_report.TEMPLATE_SPEC, template=True)
        self.assertIn('TEMPLATE', doc.markdown())
        self.assertIn('class="pending"', doc.html(title))

    def test_summary_without_trial_runs_is_pending_not_zero(self):
        # a trial whose merged summary is gone: its load cells are pending, never 0
        spec = json.loads(json.dumps(self.spec))
        state = json.loads((self.tmp / 'R1' / 'search.json').read_text())
        missing_root = self.tmp / 'copy'
        (missing_root / 'R1').mkdir(parents=True)
        (missing_root / 'R1' / 'search.json').write_text(json.dumps(state))
        spec['evidence_root'] = str(missing_root)
        spec['rows'] = [{'row': 'R1', 'search': 'R1/search.json'}]
        md = rfc_report.build(spec)[0].markdown()
        self.assertIn(rfc_report.PENDING, md)
        self.assertIn('%s sps | DUT' % rfc_report.fmt(state['result']), md, 'the search state alone still gives R')


if __name__ == '__main__':
    unittest.main()
