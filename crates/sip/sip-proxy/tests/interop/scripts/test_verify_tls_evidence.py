#!/usr/bin/env python3
"""Fail-closed tests for the external TLS evidence verifier."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parent
TLS_HELPERS = SCRIPT_DIR / "tls.sh"
MODULE_SPEC = importlib.util.spec_from_file_location(
    "verify_tls_evidence", SCRIPT_DIR / "verify_tls_evidence.py"
)
assert MODULE_SPEC and MODULE_SPEC.loader
verifier = importlib.util.module_from_spec(MODULE_SPEC)
MODULE_SPEC.loader.exec_module(verifier)


class TlsEvidenceVerifierTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls._base_temporary = tempfile.TemporaryDirectory()
        cls.base = Path(cls._base_temporary.name)
        cls.fixture = cls.base / "fixture"
        cls.fixture.mkdir()
        subprocess.run(
            [
                "bash",
                "-c",
                """
set -euo pipefail
source "$1"
tls_prepare_pki "$3"
tls_select_peer kamailio
tls_copy_public_evidence "$2" kamailio
tls_cleanup_pki
""",
                "tls-fixture",
                str(TLS_HELPERS),
                str(cls.fixture),
                str(cls.base),
            ],
            check=True,
            text=True,
            capture_output=True,
        )
        sipp_sha256 = (
            cls.fixture / "tls-public/sipp.der.sha256"
        ).read_text().split()[0]
        rvoip_sha256 = (
            cls.fixture / "tls-public/rvoip.der.sha256"
        ).read_text().split()[0]
        peer_sha256 = (
            cls.fixture / "tls-public/peer.der.sha256"
        ).read_text().split()[0]
        rvoip_metadata = (
            cls.fixture / "tls-public/rvoip.metadata.txt"
        ).read_text()
        rvoip_serial = re.search(
            r"^serial=([0-9A-Fa-f]+)$", rvoip_metadata, re.MULTILINE
        )
        assert rvoip_serial
        positive_logs = {
            "tls-rvoip-to-peer-positive.log": "kamailio.proxy.test",
            "tls-peer-to-rvoip-positive.log": "rvoip.proxy.test",
        }
        for name, identity in positive_logs.items():
            (cls.fixture / name).write_text(
                "\n".join(
                    (
                        "CONNECTION ESTABLISHED",
                        "Protocol version: TLSv1.2",
                        f"Peer certificate: CN = {identity}",
                        "Verification: OK",
                        f"Verified peername: {identity}",
                        "DONE",
                    )
                )
                + "\n"
            )
        for name, error in {
            "tls-rvoip-to-peer-wrong-name.log": "hostname mismatch",
            "tls-rvoip-to-peer-wrong-ca.log": (
                "self-signed certificate in certificate chain"
            ),
            "tls-peer-to-rvoip-wrong-name.log": "hostname mismatch",
            "tls-peer-to-rvoip-wrong-ca.log": "certificate verify failed",
            "tls-peer-rejects-untrusted-client.log": "tlsv1 alert unknown ca",
            "tls-rvoip-rejects-untrusted-client.log": "bad certificate",
        }.items():
            (cls.fixture / name).write_text(f"{error}\n")
        (cls.fixture / "rvoip.log").write_text(
            "RVOIP_PROXY_READY transport=tls listen=127.0.0.1:1 "
            "tls_dns_authority=true\n"
            "RVOIP_TLS_PEER_ACCEPTED direction=inbound transport=tls "
            f"leaf_certificate_sha256={sipp_sha256} "
            "presented_chain_len=1\n"
        )
        (cls.fixture / "peer.log").write_text(
            "INTEROP_TLS_VERIFIED direction=inbound "
            f"peer_serial={rvoip_serial.group(1)}\n"
        )
        (
            cls.fixture / "tls-kamailio-outbound-boundary.log"
        ).write_text(
            "TLS_BOUNDARY_READY mode=client "
            "expected_peer_dns=sipp.proxy.test "
            "server_identity=sipp.proxy.test\n"
            "TLS_BOUNDARY_ACCEPTED mode=client "
            "expected_peer_dns=sipp.proxy.test "
            "presented_peer_dns=sipp.proxy.test "
            "server_name=sipp.proxy.test "
            f"leaf_certificate_sha256={sipp_sha256} "
            "protocol=TLSv1.2\n"
        )
        (cls.fixture / "tls-boundary-client.log").write_text(
            "TLS_BOUNDARY_READY mode=client "
            "expected_peer_dns=rvoip.proxy.test "
            "server_identity=sipp.proxy.test\n"
            "TLS_BOUNDARY_ACCEPTED mode=client "
            "expected_peer_dns=rvoip.proxy.test "
            "presented_peer_dns=rvoip.proxy.test "
            "server_name=rvoip.proxy.test "
            f"leaf_certificate_sha256={rvoip_sha256} "
            "protocol=TLSv1.2\n"
        )
        (cls.fixture / "tls-boundary-server.log").write_text(
            "TLS_BOUNDARY_READY mode=server "
            "expected_peer_dns=kamailio.proxy.test "
            "server_identity=sipp.proxy.test\n"
            "TLS_BOUNDARY_ACCEPTED mode=server "
            "expected_peer_dns=kamailio.proxy.test "
            "presented_peer_dns=kamailio.proxy.test "
            "server_name=sipp.proxy.test "
            f"leaf_certificate_sha256={peer_sha256} "
            "protocol=TLSv1.2\n"
        )
        for index, scenario in enumerate(verifier.TLS_PACKET_SCENARIOS):
            capture_name = f"{scenario}--lo0.pcap"
            capture_bytes = (
                b"\xd4\xc3\xb2\xa1" + bytes([index + 1]) * 60
            )
            capture_path = cls.fixture / capture_name
            capture_path.write_bytes(capture_bytes)
            assertion_names = (
                verifier.BASE_PACKET_ASSERTIONS
                | verifier.SCENARIO_PACKET_ASSERTIONS.get(scenario, set())
            )
            scenario_dir = cls.fixture / "scenarios" / scenario
            scenario_dir.mkdir(parents=True)
            (scenario_dir / "packet-evidence.json").write_text(
                json.dumps(
                    {
                        "schema": verifier.PACKET_SCHEMA,
                        "scenario": scenario,
                        "status": "PASS",
                        "peer": "kamailio",
                        "order": "rvoip-first",
                        "transport": "tls",
                        "captures": [
                            {
                                "filename": capture_name,
                                "sha256": hashlib.sha256(
                                    capture_bytes
                                ).hexdigest(),
                                "bytes": len(capture_bytes),
                            }
                        ],
                        # A pooled TLS connection can put a ClientHello and
                        # certificate flight in an earlier scenario capture.
                        # The verifier must aggregate identities across the
                        # complete immutable row rather than require them in
                        # every scenario.
                        "observed_sni": (
                            [
                                "rvoip.proxy.test",
                                "kamailio.proxy.test",
                                "sipp.proxy.test",
                            ]
                            if index == 0
                            else []
                        ),
                        "observed_certificate_sha256": (
                            [rvoip_sha256, peer_sha256, sipp_sha256]
                            if index == 0
                            else []
                        ),
                        "assertions": [
                            {
                                "name": name,
                                "passed": True,
                                "observed": "fixture",
                            }
                            for name in sorted(assertion_names)
                        ],
                    },
                    indent=2,
                    sort_keys=True,
                )
                + "\n"
            )

    @classmethod
    def tearDownClass(cls) -> None:
        cls._base_temporary.cleanup()

    def setUp(self) -> None:
        self._temporary = tempfile.TemporaryDirectory()
        self.row = Path(self._temporary.name) / "row"
        shutil.copytree(self.fixture, self.row)

    def tearDown(self) -> None:
        self._temporary.cleanup()

    def verify(self) -> dict:
        return verifier.verify_row(self.row, "kamailio", "rvoip-first")

    def test_valid_fixture_is_derived_from_raw_evidence(self) -> None:
        result = self.verify()
        self.assertEqual(result["schema"], verifier.SCHEMA)
        self.assertEqual(result["result"], "PASS")
        self.assertEqual(
            result["actual_sip"]["rvoip_expected_client"], "sipp"
        )
        self.assertEqual(
            result["packet_aggregate"]["observed_sni"],
            [
                "kamailio.proxy.test",
                "rvoip.proxy.test",
                "sipp.proxy.test",
            ],
        )
        self.assertEqual(
            result["packet_aggregate"]["capture_count"],
            len(verifier.TLS_PACKET_SCENARIOS),
        )

    def test_missing_scenario_packet_evidence_fails(self) -> None:
        (
            self.row
            / "scenarios/invite-success/packet-evidence.json"
        ).unlink()
        with self.assertRaisesRegex(
            verifier.VerificationError, "invite-success TLS packet evidence"
        ):
            self.verify()

    def test_incomplete_row_sni_aggregate_fails(self) -> None:
        path = (
            self.row
            / "scenarios/options-readiness/packet-evidence.json"
        )
        packet = json.loads(path.read_text())
        packet["observed_sni"].remove("sipp.proxy.test")
        path.write_text(json.dumps(packet, indent=2, sort_keys=True) + "\n")
        with self.assertRaisesRegex(
            verifier.VerificationError, "SNI aggregate is incomplete"
        ):
            self.verify()

    def test_tampered_packet_capture_fails(self) -> None:
        path = self.row / "invite-success--lo0.pcap"
        path.write_bytes(path.read_bytes() + b"tampered")
        with self.assertRaisesRegex(
            verifier.VerificationError, "capture hash/size mismatch"
        ):
            self.verify()

    def test_missing_dialog_route_assertion_fails(self) -> None:
        path = (
            self.row
            / "scenarios/invite-success/packet-evidence.json"
        )
        packet = json.loads(path.read_text())
        packet["assertions"] = [
            item
            for item in packet["assertions"]
            if item["name"]
            != "invite-dialog-uac-ack-bye-use-contact-and-reversed-route-set"
        ]
        path.write_text(json.dumps(packet, indent=2, sort_keys=True) + "\n")
        with self.assertRaisesRegex(
            verifier.VerificationError,
            "invite-success TLS packet assertions are missing or failed",
        ):
            self.verify()

    def test_tampered_certificate_fails(self) -> None:
        shutil.copyfile(
            self.row / "tls-public/rvoip.pem",
            self.row / "tls-public/peer.pem",
        )
        with self.assertRaises(verifier.VerificationError):
            self.verify()

    def test_tampered_recorded_hash_fails(self) -> None:
        (self.row / "tls-public/peer.der.sha256").write_text(
            f"{'0' * 64}  -\n"
        )
        with self.assertRaisesRegex(
            verifier.VerificationError, "recorded DER hash"
        ):
            self.verify()

    def test_tampered_positive_log_fails(self) -> None:
        path = self.row / "tls-rvoip-to-peer-positive.log"
        path.write_text(path.read_text().replace("Verification: OK\n", ""))
        with self.assertRaisesRegex(
            verifier.VerificationError, "positive TLS handshake"
        ):
            self.verify()

    def test_wrong_peer_identity_fails(self) -> None:
        with self.assertRaises(verifier.VerificationError):
            verifier.verify_row(self.row, "opensips", "rvoip-first")

    def test_missing_negative_control_fails(self) -> None:
        (self.row / "tls-peer-to-rvoip-wrong-ca.log").unlink()
        with self.assertRaisesRegex(
            verifier.VerificationError, "wrong-CA rejection is missing"
        ):
            self.verify()

    def test_missing_peer_outbound_boundary_fails(self) -> None:
        (self.row / "tls-kamailio-outbound-boundary.log").unlink()
        with self.assertRaisesRegex(
            verifier.VerificationError,
            "kamailio outbound hostname-verifying boundary log",
        ):
            self.verify()

    def test_private_key_artifact_is_rejected(self) -> None:
        (self.row / "leaked.key.pem").write_text(
            "-----BEGIN PRIVATE KEY-----\ncanary\n"
            "-----END PRIVATE KEY-----\n"
        )
        with self.assertRaisesRegex(
            verifier.VerificationError, "private-key artifact"
        ):
            self.verify()

    def test_raw_inspect_artifact_is_rejected(self) -> None:
        (self.row / "container-inspect.json").write_text("{}\n")
        with self.assertRaisesRegex(
            verifier.VerificationError, "raw inspect"
        ):
            self.verify()

    def test_symlink_is_rejected(self) -> None:
        (self.row / "evidence-link").symlink_to(
            self.row / "tls-public/ca.pem"
        )
        with self.assertRaisesRegex(verifier.VerificationError, "symlink"):
            self.verify()


if __name__ == "__main__":
    unittest.main()
