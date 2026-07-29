#!/usr/bin/env python3
"""Focused contract tests for bounded advanced TCP interop drivers."""

from __future__ import annotations

import argparse
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

import raw_tcp_advanced_support as support
import raw_tcp_routing_auth as routing_auth
import raw_tcp_timer_failure as timer_failure


def downstream_request(
    *,
    request_uri: str = "sip:agent@destination.invalid;transport=tcp",
    routes: tuple[str, ...] = (),
    record_routes: tuple[str, ...] = (),
) -> bytes:
    lines = [
        f"INVITE {request_uri} SIP/2.0",
        "Via: SIP/2.0/TCP rvoip.test:5060;branch=rvoip",
        "Via: SIP/2.0/TCP peer.test:5070;branch=peer",
        "Via: SIP/2.0/TCP caller.test:5090;branch=caller",
        "From: <sip:caller@example.test>;tag=caller",
        "To: <sip:agent@example.test>",
        "Call-ID: routing@example.test",
        "CSeq: 1 INVITE",
    ]
    lines.extend(f"Route: {value}" for value in routes)
    lines.extend(f"Record-Route: {value}" for value in record_routes)
    lines.extend(("Content-Length: 0", "", ""))
    return "\r\n".join(lines).encode("ascii")


class RawTcpAdvancedTests(unittest.TestCase):
    def test_external_vias_require_both_proxies_in_wire_order(self) -> None:
        request = downstream_request()
        observed = support.require_external_vias(
            request,
            "rvoip.test:5060",
            "peer.test:5070",
            "peer-first",
        )
        self.assertEqual(
            observed["observed_proxy_via_order"],
            ["rvoip.test:5060", "peer.test:5070"],
        )
        with self.assertRaisesRegex(RuntimeError, "Via order"):
            support.require_external_vias(
                request,
                "rvoip.test:5060",
                "peer.test:5070",
                "rvoip-first",
            )

    def test_external_vias_reject_missing_and_duplicate_hops(self) -> None:
        missing = downstream_request().replace(
            b"Via: SIP/2.0/TCP peer.test:5070;branch=peer\r\n",
            b"",
        )
        with self.assertRaisesRegex(RuntimeError, "exactly one Via"):
            support.require_external_vias(
                missing,
                "rvoip.test:5060",
                "peer.test:5070",
                "peer-first",
            )
        duplicate = downstream_request().replace(
            b"Via: SIP/2.0/TCP peer.test:5070;branch=peer\r\n",
            (
                b"Via: SIP/2.0/TCP peer.test:5070;branch=peer\r\n"
                b"Via: SIP/2.0/TCP peer.test:5070;branch=peer-two\r\n"
            ),
        )
        with self.assertRaisesRegex(RuntimeError, "exactly one Via"):
            support.require_external_vias(
                duplicate,
                "rvoip.test:5060",
                "peer.test:5070",
                "peer-first",
            )

    def test_route_list_splitter_preserves_quoted_commas(self) -> None:
        message = downstream_request(
            routes=(
                '"Proxy, One" <sip:one.example;lr>, <sip:two.example;lr>',
            )
        )
        self.assertEqual(
            routing_auth.split_header_entries(message, "Route"),
            [
                '"Proxy, One" <sip:one.example;lr>',
                "<sip:two.example;lr>",
            ],
        )

    def test_routing_contract_checks_uri_route_and_record_route(self) -> None:
        args = SimpleNamespace(
            original_target_uri=(
                "sip:agent@destination.invalid;transport=tcp"
            ),
            next_hop_route_uri=(
                "sip:next-hop.invalid;transport=tcp;lr"
            ),
            rvoip_local_uri="sip:rvoip.invalid;transport=tcp;lr",
            expected_record_route_uri=(
                "sip:rvoip.invalid;transport=tcp;lr"
            ),
        )
        downstream = downstream_request(
            routes=(
                "<sip:next-hop.invalid;transport=tcp;lr>",
            ),
            record_routes=(
                "<sip:rvoip.invalid;transport=tcp;lr>",
                "<sip:peer.invalid;transport=tcp;lr>",
            ),
        )
        upstream = support.make_response(downstream, 200, "OK")
        observations = routing_auth.routing_observations(
            args,
            downstream,
            upstream,
        )
        self.assertTrue(observations["record_route_round_trip"])
        self.assertTrue(observations["local_route_removed"])

    def test_routing_contract_rejects_unremoved_local_route(self) -> None:
        args = SimpleNamespace(
            original_target_uri=(
                "sip:agent@destination.invalid;transport=tcp"
            ),
            next_hop_route_uri=(
                "sip:next-hop.invalid;transport=tcp;lr"
            ),
            rvoip_local_uri="sip:rvoip.invalid;transport=tcp;lr",
            expected_record_route_uri=(
                "sip:rvoip.invalid;transport=tcp;lr"
            ),
        )
        downstream = downstream_request(
            routes=(
                "<sip:rvoip.invalid;transport=tcp;lr>, "
                "<sip:next-hop.invalid;transport=tcp;lr>",
            ),
            record_routes=(
                "<sip:rvoip.invalid;transport=tcp;lr>",
            ),
        )
        upstream = support.make_response(downstream, 200, "OK")
        with self.assertRaisesRegex(RuntimeError, "unexpectedly retained"):
            routing_auth.routing_observations(
                args,
                downstream,
                upstream,
            )

    def test_mixed_authentication_aggregation_is_exact(self) -> None:
        request = downstream_request()
        response = support.make_response(
            request,
            401,
            "Unauthorized",
            extra_headers=(
                *routing_auth.challenge_headers(
                    "WWW-Authenticate",
                    routing_auth.AUTH_WWW_CHALLENGES,
                ),
                *routing_auth.challenge_headers(
                    "Proxy-Authenticate",
                    routing_auth.AUTH_PROXY_CHALLENGES,
                ),
            ),
        )
        observed = routing_auth.require_auth_aggregation(response)
        self.assertEqual(observed["www_authenticate_count"], 3)
        self.assertEqual(observed["proxy_authenticate_count"], 2)
        with self.assertRaisesRegex(
            RuntimeError,
            "Proxy-Authenticate",
        ):
            routing_auth.require_auth_aggregation(
                response.replace(
                    (
                        "Proxy-Authenticate: "
                        f"{routing_auth.AUTH_PROXY_CHALLENGES[-1]}\r\n"
                    ).encode("ascii"),
                    b"",
                )
            )

    def test_strict_local_uri_removes_only_lr_parameter(self) -> None:
        self.assertEqual(
            routing_auth.remove_lr_parameter(
                "sip:rvoip.invalid:5060;transport=tcp;lr"
            ),
            "sip:rvoip.invalid:5060;transport=tcp",
        )
        self.assertEqual(
            routing_auth.remove_lr_parameter(
                "sip:rvoip.invalid:5060;lr;transport=tcp"
            ),
            "sip:rvoip.invalid:5060;transport=tcp",
        )

    def test_loopback_tcp_primitives_emit_bounded_artifacts(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            budget = support.TraceBudget(output)
            with support.TcpEndpoint(
                "primary",
                ("127.0.0.1", 0),
                1.0,
                budget,
            ) as endpoint:
                target = endpoint.listener.getsockname()
                with support.TcpUac(
                    target,
                    ("127.0.0.1", 0),
                    1.0,
                    budget,
                ) as uac:
                    request = support.make_request(
                        "OPTIONS",
                        "transport-failure",
                        "127.0.0.1",
                        uac.socket.getsockname()[1],
                    )
                    call_id = support.first_header(request, "Call-ID")
                    uac.send(request)
                    received = endpoint.receive()
                    endpoint.send(
                        received,
                        support.make_response(
                            received.message,
                            200,
                            "OK",
                        ),
                    )
                    uac.wait_response(
                        200,
                        "OPTIONS",
                        call_id=call_id,
                    )
            payload = support.finalize_payload(
                output,
                "transport-failure",
                {"loopback": True},
                budget,
            )
            support.write_payload(output, payload)
            self.assertTrue(payload["external_peer_exercised"])
            self.assertEqual(payload["trace_messages"], 4)
            self.assertLessEqual(
                max(payload["trace_bytes"].values()),
                support.MAX_TRACE_BYTES,
            )

    def test_scenario_argument_validation_is_bounded(self) -> None:
        args = argparse.Namespace(
            timeout=1.0,
            timer_c_ms=500,
            quiet_window_ms=250,
            expected_failure_status=500,
            expected_overload_status=503,
            capacity_fill_limit=72,
        )
        timer_failure.validate_args(args)
        args.expected_overload_status = 200
        with self.assertRaisesRegex(ValueError, "400..699"):
            timer_failure.validate_args(args)

    def test_overload_upstream_status_defaults_follow_proxy_order(self) -> None:
        for order, expected in (("peer-first", 500), ("rvoip-first", 503)):
            args = argparse.Namespace(
                expected_overload_status=None,
                order=order,
            )
            self.assertEqual(timer_failure.expected_overload_status(args), expected)

    def test_capacity_release_sends_every_final_before_waiting(self) -> None:
        requests = [
            support.make_request(
                "INVITE",
                "capacity-overload",
                "127.0.0.1",
                45000,
                call_id=f"held-{index}@example.test",
            )
            for index in range(2)
        ]
        held = [
            (
                support.first_header(request, "Call-ID"),
                SimpleNamespace(message=request),
            )
            for request in requests
        ]

        class Primary:
            def __init__(self) -> None:
                self.sent: list[bytes] = []

            def send(self, _received: object, response: bytes) -> None:
                self.sent.append(response)

        primary = Primary()
        finals = [
            support.make_response(request, 486, "Busy Here")
            for request in reversed(requests)
        ]

        class Uac:
            def wait_final_response(self, method: str) -> bytes:
                self.assert_all_sent(method)
                return finals.pop(0)

            def assert_all_sent(self, method: str) -> None:
                self_outer.assertEqual(method, "INVITE")
                self_outer.assertEqual(len(primary.sent), len(held))

        self_outer = self
        released = timer_failure.release_held_calls(primary, Uac(), held)
        self.assertEqual(released, 2)
        self.assertEqual(finals, [])

    def test_tcp_uac_bind_accepts_explicit_ephemeral_port(self) -> None:
        self.assertEqual(
            support.parse_bind_address("127.0.0.1:0"),
            ("127.0.0.1", 0),
        )
        with self.assertRaises(argparse.ArgumentTypeError):
            support.parse_address("127.0.0.1:0")


if __name__ == "__main__":
    unittest.main()
