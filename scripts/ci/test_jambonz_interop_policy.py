#!/usr/bin/env python3
"""Static policy tests for the Jambonz release peer."""

from __future__ import annotations

from pathlib import Path
import json
import re
import unittest


ROOT = Path(__file__).resolve().parents[2]
PINS = ROOT / "infra/release-runners/pbx/jambonz/versions.env"
CATALOG = ROOT / "scripts/release/gates.json"
LAB = ROOT / "infra/release-runners/pbx/jambonz"


class JambonzInteropPolicyTests(unittest.TestCase):
    def pin_values(self) -> dict[str, str]:
        return dict(
            line.split("=", 1)
            for line in PINS.read_text(encoding="utf-8").splitlines()
            if line and not line.startswith("#")
        )

    def test_sources_and_images_are_immutable(self) -> None:
        text = PINS.read_text(encoding="utf-8")
        values = dict(
            line.split("=", 1)
            for line in text.splitlines()
            if line and not line.startswith("#")
        )
        for key in ("JAMBONZ_INBOUND_COMMIT", "JAMBONZ_OUTBOUND_COMMIT"):
            self.assertRegex(values[key], r"^[0-9a-f]{40}$")
        for key, value in values.items():
            if key.endswith("_IMAGE"):
                self.assertRegex(value, r"@sha256:[0-9a-f]{64}$")
                self.assertNotIn(":latest", value)
        for key in (
            "JAMBONZ_INBOUND_TARBALL_SHA256",
            "JAMBONZ_OUTBOUND_TARBALL_SHA256",
        ):
            self.assertRegex(values[key], r"^[0-9a-f]{64}$")

    def test_current_oss_component_line_is_explicit(self) -> None:
        text = PINS.read_text(encoding="utf-8")
        match = re.search(r"(?m)^JAMBONZ_RELEASE_LINE=([^\s]+)$", text)
        self.assertIsNotNone(match)
        self.assertEqual(match.group(1), "0.9.9")

    def test_release_profile_uses_the_shared_rvoip_sip_matrix(self) -> None:
        catalog = json.loads(CATALOG.read_text(encoding="utf-8"))
        by_id = {gate["id"]: gate for gate in catalog["gates"]}
        expected = {
            "interop.jambonz-latest",
            "interop.jambonz-up",
            "interop.jambonz-matrix",
            "interop.jambonz-down",
        }
        self.assertTrue(expected <= set(catalog["profiles"]["remote-release"]))
        matrix = by_id["interop.jambonz-matrix"]
        command = matrix["command"]
        self.assertIn(
            "{workspace}/crates/sip/rvoip-sip/examples/pbx/run.sh", command
        )
        self.assertEqual(command[command.index("--pbx") + 1], "jambonz")
        self.assertEqual(command[command.index("--api") + 1], "all")
        self.assertEqual(command[command.index("--scenario") + 1], "all")
        self.assertIn("PBX_G711_PROFILES=pcmu pcma", command)
        self.assertNotIn("PBX_G729_PROFILES=g729a g729ab", command)
        self.assertEqual(matrix["dependencies"], ["interop.jambonz-up"])
        self.assertEqual(
            by_id["interop.jambonz-down"]["dependencies"],
            ["interop.jambonz-matrix"],
        )

    def test_lab_does_not_define_a_parallel_sip_scenario_suite(self) -> None:
        scenario_suffixes = {".xml", ".pcap"}
        found = [path for path in LAB.rglob("*") if path.suffix in scenario_suffixes]
        self.assertEqual(found, [])

    def test_release_lab_fails_closed_on_non_amd64_engines(self) -> None:
        up = (LAB / "up.sh").read_text(encoding="utf-8")
        self.assertIn("docker info --format '{{.Architecture}}'", up)
        self.assertIn("amd64|x86_64", up)
        self.assertIn("x86_64 Colima profile", up)

    def test_colima_uses_explicit_loopback_and_host_gateway_routing(self) -> None:
        up = (LAB / "up.sh").read_text(encoding="utf-8")
        override = (LAB / "docker-compose.colima.yml").read_text(encoding="utf-8")
        self.assertIn('$(uname -s)', up)
        self.assertIn("docker-compose.colima.yml", up)
        self.assertIn("host.docker.internal", up)
        self.assertIn("colima-host-forward", up)
        self.assertIn("verify_colima_udp_forwarding", up)
        self.assertIn("--port-forwarder grpc", up)
        self.assertIn("127.0.0.1", override)
        self.assertRegex(override, r"55060.*55060/udp")
        self.assertIn("--external-ip", override)
        self.assertIn("--dns-name", override)
        self.assertIn("sip.rvoip.test", override)
        self.assertEqual(override.count("JAMBONZ_RTP_PORT_START:-10000"), 2)
        self.assertEqual(override.count("JAMBONZ_RTP_PORT_END:-10199"), 2)
        self.assertIn("${JAMBONZ_RTP_PORT_END:-10199}/udp", override)
        compose = (LAB / "docker-compose.yml").read_text(encoding="utf-8")
        self.assertIn("JAMBONZ_RTP_ADVERTISED_IP", compose)
        self.assertIn("--port-min", compose)
        self.assertIn("--port-max", compose)

    def test_logical_sip_realm_is_dns_named_and_consistent(self) -> None:
        realm = "sip.rvoip.test"
        up = (LAB / "up.sh").read_text(encoding="utf-8")
        compose = (LAB / "docker-compose.yml").read_text(encoding="utf-8")
        fixture = (LAB / "rvoip.sql").read_text(encoding="utf-8")
        example = (
            ROOT / "crates/sip/rvoip-sip/examples/pbx/env/jambonz.env.example"
        ).read_text(encoding="utf-8")

        self.assertIn(f"JAMBONZ_SIP_DOMAIN={realm}", up)
        self.assertIn(f"JAMBONZ_SIP_DOMAIN={realm}", example)
        self.assertIn(f'"credentials":{{"{realm}"', compose)
        self.assertEqual(fixture.count(f"'{realm}'"), 2)
        self.assertIn("http://auth-server:4000/auth", fixture)
        self.assertNotRegex(realm, r"^(?:[0-9]{1,3}\.){3}[0-9]{1,3}$")

    def test_shared_harness_uses_public_custom_header_api(self) -> None:
        harness = (
            ROOT / "crates/sip/rvoip-sip/examples/pbx/common.rs"
        ).read_text(encoding="utf-8")

        self.assertIn("SipRequestOptions::with_headers", harness)
        self.assertIn(".with_headers(cfg.outbound_invite_headers())?", harness)
        self.assertIn('HeaderName::Other("X-Account-Sid".into())', harness)
        self.assertIn('HeaderName::Other("X-Jambonz-Routing".into())', harness)
        self.assertIn('HeaderName::Other("X-Call-Sid".into())', harness)
        self.assertIn("uuid::Uuid::new_v4()", harness)
        self.assertNotIn("with_extra_headers(", harness)

        runner = (
            ROOT / "crates/sip/rvoip-sip/examples/pbx/run.sh"
        ).read_text(encoding="utf-8")
        self.assertIn('PBX_G711_PROFILES="${PBX_G711_PROFILES:-default}"', runner)
        self.assertIn("g729_call|amr_call|amr_transcode_call|b2bua_call", runner)

    def test_up_requires_every_interop_component_to_stay_running(self) -> None:
        up = (LAB / "up.sh").read_text(encoding="utf-8")
        compose = (LAB / "docker-compose.yml").read_text(encoding="utf-8")
        self.assertIn("JAMBONES_TIME_SERIES_HOST", compose)
        for component in (
            "mysql",
            "drachtio",
            "redis",
            "rtpengine",
            "registrar",
            "auth",
            "sbc-outbound",
            "influxdb",
        ):
            self.assertIn(f"rvoip-jambonz-{component}", up)
        self.assertIn("Required Jambonz container is not running", up)

    def test_public_and_release_docs_define_the_same_jambonz_profile(self) -> None:
        release_line = self.pin_values()["JAMBONZ_RELEASE_LINE"]
        documents = {
            "README.md": ROOT / "README.md",
            "SIP README": ROOT / "crates/sip/rvoip-sip/README.md",
            "release notes": ROOT
            / "crates/sip/rvoip-sip/docs/RELEASE_NOTES_NEXT.md",
            "compatibility matrix": ROOT
            / "crates/sip/rvoip-sip/docs/COMPATIBILITY_MATRIX.md",
            "interop plan": ROOT / "crates/sip/rvoip-sip/docs/INTEROP_CI_PLAN.md",
            "topology profiles": ROOT
            / "crates/sip/rvoip-sip/docs/TOPOLOGY_PROFILES.md",
            "changelog": ROOT / "CHANGELOG.md",
        }
        for label, path in documents.items():
            with self.subTest(document=label):
                text = path.read_text(encoding="utf-8")
                self.assertIn("Jambonz", text)
                self.assertIn(release_line, text)

        required_scope = (
            "Endpoint",
            "StreamPeer",
            "CallbackPeer",
            "UDP",
            "PCMU",
            "PCMA",
            "DTMF",
            "CANCEL",
            "transfer",
        )
        for label in ("release notes", "compatibility matrix"):
            text = documents[label].read_text(encoding="utf-8")
            with self.subTest(scope_document=label):
                for term in required_scope:
                    self.assertIn(term, text)
                for exclusion in ("G.729", "AMR", "TLS/SRTP", "WebRTC", "PSTN"):
                    self.assertIn(exclusion, text)

    def test_transfer_documentation_keeps_referred_by_optional(self) -> None:
        documents = (
            ROOT / "crates/sip/rvoip-sip/docs/RELEASE_NOTES_NEXT.md",
            ROOT / "crates/sip/rvoip-sip/docs/RFC_COMPLIANCE_MATRIX.md",
            ROOT / "docs/sip/SIP_RFC_COMPLIANCE.md",
        )
        for path in documents:
            text = path.read_text(encoding="utf-8")
            normalized = " ".join(text.split())
            with self.subTest(document=str(path.relative_to(ROOT))):
                self.assertIn("Referred-By", text)
                self.assertRegex(
                    normalized,
                    r"(?i)Referred-By.{0,160}optional|optional.{0,160}Referred-By",
                )


if __name__ == "__main__":
    unittest.main()
