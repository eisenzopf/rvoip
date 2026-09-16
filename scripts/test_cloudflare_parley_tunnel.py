#!/usr/bin/env python3
"""Guard the Parley Cloudflare Tunnel ingress hostnames."""

from pathlib import Path
import subprocess
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
CONFIG = ROOT / "deploy" / "cloudflare" / "config.yml"
SCRIPT = ROOT / "scripts" / "run-cloudflare-tunnel.sh"


class CloudflareParleyTunnelTests(unittest.TestCase):
    def test_ingress_maps_http_and_uctp_not_apex(self):
        text = CONFIG.read_text(encoding="utf-8")
        self.assertIn("hostname: parley.rudeless.ai", text)
        self.assertIn("hostname: parley-uctp.rudeless.ai", text)
        self.assertIn("http://127.0.0.1:8080", text)
        self.assertIn("http://127.0.0.1:7443", text)
        self.assertNotIn("hostname: rudeless.ai", text)
        self.assertNotIn(":5060", text)

    def test_runner_is_valid_bash(self):
        subprocess.run(["bash", "-n", str(SCRIPT)], check=True)


if __name__ == "__main__":
    sys.exit(0 if unittest.main(verbosity=2).result.wasSuccessful() else 1)
