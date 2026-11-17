"""host_sampler, disk_guard and evidence helpers."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import caller_policy  # noqa: E402
import disk_guard  # noqa: E402
import evidence  # noqa: E402
import host_sampler  # noqa: E402


def quiet(fn):
    """Run a CLI entry point with its stdout captured."""
    import contextlib
    import io
    with contextlib.redirect_stdout(io.StringIO()):
        return fn()


class HostSamplerTests(unittest.TestCase):
    def test_parses_proc_stat_with_spaces_in_the_name(self):
        line = '4242 (loadgen (x) y) S 1 4242 4242 0 -1 4194560 100 0 0 0 250 50 0 0 20 0 7 0 12345 1000000 512 18446744073709551615'
        row = host_sampler.parse_proc_stat(line)
        self.assertEqual((row['state'], row['utime'], row['stime'], row['threads'], row['starttime'], row['rss_pages']),
                         ('S', 250, 50, 7, 12345, 512))

    def test_busy_percentages(self):
        self.assertAlmostEqual(host_sampler.busy_pct_ticks([0] * 8, [30, 0, 10, 50, 10, 0, 0, 0]), 40.0)
        self.assertIsNone(host_sampler.busy_pct_ticks([0] * 8, [0] * 8))
        self.assertAlmostEqual(host_sampler.busy_pct_idle({'0': 0, '1': 0}, {'0': 1_500_000, '1': 1_500_000}, 2.0, 2), 25.0)
        self.assertIsNone(host_sampler.busy_pct_idle({'0': 0}, {'1': 5}, 2.0, 2))

    @unittest.skipUnless(sys.platform.startswith('linux'), 'reads /proc')
    def test_samples_a_live_process_until_it_exits(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / 'resources.jsonl'
            child = subprocess.Popen([sys.executable, '-c', 'import time\nend = time.time() + 1.6\nwhile time.time() < end: pass'])
            try:
                written = host_sampler.run(child.pid, out, interval=0.4, max_samples=50)
            finally:
                child.wait()
            rows = [json.loads(line) for line in out.read_text().splitlines()]
        self.assertEqual(written, len(rows))
        self.assertGreaterEqual(len(rows), 2)
        self.assertNotIn('cpu_cores_pct', rows[0])
        self.assertGreater(max(r.get('cpu_cores_pct', 0) for r in rows), 30.0, 'a busy loop is most of a core')
        self.assertEqual(rows[0]['logical_cpus'], os.cpu_count())
        self.assertTrue(rows[1]['thread_cpu_times'] and rows[1]['thread_names'])
        self.assertIsNotNone(rows[1]['host_cpu_pct'])
        peak, hottest = caller_policy.thread_peaks(rows)
        self.assertGreater(peak, 0.3)
        self.assertEqual(hottest['tid'], str(child.pid), 'the main thread is the busy one')
        self.assertIn('irq_cpus', rows[0], 'every sample carries the NIC IRQ CPU list (None when unknown)')
        softirq = caller_policy.softirq_saturation(rows)
        self.assertIsNotNone(softirq['peak_share'], 'per-CPU softirq comes from the per-CPU /proc/stat lines')
        self.assertEqual(softirq['missing'], [])

    def test_exits_at_once_for_a_missing_pid(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(host_sampler.run(2 ** 22 + 12345, Path(tmp) / 'r.jsonl', interval=0.01), 0)


class NicIrqTests(unittest.TestCase):
    def test_nic_irq_cpus_follow_the_queue_irq_affinity(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            (tmp / 'interrupts').write_text(
                '            CPU0       CPU1\n'
                ' 110:          1          0  PCI-MSIX-0000:27:00.0    0-edge      ena-mgmnt@pci:0000:27:00.0\n'
                ' 111:        900          0  PCI-MSIX-0000:27:00.0    1-edge      ens5-Tx-Rx-0\n'
                ' 112:          0        800  PCI-MSIX-0000:27:00.0    2-edge      ens5-Tx-Rx-1\n'
                ' 113:          0        800  PCI-MSIX-0000:28:00.0    1-edge      ens6-Tx-Rx-0\n'
                'NMI:          0          0   Non-maskable interrupts\n')
            for irq, cpus in (('111', '48'), ('112', '49-50'), ('113', '7')):
                (tmp / 'irq' / irq).mkdir(parents=True)
                (tmp / 'irq' / irq / 'effective_affinity_list').write_text(cpus + '\n')
            self.assertEqual(host_sampler.nic_irq_cpus('ens5', tmp / 'interrupts', tmp / 'irq'), [48, 49, 50])
            self.assertIsNone(host_sampler.nic_irq_cpus('eth9', tmp / 'interrupts', tmp / 'irq'))
            self.assertIsNone(host_sampler.nic_irq_cpus(None))
        self.assertEqual(host_sampler.parse_cpu_list('0-2,7, 9-9'), [0, 1, 2, 7, 9])


class DiskGuardTests(unittest.TestCase):
    def test_require_free(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertGreater(disk_guard.require_free(tmp, 0), 0)
            with self.assertRaises(disk_guard.DiskGuardError):
                disk_guard.require_free(tmp, 10 ** 9)
            self.assertEqual(quiet(lambda: disk_guard.main([tmp, '--min-gib', '0'])), 0)
            self.assertEqual(quiet(lambda: disk_guard.main([tmp, '--min-gib', '1e9'])), 1)

    def test_prune_keeps_small_files_symlinks_and_kept_names(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'caller-1'
            root.mkdir()
            (root / 'events.jsonl').write_bytes(b'x' * 4096)
            (root / 'report.json').write_bytes(b'x' * 10)
            (root / 'keep.bin').write_bytes(b'x' * 4096)
            (root / 'link').symlink_to(root / 'events.jsonl')
            trimmed = disk_guard.prune_unpacked([root, Path(tmp) / 'absent'], min_bytes=1024, keep={'keep.bin'})
            self.assertEqual(trimmed, 4096)
            self.assertEqual(sorted(p.name for p in root.iterdir()), ['keep.bin', 'link', 'report.json'])


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())

    def tearDown(self):
        shutil.rmtree(self.tmp)

    def test_layout_names(self):
        self.assertEqual(evidence.trial_dir_name(3, 4000), 't03-r4000')
        self.assertEqual(evidence.trial_path('R1', 12, 5384), 'R1/t12-r5384')
        self.assertEqual(evidence.parse_trial_dir_name('R1/t12-r5384'), (12, 5384))
        self.assertIsNone(evidence.parse_trial_dir_name('baseline'))

    def test_sha256_sidecar_and_manifest(self):
        f = self.tmp / 'a.json'
        f.write_text('{}')
        digest = evidence.write_sha256(f)
        self.assertEqual(Path(str(f) + '.sha256').read_text(), '%s  a.json\n' % digest)
        self.assertTrue(evidence.verify_sha256(f))
        f.write_text('{"x": 1}')
        self.assertFalse(evidence.verify_sha256(f))
        self.assertEqual(quiet(lambda: evidence.main(['verify', str(f)])), 1)
        rows = evidence.manifest(self.tmp)
        self.assertEqual(sorted(rows), ['a.json', 'a.json.sha256'])
        self.assertEqual(evidence.verify_manifest(self.tmp, rows), [])
        f.write_text('changed')
        self.assertEqual(evidence.verify_manifest(self.tmp, rows), ['a.json'])

    def test_pack_is_deterministic_and_unpacks(self):
        src = self.tmp / 'dut'
        (src / 'host').mkdir(parents=True)
        (src / 'summary.json').write_text('{"true_zero": true}')
        (src / 'host' / 'snmp.txt').write_text('Tcp: 1')
        first = evidence.pack(src, self.tmp / 'dut.tar.gz')
        os.utime(src / 'summary.json', (1, 1))
        second = evidence.pack(src, self.tmp / 'dut2.tar.gz')
        self.assertEqual(first, evidence.read_sha256(str(self.tmp / 'dut.tar.gz') + '.sha256'))
        out = evidence.unpack(self.tmp / 'dut.tar.gz', self.tmp / 'x')
        self.assertEqual(json.loads((out / 'dut' / 'summary.json').read_text()), {'true_zero': True})
        self.assertEqual((out / 'dut' / 'host' / 'snmp.txt').read_text(), 'Tcp: 1')
        self.assertNotEqual(first, second, 'file mtimes are part of the archive')
        again = evidence.pack(src, self.tmp / 'dut3.tar.gz')
        self.assertEqual(second, again, 'the same tree packs to the same bytes')

    def test_unpack_refuses_bad_archives(self):
        bad = self.tmp / 'bad.tar.gz'
        with tarfile.open(bad, 'w:gz') as tar:
            info = tarfile.TarInfo('../escape.txt')
            info.size = 1
            import io
            tar.addfile(info, io.BytesIO(b'x'))
        with self.assertRaises(ValueError):
            evidence.unpack(bad, self.tmp / 'out')
        good = self.tmp / 'good'
        good.mkdir()
        (good / 'f').write_text('1')
        evidence.pack(good, self.tmp / 'good.tar.gz')
        (self.tmp / 'good.tar.gz.sha256').write_text('%s  good.tar.gz\n' % ('0' * 64))
        with self.assertRaises(ValueError):
            evidence.unpack(self.tmp / 'good.tar.gz', self.tmp / 'out2')
        (self.tmp / 'good.tar.gz.sha256').unlink()
        with self.assertRaises(ValueError):
            evidence.unpack(self.tmp / 'good.tar.gz', self.tmp / 'out3', require_sidecar=True)


if __name__ == '__main__':
    unittest.main()
