#!/usr/bin/env python3

import unittest
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import analyze


class AnalyzeDiagnosticsTest(unittest.TestCase):
    def test_parse_nested_diagnostic_brackets(self):
        line = (
            "[sip_retrans_diag] dup_invite_cache_miss=0 ack_unmatched=0 "
            "worker_mismatch=0 invite_2xx_proactive_retx=4 bye_200_sent=10 "
            "bye_path=[udp_to_handler=[count=1 avg_us=10 p50_us=10 p95_us=10 "
            "p99_us=10 p999_us=10 max_us=10 over_500ms=0] "
            "send_response=[count=1 avg_us=20 p50_us=25 p95_us=25 p99_us=25 "
            "p999_us=25 max_us=22 over_500ms=0]] "
            "transaction_dispatch_queue_by_kind=[invite=[count=2 avg_us=30 "
            "p50_us=50 p95_us=50 p99_us=50 p999_us=50 max_us=31 over_500ms=0] "
            "bye=[count=3 avg_us=40 p50_us=50 p95_us=50 p99_us=50 p999_us=50 "
            "max_us=41 over_500ms=0]] "
            "transaction_dispatch_queue_by_worker=[w0=[count=5 avg_us=60 "
            "p50_us=100 p95_us=100 p99_us=100 p999_us=100 max_us=61 "
            "over_500ms=0 depth_max=7]]"
        )

        parsed = analyze.parse_sip_retrans_diag(line)

        self.assertEqual(parsed["dup_invite_cache_miss"], 0)
        self.assertIn("send_response=[count=1", parsed["bye_path"])
        self.assertIn("bye=[count=3", parsed["transaction_dispatch_queue_by_kind"])
        self.assertIn("depth_max=7", parsed["transaction_dispatch_queue_by_worker"])


if __name__ == "__main__":
    unittest.main()
