#!/usr/bin/env python3
"""The SIPp comparison harness never binds the target's own port."""

from __future__ import annotations

from pathlib import Path
import subprocess
import unittest


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "crates/sip/rvoip-sip/tests/perf/sipp_scenarios/run_comparison.sh"


class SippComparisonPortTests(unittest.TestCase):
    def port_selection(self) -> str:
        text = SCRIPT.read_text(encoding="utf-8")
        start = text.index("  while :; do\n    BASE_PORT=")
        end = text.index("  done\n", start) + len("  done\n")
        return text[start:end]

    def test_base_port_draw_excludes_the_target_port(self) -> None:
        # The release matrix targets 35060, inside the 35000-35999 draw range;
        # an unguarded draw failed a whole rate with errno 98 in 36698782401.
        loop = self.port_selection()
        probe = f"""
set -euo pipefail
TARGET_PORT=35060
for RUNNERS in 1 2 4 8; do
  for _ in $(seq 1 4000); do
{loop}
    if (( TARGET_PORT >= BASE_PORT && TARGET_PORT < BASE_PORT + RUNNERS )); then
      echo "collision base=$BASE_PORT runners=$RUNNERS"; exit 1
    fi
  done
done
"""
        result = subprocess.run(
            ["bash", "-c", probe], capture_output=True, text=True, check=False
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
