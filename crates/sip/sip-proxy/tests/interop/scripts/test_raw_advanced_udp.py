#!/usr/bin/env python3
"""Focused contract tests for the advanced external UDP drivers."""

from __future__ import annotations

import argparse
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

import raw_advanced_udp as advanced


def fake_request(method: str, scenario: str, branch_name: str) -> bytes:
    extra_headers = (
        "Record-Route: <sip:rvoip.test:5060;lr>",
        "Record-Route: <sip:peer.test:5070;lr>",
    )
    return advanced.make_request(
        method,
        scenario,
        "sip:agent@example.test;transport=udp",
        "127.0.0.1",
        39090,
        f"z9hG4bK-{scenario}-{branch_name}",
        f"{scenario}@example.test",
        "fake-from",
        "<sip:agent@example.test>",
        extra_headers=extra_headers if method == "INVITE" else (),
    )


class FakeHarness:
    def __init__(self, args: argparse.Namespace) -> None:
        self.args = args
        self.branches = {
            name: SimpleNamespace(
                name=name,
                address=address,
                received_counts={},
            )
            for name, address in args.uas_listen
        }
        self.sent_responses: list[tuple[str, bytes, bool]] = []
        self.sent_uac: list[bytes] = []
        self.direct_sends: list[tuple[str, bytes, tuple[str, int]]] = []
        self.delivered_uac_acks: set[int] = set()

    def __enter__(self) -> FakeHarness:
        return self

    def __exit__(self, *_error: object) -> None:
        return None

    def send_uac(self, message: bytes) -> None:
        self.sent_uac.append(message)

    def send_branch(self, name: str, message: bytes) -> None:
        self.sent_responses.append((name, message, False))

    def send_branch_direct(
        self, name: str, message: bytes, destination: tuple[str, int]
    ) -> None:
        self.direct_sends.append((name, message, destination))

    def wait_branch_request(
        self, name: str, method: str, timeout: float | None = None
    ) -> bytes:
        del timeout
        counts = self.branches[name].received_counts
        counts[method] = counts.get(method, 0) + 1
        if method == "ACK":
            branch_port = self.branches[name].address[1]
            for index, message in enumerate(self.sent_uac):
                if index in self.delivered_uac_acks:
                    continue
                if advanced.request_method(
                    message
                ) == "ACK" and f":{branch_port};" in advanced.request_uri(message):
                    self.delivered_uac_acks.add(index)
                    lines = [
                        line
                        for line in message.decode("iso-8859-1").split("\r\n")
                        if not line.lower().startswith("route:")
                    ]
                    return "\r\n".join(lines).encode("iso-8859-1")
        return fake_request(method, self.args.scenario, name)

    def collect_branch_requests(
        self, method: str, names: tuple[str, ...]
    ) -> dict[str, bytes]:
        return {name: self.wait_branch_request(name, method) for name in names}

    def wait_uac_response(
        self, status: int, method: str, timeout: float | None = None
    ) -> bytes:
        del timeout
        for index, (name, response, consumed) in enumerate(self.sent_responses):
            if not consumed and advanced.status_and_method(response) == (
                status,
                method,
            ):
                self.sent_responses[index] = (name, response, True)
                return response
        raise AssertionError(f"fake harness lacks {status} {method}")

    def assert_external_path(self, request: bytes) -> None:
        self.assert_request(request)

    def assert_request(self, request: bytes) -> None:
        if advanced.request_method(request) not in {"INVITE", "OPTIONS", "ACK"}:
            raise AssertionError("unexpected fake path request")

    def assert_no_branch_request(
        self, method: str, names: tuple[str, ...], duration: float
    ) -> None:
        del method, names, duration

    def assert_no_uac_call_id(self, call_id: str, duration: float) -> None:
        del call_id, duration


def args_for(scenario: str, output: Path) -> argparse.Namespace:
    return argparse.Namespace(
        scenario=scenario,
        uac_bind=("127.0.0.1", 39090),
        proxy_target=("127.0.0.1", 39070),
        rvoip_target=("127.0.0.1", 39060),
        uas_listen=[
            ("primary", ("127.0.0.1", 39080)),
            ("aux1", ("127.0.0.1", 39081)),
            ("aux2", ("127.0.0.1", 39082)),
        ],
        advertised_host="127.0.0.1",
        expected_rvoip_sent_by="127.0.0.1:39060",
        expected_peer_sent_by="127.0.0.1:39070",
        output_dir=output,
        timeout=1.0,
        late_2xx_delay=0.001,
        rfc6026_accepted_window=32.0,
        stray_observation_seconds=0.01,
    )


class RawAdvancedUdpTests(unittest.TestCase):
    def test_response_preserves_via_chain_and_record_route(self) -> None:
        request = (
            b"INVITE sip:agent@example.test SIP/2.0\r\n"
            b"Via: SIP/2.0/UDP rvoip.test:5060;branch=rvoip\r\n"
            b"Via: SIP/2.0/UDP peer.test:5070;branch=peer\r\n"
            b"Via: SIP/2.0/UDP caller.test:5090;branch=caller\r\n"
            b"Record-Route: <sip:peer.test:5070;lr>\r\n"
            b"From: <sip:caller@example.test>;tag=caller\r\n"
            b"To: <sip:agent@example.test>\r\n"
            b"Call-ID: preserve@example.test\r\n"
            b"CSeq: 1 INVITE\r\n"
            b"Content-Length: 0\r\n\r\n"
        )
        response = advanced.make_response(
            request,
            200,
            "OK",
            "branch-a",
            ("127.0.0.1", 39080),
        )
        self.assertEqual(len(advanced.header_values(response, "Via")), 3)
        self.assertEqual(
            advanced.header_values(response, "Record-Route"),
            ["<sip:peer.test:5070;lr>"],
        )
        self.assertIn(b"To: <sip:agent@example.test>;tag=branch-a", response)
        self.assertIn(b"Contact: <sip:branch-a@127.0.0.1:39080", response)

    def test_stray_response_has_unmatched_rvoip_top_via(self) -> None:
        call_id, response = advanced.make_stray_response(
            "stray-response-drop",
            ("127.0.0.1", 39060),
            ("127.0.0.1", 39070),
            ("127.0.0.1", 39090),
            "abcdef",
        )
        self.assertEqual(
            advanced.via_sent_by_values(response),
            ["127.0.0.1:39060", "127.0.0.1:39070", "127.0.0.1:39090"],
        )
        self.assertEqual(advanced.first_header(response, "Call-ID"), call_id)

    def test_dialog_ack_reverses_complete_record_route_entries(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            args = args_for("multiple-2xx", Path(temporary))
            harness = FakeHarness(args)
            call = advanced.call_identity("multiple-2xx")
            response = (
                b"SIP/2.0 200 OK\r\n"
                b"Via: SIP/2.0/UDP 127.0.0.1:39090;branch=test\r\n"
                b"Record-Route: <sip:rvoip.test:5060;transport=udp;lr>;x-hop=rvoip\r\n"
                b"Record-Route: <sip:peer.test:5070;lr>;x-hop=peer\r\n"
                b"From: <sip:caller@127.0.0.1>;tag=fake-from\r\n"
                b"To: <sip:agent@example.test>;tag=branch-a\r\n"
                + f"Call-ID: {call.call_id}\r\n".encode()
                + b"CSeq: 1 INVITE\r\n"
                b"Contact: <sip:aux1@127.0.0.1:39081;transport=udp>\r\n"
                b"Content-Length: 0\r\n\r\n"
            )

            ack, _ = advanced.send_dialog_ack(harness, call, response, "aux1", 1)

            self.assertEqual(
                advanced.header_values(ack, "Route"),
                [
                    "<sip:peer.test:5070;lr>;x-hop=peer",
                    "<sip:rvoip.test:5060;transport=udp;lr>;x-hop=rvoip",
                ],
            )

    def test_all_scenario_plans_emit_pass_contracts(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            for scenario in advanced.SCENARIOS:
                with self.subTest(scenario=scenario):
                    args = args_for(scenario, output)
                    fake = FakeHarness(args)
                    with mock.patch.object(advanced, "UdpHarness", return_value=fake):
                        payload = advanced.run(args)
                    self.assertEqual(payload["schema"], advanced.SCHEMA)
                    self.assertEqual(payload["scenario"], scenario)
                    self.assertEqual(payload["status"], "PASS")
                    self.assertTrue(payload["external_peer_path_observed"])
                    if scenario in advanced.FORK_SCENARIOS:
                        self.assertEqual(
                            set(payload["branch_request_counts"]),
                            set(advanced.REQUIRED_BRANCHES),
                        )
                    else:
                        self.assertEqual(len(fake.direct_sends), 1)
                    if scenario == "sequential-fork":
                        self.assertEqual(payload["branch_order"], ["primary", "aux1"])
                        self.assertEqual(payload["unused_branches"], ["aux2"])
                    elif scenario == "multiple-2xx":
                        self.assertEqual(payload["upstream_invite_2xx"], 2)
                        self.assertEqual(payload["distinct_2xx_to_tags"], 2)
                        self.assertEqual(len(payload["dialog_ack_routes"]), 2)
                        for ack in payload["dialog_ack_routes"]:
                            self.assertEqual(
                                ack["uac_ack_route_set"],
                                list(reversed(ack["response_record_route_set"])),
                            )
                            self.assertEqual(
                                ack["downstream_ack_request_uri"],
                                ack["response_contact_uri"],
                            )
                            self.assertEqual(ack["downstream_ack_route_set"], [])
                    elif scenario == "late-2xx":
                        self.assertEqual(payload["same_dialog_upstream_2xx"], 2)
                        self.assertEqual(
                            payload["late_2xx_timing"]["phase"],
                            "rfc6026-accepted",
                        )
                        self.assertFalse(
                            payload["late_2xx_timing"][
                                "post_transaction_termination_claimed"
                            ]
                        )
                    elif scenario == "sixxx-cancel":
                        self.assertEqual(
                            payload["cancel_requests_per_branch"],
                            {"primary": 0, "aux1": 1, "aux2": 1},
                        )
                    elif scenario == "stray-response-drop":
                        self.assertEqual(payload["stray_upstream_responses"], 0)

    def test_fork_scenarios_require_three_named_endpoints(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            args = args_for("multiple-2xx", Path(temporary))
            args.uas_listen = args.uas_listen[:2]
            with self.assertRaisesRegex(ValueError, "requires UAS listeners"):
                advanced.validate_args(args)

    def test_named_address_parser_is_strict(self) -> None:
        self.assertEqual(
            advanced.parse_named_address("aux1=127.0.0.1:39081"),
            ("aux1", ("127.0.0.1", 39081)),
        )
        with self.assertRaises(argparse.ArgumentTypeError):
            advanced.parse_named_address("Aux One=127.0.0.1:39081")


if __name__ == "__main__":
    unittest.main()
