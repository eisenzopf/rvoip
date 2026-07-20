#!/usr/bin/env python3

import csv
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT_DIR = Path(__file__).resolve().parent
RUNNER = SCRIPT_DIR / "run_comparison.sh"


FAKE_SIPP = r"""#!/usr/bin/env python3
import csv
import json
import os
from pathlib import Path
import sys


args = sys.argv[1:]
if "-v" in args:
    print("SIPp fake-1.0")
    raise SystemExit(0)


def option(name: str) -> str:
    return args[args.index(name) + 1]


stat = Path(option("-stf"))
screen = Path(option("-screen_file"))
error = Path(option("-error_file"))
calls = int(option("-m"))
rate = int(option("-r"))
mode = os.environ.get("FAKE_SIPP_MODE", "success")
affected_suffix = os.environ.get("FAKE_SIPP_AFFECT_SUFFIX", "")
affected = not affected_suffix or stat.name.endswith(affected_suffix)

total = calls
success = calls
failed = 0
current = 0
if affected and mode == "incomplete":
    total -= 1
    success -= 1
elif affected and mode == "current":
    success -= 1
    current = 1
elif affected and mode == "failed":
    success -= 1
    failed = 1

stat.parent.mkdir(parents=True, exist_ok=True)
with stat.open("w", newline="") as handle:
    writer = csv.writer(handle, delimiter=";")
    writer.writerow(
        [
            "TargetRate",
            "ElapsedTime(C)",
            "TotalCallCreated",
            "SuccessfulCall(C)",
            "FailedCall(C)",
            "CurrentCall",
            "Retransmissions(C)",
        ]
    )
    writer.writerow([rate, "00:00:01", total, success, failed, current, 0])
screen.touch()
error.touch()

log_dir = Path(os.environ["FAKE_SIPP_LOG_DIR"])
log_dir.mkdir(parents=True, exist_ok=True)
(log_dir / f"{stat.name}.json").write_text(json.dumps(args))

exit_code = int(os.environ.get("FAKE_SIPP_EXIT_CODE", "0")) if affected else 0
raise SystemExit(exit_code)
"""


class RunComparisonTest(unittest.TestCase):
    def setUp(self):
        self.tempdir = tempfile.TemporaryDirectory()
        self.root = Path(self.tempdir.name)
        self.fake_sipp = self.root / "sipp"
        self.fake_sipp.write_text(FAKE_SIPP)
        self.fake_sipp.chmod(0o755)
        self.run_number = 0

    def tearDown(self):
        self.tempdir.cleanup()

    def run_harness(
        self,
        *,
        cps: int,
        shard_cps: int | None = None,
        mode: str = "success",
        affected_suffix: str = "",
        exit_code: int = 0,
    ) -> tuple[subprocess.CompletedProcess[str], Path, list[list[str]]]:
        self.run_number += 1
        run_root = self.root / f"run-{self.run_number}"
        results = run_root / "results"
        logs = run_root / "fake-logs"
        env = os.environ.copy()
        env.update(
            {
                "SIPP_BIN": str(self.fake_sipp),
                "FAKE_SIPP_LOG_DIR": str(logs),
                "FAKE_SIPP_MODE": mode,
                "FAKE_SIPP_AFFECT_SUFFIX": affected_suffix,
                "FAKE_SIPP_EXIT_CODE": str(exit_code),
                "RVOIP_PERF_RESULTS": str(results),
                "RVOIP_PERF_CPS": str(cps),
                "RVOIP_PERF_STEADY_SECS": "2",
                "RVOIP_PERF_INTER_RUN_PAUSE_SECS": "0",
            }
        )
        if shard_cps is not None:
            env["RVOIP_PERF_SIPP_SHARD_CPS"] = str(shard_cps)
        else:
            env.pop("RVOIP_PERF_SIPP_SHARD_CPS", None)

        completed = subprocess.run(
            [str(RUNNER), "127.0.0.1", "35060", "rvoip"],
            cwd=SCRIPT_DIR,
            env=env,
            text=True,
            capture_output=True,
            timeout=20,
            check=False,
        )
        invocations = [
            json.loads(path.read_text()) for path in sorted(logs.glob("*.json"))
        ]
        return completed, results, invocations

    @staticmethod
    def option(args: list[str], name: str) -> str:
        return args[args.index(name) + 1]

    @staticmethod
    def matrix_row(results: Path) -> list[str]:
        with (results / "runs.tsv").open(newline="") as handle:
            rows = list(csv.reader(handle, delimiter="\t"))
        if len(rows) != 2:
            raise AssertionError(f"expected header plus one aggregate row, got {rows!r}")
        return rows[1]

    def test_default_preserves_single_runner_and_hardens_its_limits(self):
        completed, results, invocations = self.run_harness(cps=2002)

        self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)
        self.assertEqual(len(invocations), 1)
        args = invocations[0]
        self.assertEqual(self.option(args, "-r"), "2002")
        self.assertEqual(self.option(args, "-m"), "4004")
        self.assertGreater(int(self.option(args, "-l")), 4004)
        self.assertEqual(self.option(args, "-timer_resol"), "1")
        self.assertEqual(self.option(args, "-max_recv_loops"), "10000")
        self.assertEqual(self.option(args, "-max_sched_loops"), "10000")
        self.assertEqual(Path(self.option(args, "-stf")).name, "rvoip_2002cps")

        row = self.matrix_row(results)
        self.assertEqual(row[0], "PASS")
        self.assertEqual(row[4], "2002")
        self.assertEqual(row[5], "4004")
        self.assertEqual(row[7], "0")
        self.assertNotIn(",", row[8])

    def test_optional_sharding_distributes_remainders_and_aggregates(self):
        completed, results, invocations = self.run_harness(cps=2002, shard_cps=1000)

        self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)
        self.assertEqual(len(invocations), 3)
        rates = [int(self.option(args, "-r")) for args in invocations]
        calls = [int(self.option(args, "-m")) for args in invocations]
        ports = [int(self.option(args, "-p")) for args in invocations]
        limits = [int(self.option(args, "-l")) for args in invocations]
        names = [Path(self.option(args, "-stf")).name for args in invocations]
        self.assertEqual(sorted(rates), [667, 667, 668])
        self.assertEqual(sum(rates), 2002)
        self.assertEqual(sum(calls), 4004)
        self.assertEqual(len(set(ports)), 3)
        self.assertTrue(all(limit > count for limit, count in zip(limits, calls)))
        self.assertEqual(
            names,
            ["rvoip_2002cps_s0", "rvoip_2002cps_s1", "rvoip_2002cps_s2"],
        )

        row = self.matrix_row(results)
        self.assertEqual(row[0], "PASS")
        self.assertEqual(row[4:6], ["2002", "4004"])
        self.assertEqual(row[7], "0")
        self.assertEqual(len(row[8].split(",")), 3)
        analysis = (results / "analysis.md").read_text()
        self.assertIn("| rvoip | 2002 | 3 | 4004 | 4004 |", analysis)

    def test_nonzero_shard_exit_fails_even_with_complete_csv(self):
        completed, results, _ = self.run_harness(
            cps=2000,
            shard_cps=1000,
            affected_suffix="_s1",
            exit_code=255,
        )

        self.assertEqual(completed.returncode, 1)
        row = self.matrix_row(results)
        self.assertEqual(row[0], "FAIL")
        self.assertEqual(row[7], "255")
        self.assertIn("nonzero_sipp_rc=255", completed.stdout)

    def test_incomplete_current_and_failed_calls_each_fail(self):
        for mode in ("incomplete", "current", "failed"):
            with self.subTest(mode=mode):
                completed, results, _ = self.run_harness(cps=30, mode=mode)
                self.assertEqual(completed.returncode, 1)
                self.assertEqual(self.matrix_row(results)[0], "FAIL")
                self.assertIn("expected=60", completed.stdout)


if __name__ == "__main__":
    unittest.main()
