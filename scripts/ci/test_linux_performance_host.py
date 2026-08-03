import importlib.util
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "scripts/release/linux_performance_host.py"
SPEC = importlib.util.spec_from_file_location("linux_performance_host", MODULE_PATH)
assert SPEC and SPEC.loader
HOST = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(HOST)


class LinuxPerformanceHostTests(unittest.TestCase):
    def make_roots(self, directory: Path, *, rcvbuf: int = 0, sndbuf: int = 0):
        proc = directory / "proc"
        sys = directory / "sys"
        (proc / "net").mkdir(parents=True)
        loopback = sys / "class/net/lo/statistics"
        loopback.mkdir(parents=True)
        (proc / "net/snmp").write_text(
            "Ip: Forwarding DefaultTTL\n"
            "Ip: 2 64\n"
            "Udp: InDatagrams NoPorts InErrors OutDatagrams RcvbufErrors SndbufErrors InCsumErrors IgnoredMulti MemErrors\n"
            f"Udp: 100 4 {rcvbuf + sndbuf} 120 {rcvbuf} {sndbuf} 0 0 0\n",
            encoding="utf-8",
        )
        (proc / "net/sockstat").write_text(
            "sockets: used 20\nUDP: inuse 8 mem 21\n",
            encoding="utf-8",
        )
        (proc / "net/softnet_stat").write_text(
            "0000000a 00000002 00000003 00000000\n"
            "0000000b 00000004 00000005 00000000\n",
            encoding="utf-8",
        )
        (loopback / "rx_dropped").write_text("7\n", encoding="utf-8")
        (loopback / "tx_dropped").write_text("9\n", encoding="utf-8")
        return proc, sys

    def test_snapshot_uses_linux_native_counters(self):
        with tempfile.TemporaryDirectory() as raw:
            proc, sys = self.make_roots(Path(raw), rcvbuf=2, sndbuf=3)
            snapshot = HOST.capture_snapshot(proc, sys)

        self.assertEqual(snapshot["udp_datagrams_received"], 100)
        self.assertEqual(snapshot["udp_dropped_no_socket"], 4)
        self.assertEqual(snapshot["udp_dropped_full_socket_buffers"], 5)
        self.assertEqual(snapshot["udp_open_sockets"], 8)
        self.assertEqual(snapshot["udp_memory_pages"], 21)
        self.assertEqual(snapshot["softnet_dropped_total"], 6)
        self.assertEqual(snapshot["softnet_time_squeeze_total"], 8)
        self.assertEqual(snapshot["loopback_rx_dropped"], 7)
        self.assertEqual(snapshot["loopback_tx_dropped"], 9)

    def test_delta_preserves_existing_keys_and_passes_zero_drops(self):
        with tempfile.TemporaryDirectory() as raw:
            directory = Path(raw)
            before = {
                "schema": HOST.SNAPSHOT_SCHEMA,
                "platform": "linux",
                **{key: 10 for key in HOST.COUNTER_KEYS},
                **{key: 20 for key in HOST.GAUGE_KEYS},
            }
            after = dict(before)
            after["udp_datagrams_received"] = 110
            after["udp_datagram_output"] = 95
            after["udp_dropped_no_socket"] = 12
            after["udp_open_sockets"] = 18
            before_path = directory / "before.txt"
            after_path = directory / "after.txt"
            HOST.write_key_values(before_path, before)
            HOST.write_key_values(after_path, after)
            delta = HOST.calculate_delta(before_path, after_path, True)

        self.assertEqual(delta["udp_datagrams_received_delta"], 100)
        self.assertEqual(delta["udp_dropped_no_socket_delta"], 2)
        self.assertEqual(delta["udp_open_sockets_delta"], -2)
        self.assertEqual(delta["zero_drop_validation"], "PASS")

    def test_release_validation_fails_on_buffer_drop(self):
        with tempfile.TemporaryDirectory() as raw:
            directory = Path(raw)
            before = {
                "schema": HOST.SNAPSHOT_SCHEMA,
                "platform": "linux",
                **{key: 10 for key in HOST.COUNTER_KEYS},
                **{key: 20 for key in HOST.GAUGE_KEYS},
            }
            after = dict(before)
            after["udp_rcvbuf_errors"] = 11
            before_path = directory / "before.txt"
            after_path = directory / "after.txt"
            HOST.write_key_values(before_path, before)
            HOST.write_key_values(after_path, after)
            delta = HOST.calculate_delta(before_path, after_path, True)
            with self.assertRaisesRegex(HOST.EvidenceError, "udp_rcvbuf_errors=1"):
                HOST.validate_delta(delta)

    def test_missing_mandatory_snmp_counter_fails_closed(self):
        with tempfile.TemporaryDirectory() as raw:
            proc, sys = self.make_roots(Path(raw))
            (proc / "net/snmp").write_text(
                "Udp: InDatagrams NoPorts InErrors OutDatagrams RcvbufErrors\n"
                "Udp: 1 2 3 4 5\n",
                encoding="utf-8",
            )
            with self.assertRaisesRegex(HOST.EvidenceError, "SndbufErrors"):
                HOST.capture_snapshot(proc, sys)


if __name__ == "__main__":
    unittest.main()
