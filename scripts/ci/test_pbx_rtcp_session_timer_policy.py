#!/usr/bin/env python3
"""Static policy tests for the PBX RTCP and session-timer release cells (#318).

The Asterisk and FreeSWITCH release gates run `examples/pbx/run.sh
--scenario all`. These tests pin that the RTCP and RFC 4028 cells are part of
that run, that the runner itself fails a full run missing any of them, and
that the PBX configuration they need exists only as dedicated endpoints and
profiles, so every pre-existing row keeps its meaning.
"""

from __future__ import annotations

from pathlib import Path
import json
import re
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
CATALOG = ROOT / "scripts/release/gates.json"
PBX = ROOT / "crates/sip/rvoip-sip/examples/pbx"
RUNNER = PBX / "run.sh"
EVIDENCE = PBX / "pbx_evidence.sh"
ASTERISK = ROOT / "infra/release-runners/pbx/asterisk/config"
FREESWITCH = ROOT / "infra/release-runners/pbx/freeswitch/docker-entrypoint.sh"
LIFECYCLE = ROOT / "infra/release-runners/interop-lifecycle.sh"

NEW_SCENARIOS = ("rtcp_mux", "rtcp_non_mux", "rtcp_mux_declined", "session_timer")
REQUIRED_CELLS = {
    "asterisk": {
        ("rtcp_mux", "UDP"),
        ("rtcp_mux", "TLS"),
        ("rtcp_non_mux", "UDP"),
        ("rtcp_non_mux", "TLS"),
        ("rtcp_mux_declined", "UDP"),
        ("rtcp_mux_declined", "TLS"),
        ("session_timer", "UDP"),
    },
    "freeswitch": {("rtcp_mux", "UDP"), ("session_timer", "UDP")},
}


def shell_function(text: str, name: str) -> str:
    """The body of a top-level POSIX shell function in run.sh."""
    match = re.search(rf"(?ms)^{re.escape(name)}\(\) \{{\n(.*?)^\}}\n", text)
    if match is None:
        raise AssertionError(f"run.sh has no function {name}()")
    return match.group(1)


def parse_pjsip(text: str) -> dict[str, dict[str, list[str]]]:
    """Asterisk config sections with template inheritance resolved.

    Several sections share a name (endpoint, aor); they are keyed by name and
    `type`, which is how Asterisk tells them apart.
    """
    raw: list[tuple[str, list[str], bool, list[tuple[str, str]]]] = []
    for line in text.splitlines():
        line = line.split(";", 1)[0].strip()
        if not line:
            continue
        header = re.fullmatch(r"\[([^\]]+)\](?:\(([^)]*)\))?", line)
        if header:
            options = [o.strip() for o in (header.group(2) or "").split(",") if o.strip()]
            raw.append(
                (header.group(1), [o for o in options if o != "!"], "!" in options, [])
            )
            continue
        key, _, value = line.partition("=")
        raw[-1][3].append((key.strip(), value.strip()))
    templates = {name: (parents, pairs) for name, parents, is_template, pairs in raw if is_template}

    def resolve(parents: list[str], pairs: list[tuple[str, str]]) -> dict[str, list[str]]:
        values: dict[str, list[str]] = {}
        for parent in parents:
            for key, vals in resolve(*templates[parent]).items():
                values.setdefault(key, []).extend(vals)
        for key, value in pairs:
            if key in values and key not in ("allow",):
                values[key] = [value]
            else:
                values.setdefault(key, []).append(value)
        return values

    sections: dict[str, dict[str, list[str]]] = {}
    for name, parents, is_template, pairs in raw:
        if is_template:
            continue
        values = resolve(parents, pairs)
        kind = values.get("type", ["?"])[-1]
        sections[f"{name}/{kind}"] = values
    return sections


class PbxRtcpSessionTimerPolicyTests(unittest.TestCase):
    def runner(self) -> str:
        return RUNNER.read_text(encoding="utf-8")

    def test_release_gates_run_the_full_scenario_list_on_the_endpoint_api(self) -> None:
        catalog = json.loads(CATALOG.read_text(encoding="utf-8"))
        by_id = {gate["id"]: gate for gate in catalog["gates"]}
        for gate_id, provider in (
            ("interop.asterisk-matrix", "asterisk"),
            ("interop.freeswitch-matrix", "freeswitch"),
        ):
            with self.subTest(gate=gate_id):
                command = by_id[gate_id]["command"]
                self.assertIn("{workspace}/crates/sip/rvoip-sip/examples/pbx/run.sh", command)
                self.assertEqual(command[command.index("--pbx") + 1], provider)
                # `all` includes the endpoint API, the only one these cells use.
                self.assertIn(command[command.index("--api") + 1], ("all", "endpoint"))
                self.assertEqual(command[command.index("--scenario") + 1], "all")
                self.assertNotIn("--transport", command)

    def test_scenario_all_includes_every_new_cell(self) -> None:
        body = shell_function(self.runner(), "scenario_list")
        all_line = next(line for line in body.splitlines() if line.strip().startswith("all)"))
        for scenario in NEW_SCENARIOS:
            self.assertRegex(all_line, rf"\b{scenario}\b")

    def test_runner_cell_selection_matches_the_required_cells(self) -> None:
        text = self.runner()
        supported = shell_function(text, "provider_scenario_supported")
        self.assertIn("rtcp_mux|session_timer)", supported)
        self.assertIn("asterisk|freeswitch) return 0 ;;", supported)
        self.assertIn("rtcp_non_mux|rtcp_mux_declined)", supported)
        self.assertIn('[ "$pss_provider" = "asterisk" ]', supported)

        cell = shell_function(text, "run_matrix_cell")
        self.assertIn("rtcp_mux|rtcp_non_mux|rtcp_mux_declined|session_timer)", cell)
        self.assertIn('run_rtcp_timer_cell "$provider" "$example" "$scenario" UDP', cell)
        self.assertIn('run_rtcp_timer_cell "$provider" "$example" "$scenario" TLS', cell)
        self.assertIn('[ "$provider" = "asterisk" ] && [ "$scenario" != "session_timer" ]', cell)

        required = shell_function(text, "required_rtcp_timer_cells")
        parsed = {
            provider: set(re.findall(r'"(\w+) (UDP|TLS)"', block))
            for provider, block in re.findall(r"(?ms)^\s*(asterisk|freeswitch)\)(.*?);;", required)
        }
        self.assertEqual(parsed, REQUIRED_CELLS)

    def test_a_full_run_fails_when_a_required_cell_is_missing(self) -> None:
        text = self.runner()
        self.assertIn('if [ "$SCENARIO_ARG" = "all" ]; then', text)
        self.assertIn('verify_required_rtcp_timer_cells "$provider" || RUN_FAILURES=1', text)
        verify = shell_function(text, "verify_required_rtcp_timer_cells")
        # A cell counts only through its PBX verdict row, which run.sh records
        # after both rvoip roles passed.
        self.assertIn('$1 == "PASS"', verify)
        self.assertIn('$6 == "pbx"', verify)
        self.assertIn("record_matrix FAIL", verify)

    def test_every_cell_records_a_pbx_verdict(self) -> None:
        text = self.runner()
        cell = shell_function(text, "run_rtcp_timer_cell")
        self.assertIn("run_pbx_evidence_check", cell)
        check = shell_function(text, "run_pbx_evidence_check")
        self.assertIn('sh "$SCRIPT_DIR/pbx_evidence.sh" check', check)
        self.assertIn('"$rpec_transport" pbx', check)
        evidence = EVIDENCE.read_text(encoding="utf-8")
        for case in (
            "asterisk:rtcp_mux|asterisk:rtcp_non_mux|asterisk:rtcp_mux_declined)",
            "asterisk:session_timer)",
            "freeswitch:rtcp_mux)",
            "freeswitch:session_timer)",
        ):
            self.assertIn(case, evidence)

    def test_pbx_debug_switches_are_turned_back_off(self) -> None:
        commands = shell_function(self.runner(), "pbx_evidence_commands")
        for switch in ("rtcp set debug", "rtp set debug", "pjsip set logger", "sofia global siptrace"):
            self.assertIn(f"{switch} $pec_state", commands)
        end = shell_function(self.runner(), "pbx_evidence_end")
        self.assertIn('pbx_evidence_commands "$pee_provider" "$pee_scenario" off', end)

    def test_kept_pbx_logs_redact_digest_credentials(self) -> None:
        collect = shell_function(self.runner(), "pbx_collect_log")
        awk = re.search(r"awk '(\{.*?\})'", collect, re.S).group(1)
        sample = (
            "WWW-Authenticate: Digest realm=x, nonce=secret1\n"
            "[2026-10-12 07:15:38] Authorization: Digest response=secret2\n"
            "Proxy-Authorization: Digest response=secret3\n"
            "\x1b[1;30mCSeq: 2 INVITE\x1b[0m\n"
        )
        result = subprocess.run(
            ["awk", awk], input=sample, capture_output=True, text=True, check=True
        ).stdout
        for secret in ("secret1", "secret2", "secret3"):
            self.assertNotIn(secret, result)
        self.assertEqual(result.count("<redacted>"), 3)
        self.assertIn("CSeq: 2 INVITE", result)
        self.assertNotIn("\x1b", result)

    def test_asterisk_endpoints_are_dedicated_and_existing_ones_unchanged(self) -> None:
        sections = parse_pjsip((ASTERISK / "pjsip.conf").read_text(encoding="utf-8"))
        endpoint = lambda user: sections[f"{user}/endpoint"]  # noqa: E731

        for user in ("1001", "1002", "1003", "2001", "2002", "2003"):
            with self.subTest(existing=user):
                self.assertNotIn("rtcp_mux", endpoint(user))
                self.assertEqual(endpoint(user)["timers"], ["no"])
                self.assertNotIn("timers_sess_expires", endpoint(user))

        for user, transport, rtcp_mux in (
            ("1101", "transport-tls", "yes"),
            ("1102", "transport-tls", "yes"),
            ("2101", "transport-udp", "yes"),
            ("2102", "transport-udp", "yes"),
            ("1201", "transport-tls", "no"),
            ("1202", "transport-tls", "no"),
            ("2201", "transport-udp", "no"),
            ("2202", "transport-udp", "no"),
        ):
            with self.subTest(rtcp=user):
                self.assertEqual(endpoint(user)["rtcp_mux"], [rtcp_mux])
                self.assertEqual(endpoint(user)["transport"], [transport])
                self.assertEqual(endpoint(user)["timers"], ["no"])
                if transport == "transport-tls":
                    self.assertEqual(endpoint(user)["media_encryption"], ["sdes"])
                self.assertEqual(sections[f"auth{user}/auth"]["password"], ["<redacted>"])
                self.assertIn(f"{user}/aor", sections)

        for user in ("2301", "2302"):
            with self.subTest(timer=user):
                self.assertEqual(endpoint(user)["timers"], ["yes"])
                # RFC 4028's 90 s floor keeps a refresh inside a short cell.
                self.assertEqual(endpoint(user)["timers_sess_expires"], ["90"])
                self.assertNotIn("rtcp_mux", endpoint(user))

        dialplan = (ASTERISK / "extensions.conf").read_text(encoding="utf-8")
        self.assertIn("exten => _[12][12]0[12],1,", dialplan)
        self.assertIn("exten => _230[12],1,", dialplan)
        # The existing extensions stay explicit and come first.
        self.assertLess(dialplan.index("exten => 2003,1"), dialplan.index("exten => _[12][12]0[12]"))

    def test_freeswitch_profile_is_dedicated_and_waited_for(self) -> None:
        entrypoint = FREESWITCH.read_text(encoding="utf-8")
        self.assertIn("udp_rtcp_timer_port=${FS_RVOIP_UDP_RTCP_TIMER_SIP_PORT:-5069}", entrypoint)
        self.assertRegex(
            entrypoint,
            r"write_rvoip_profile rvoip_udp_rtcp_timer \"\$udp_rtcp_timer_port\" false false false",
        )
        settings = re.search(
            r"(?s)add_profile_settings rvoip_udp_rtcp_timer \\\n(.*?)\n\}", entrypoint
        ).group(1)
        for setting in (
            "rtcp-audio-interval-msec=5000",
            "enable-timer=true",
            "session-timeout=120",
            "minimum-session-expires=120",
        ):
            self.assertIn(setting, settings)
        # Only the new profile gets the extra settings.
        self.assertEqual(entrypoint.count("add_profile_settings rvoip_"), 1)

        lifecycle = LIFECYCLE.read_text(encoding="utf-8")
        self.assertIn("rvoip_udp_rtcp_timer[[:space:]].*RUNNING", lifecycle)
        self.assertIn("FREESWITCH_RTCP_TIMER_UDP_ADDR=127.0.0.1:5069", lifecycle)
        self.assertIn("FREESWITCH_RTCP_TIMER_UDP_ADDR", self.runner())
        self.assertIn("rtcp_mux|session_timer)", shell_function(self.runner(), "run_one"))

    def test_freeswitch_port_stays_in_the_daemon_block_and_clear_of_the_proxy_labs(self) -> None:
        port = 5069
        self.assertLess(port, 5070)
        for lab in ("kamailio", "opensips"):
            up = (ROOT / f"infra/release-runners/pbx/{lab}/up.sh").read_text(encoding="utf-8")
            ports = {int(value) for value in re.findall(r"_PORT:-(50\d\d)\}", up)}
            self.assertNotIn(port, ports, lab)
        harness = (PBX / "common.rs").read_text(encoding="utf-8")
        self.assertIn('("freeswitch-rtcp-timer", 5069)', harness)

    def test_docs_mark_the_cells_release_gated_without_claiming_evidence(self) -> None:
        documents = {
            "compatibility matrix": ROOT / "crates/sip/rvoip-sip/docs/COMPATIBILITY_MATRIX.md",
            "RFC matrix": ROOT / "crates/sip/rvoip-sip/docs/RFC_COMPLIANCE_MATRIX.md",
            "interop plan": ROOT / "crates/sip/rvoip-sip/docs/INTEROP_CI_PLAN.md",
        }
        for label, path in documents.items():
            with self.subTest(document=label):
                text = " ".join(path.read_text(encoding="utf-8").split())
                self.assertRegex(text, r"release-gated (?:against Asterisk and FreeSWITCH )?from `?0\.4\.1`?")
                self.assertRegex(text, r"(?i)pending|no protected qualification has recorded")
        readme = (PBX / "README.md").read_text(encoding="utf-8")
        for scenario in NEW_SCENARIOS:
            self.assertIn(f"`{scenario}`", readme)


FIXTURES = PBX / "probe-fixtures/rtcp-timer"


class PbxEvidenceParserTests(unittest.TestCase):
    """Pins `pbx_evidence.sh` against trimmed genuine captures from the
    local Asterisk 20.20.1 and FreeSWITCH 1.10.12 labs. Every fixture must
    pass its own check and fail the others, and a one-line mutation of the
    property under test must flip the verdict."""

    def run_check(self, provider: str, scenario: str, fixture: str, mutate=None) -> int:
        with tempfile.TemporaryDirectory() as temporary:
            cell = Path(temporary) / "cell"
            shutil.copytree(FIXTURES / fixture, cell)
            # The repository ignores *.log, so the one-line role logs are
            # stored as <role>-log.txt and restored to the names run.sh uses.
            for role in ("caller", "callee"):
                stored = cell / f"{role}-log.txt"
                if stored.exists():
                    stored.rename(cell / f"{role}.log")
            if mutate:
                mutate(cell)
            result = subprocess.run(
                ["sh", str(EVIDENCE), "check", provider, scenario, "UDP", str(cell)],
                capture_output=True,
                text=True,
            )
            return result.returncode

    def assert_only(self, provider: str, fixture: str, scenario: str, others) -> None:
        self.assertEqual(self.run_check(provider, scenario, fixture), 0, fixture)
        for other in others:
            with self.subTest(fixture=fixture, wrong_check=other):
                self.assertNotEqual(self.run_check(provider, other, fixture), 0)

    def test_asterisk_rtcp_fixtures_prove_exactly_their_mode(self) -> None:
        modes = ("rtcp_mux", "rtcp_non_mux", "rtcp_mux_declined")
        for fixture, scenario in (
            ("asterisk-rtcp-mux-tls", "rtcp_mux"),
            ("asterisk-rtcp-non-mux-udp", "rtcp_non_mux"),
            ("asterisk-rtcp-mux-declined-udp", "rtcp_mux_declined"),
        ):
            self.assert_only(
                "asterisk", fixture, scenario, [m for m in modes if m != scenario]
            )

    def test_rtcp_on_the_rtp_port_fails_the_declined_cell(self) -> None:
        def inject(cell: Path) -> None:
            port = re.search(r"with local port (\d+)", (cell / "caller.log").read_text()).group(1)
            with (cell / "pbx-log.txt").open("a") as log:
                log.write(
                    f"[2026-10-12 07:11:16] Got  RTP packet from    192.0.2.1:{port} "
                    "(type 72, seq 000007, ts 287454020, len 000020)\n"
                )

        self.assertNotEqual(
            self.run_check("asterisk", "rtcp_mux_declined", "asterisk-rtcp-mux-declined-udp", inject),
            0,
        )

    def test_asterisk_rtt_is_required(self) -> None:
        def zero_rtt(cell: Path) -> None:
            log = cell / "pbx-log.txt"
            log.write_text(re.sub(r"RTT: [0-9.]+\(sec\)", "RTT: 0.0000(sec)", log.read_text()))

        self.assertNotEqual(self.run_check("asterisk", "rtcp_mux", "asterisk-rtcp-mux-tls", zero_rtt), 0)

    def test_session_timer_fixtures_need_refreshes_and_the_422(self) -> None:
        for provider, fixture in (
            ("asterisk", "asterisk-session-timer"),
            ("freeswitch", "freeswitch-session-timer"),
        ):
            with self.subTest(fixture=fixture):
                self.assertEqual(self.run_check(provider, "session_timer", fixture), 0)

        def drop_refreshes(cell: Path) -> None:
            # Asterisk refreshes both legs with UPDATE; turn every one into
            # an OPTIONS so neither leg has a refresh.
            log = cell / "pbx-log.txt"
            log.write_text(
                re.sub(r"\bUPDATE\b", "OPTIONS", log.read_text())
            )

        self.assertNotEqual(
            self.run_check("asterisk", "session_timer", "asterisk-session-timer", drop_refreshes),
            0,
        )

        def drop_422(cell: Path) -> None:
            log = cell / "pbx-log.txt"
            log.write_text(log.read_text().replace("SIP/2.0 422 ", "SIP/2.0 499 "))

        self.assertNotEqual(
            self.run_check("freeswitch", "session_timer", "freeswitch-session-timer", drop_422), 0
        )

    def test_freeswitch_rtcp_needs_rvoip_to_echo_its_reports(self) -> None:
        self.assertEqual(self.run_check("freeswitch", "rtcp_mux", "freeswitch-rtcp-mux"), 0)

        def no_echo(cell: Path) -> None:
            events = cell / "pbx-rtcp-events.txt"
            events.write_text(re.sub(r"Source0-LSR: \d+", "Source0-LSR: 0", events.read_text()))

        self.assertNotEqual(
            self.run_check("freeswitch", "rtcp_mux", "freeswitch-rtcp-mux", no_echo), 0
        )


if __name__ == "__main__":
    unittest.main()
