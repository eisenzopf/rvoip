#!/usr/bin/env python3
"""Focused contracts for scenario-bound packet evidence."""

from __future__ import annotations

import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

import packet_evidence


def row(
    *,
    call_id: str,
    marker: str,
    method: str = "",
    status: str = "",
    cseq_method: str | None = None,
    timestamp: float = 1.0,
    source: tuple[str, str] = ("127.0.0.1", "25090"),
    destination: tuple[str, str] = ("127.0.0.1", "25070"),
    vias: tuple[str, str] = (
        "127.0.0.1|192.0.2.10",
        "25060|25070",
    ),
    branches: str = "z9hG4bK-rvoip|z9hG4bK-peer",
    request_uri: str = "",
    routes: str = "",
    record_routes: str = "",
    contact: str = "",
    to_tag: str = "",
) -> dict[str, str]:
    return {
        "frame.number": str(round(timestamp * 1000)),
        "frame.time_epoch": str(timestamp),
        "sip.Call-ID": call_id,
        "sip.msg_hdr": f"X-Interop-Scenario: {marker}",
        "sip.Method": method,
        "sip.Status-Code": status,
        "sip.CSeq.method": cseq_method or method,
        "sip.Via.sent-by.address": vias[0],
        "sip.Via.sent-by.port": vias[1],
        "sip.Via.branch": branches,
        "sip.Request-Line": (
            f"{method} {request_uri} SIP/2.0" if method and request_uri else ""
        ),
        "sip.r-uri": request_uri,
        "sip.Route.uri": routes,
        "sip.Record-Route.uri": record_routes,
        "sip.contact.uri": contact,
        "sip.to.tag": to_tag,
        "ip.src": source[0],
        "udp.srcport": source[1],
        "ip.dst": destination[0],
        "udp.dstport": destination[1],
        "ipv6.src": "",
        "ipv6.dst": "",
        "tcp.srcport": source[1],
        "tcp.dstport": destination[1],
    }


def tcp_row(
    *,
    destination_port: int,
    frame: int = 1,
    destination_address: str = "127.0.0.1",
    syn: str = "1",
    ack: str = "0",
) -> dict[str, str]:
    return {
        "frame.number": str(frame),
        "frame.time_epoch": str(frame),
        "ip.src": "127.0.0.1",
        "ipv6.src": "",
        "tcp.srcport": "45000",
        "ip.dst": destination_address,
        "ipv6.dst": "",
        "tcp.dstport": str(destination_port),
        "tcp.flags.syn": syn,
        "tcp.flags.ack": ack,
        "tcp.flags.reset": "0",
    }


def dns_row(
    *,
    name: str,
    query_type: int,
    response: bool = False,
    priorities: str = "",
    ports: str = "",
    targets: str = "",
) -> dict[str, str]:
    source = ("127.0.0.1", "25353") if response else ("127.0.0.1", "45001")
    destination = ("127.0.0.1", "45001") if response else ("127.0.0.1", "25353")
    return {
        "frame.number": "1",
        "frame.time_epoch": "1.0",
        "ip.src": source[0],
        "ipv6.src": "",
        "udp.srcport": source[1],
        "ip.dst": destination[0],
        "ipv6.dst": "",
        "udp.dstport": destination[1],
        "dns.flags.response": "1" if response else "0",
        "dns.qry.name": name,
        "dns.qry.type": str(query_type),
        "dns.srv.priority": priorities,
        "dns.srv.port": ports,
        "dns.srv.target": targets,
        "dns.a": "",
    }


def tls_row(
    *,
    source_port: int,
    destination_port: int,
    content_type: str = "23",
    handshake_type: str = "",
    sni: str = "",
    certificate: str = "",
) -> dict[str, str]:
    return {
        "frame.number": "1",
        "frame.time_epoch": "1.0",
        "ip.src": "127.0.0.1",
        "ipv6.src": "",
        "tcp.srcport": str(source_port),
        "ip.dst": "127.0.0.1",
        "ipv6.dst": "",
        "tcp.dstport": str(destination_port),
        "tls.record.content_type": content_type,
        "tls.handshake.type": handshake_type,
        "tls.handshake.version": "0x0303",
        "tls.handshake.extensions_server_name": sni,
        "tls.handshake.certificate": certificate,
        "tls.alert_message.level": "",
        "tls.alert_message.desc": "",
    }


class PacketEvidenceTests(unittest.TestCase):
    def test_sip_field_reader_forces_decode_over_unrelated_port_heuristics(
        self,
    ) -> None:
        completed = SimpleNamespace(
            returncode=0,
            stdout='"frame.number","sip.Method"\n"1","INVITE"\n',
            stderr="",
        )
        with mock.patch.object(
            packet_evidence.subprocess, "run", return_value=completed
        ) as run:
            rows = packet_evidence.run_fields(
                "tshark",
                Path("capture.pcap"),
                "sip",
                ("frame.number", "sip.Method"),
            )
        command = run.call_args.args[0]
        self.assertIn("udp.port==1-65535,sip", command)
        self.assertIn("tcp.port==1-65535,sip", command)
        self.assertEqual(rows, [{"frame.number": "1", "sip.Method": "INVITE"}])

    def test_non_sip_field_reader_does_not_force_sip_decode(self) -> None:
        completed = SimpleNamespace(
            returncode=0,
            stdout='"frame.number"\n',
            stderr="",
        )
        with mock.patch.object(
            packet_evidence.subprocess, "run", return_value=completed
        ) as run:
            packet_evidence.run_fields(
                "tshark",
                Path("capture.pcap"),
                "dns",
                ("frame.number",),
            )
        command = run.call_args.args[0]
        self.assertNotIn("udp.port==1-65535,sip", command)
        self.assertNotIn("tcp.port==1-65535,sip", command)

    def args(self, scenario: str = "stray-response-drop") -> SimpleNamespace:
        return SimpleNamespace(
            scenario=scenario,
            order="peer-first",
            tshark="tshark",
            rvoip_address="127.0.0.1",
            rvoip_port=25060,
            peer_address="192.0.2.10",
            peer_port=25070,
            uac_port=25090,
            uas_port=25080,
            tls_uac_boundary_port=25190,
            tls_uas_boundary_port=25180,
            tls_peer_boundary_port=25170,
            expected_sips_uri="sips:probe@example.test",
            timer_c_ms=500,
            late_2xx_delay=0.75,
            rfc6026_accepted_window=32.0,
            dead_target_port=25998,
            normal_target_port=25080,
            dns_port=25353,
            live_target_port=25081,
            rfc3263_zone="failover.interop.test",
            original_target_uri="sip:routed@interop.test",
            next_hop_route_uri="sip:next-hop@127.0.0.1:25081;transport=tcp;lr",
            record_route_uri="sip:rvoip@127.0.0.1:25060;transport=tcp;lr",
            local_route_uri="sip:local@127.0.0.1:25060;transport=tcp;lr",
            expected_tls_sni=[
                "rvoip.proxy.test",
                "peer.proxy.test",
                "sipp.proxy.test",
            ],
        )

    def analyze(
        self,
        scenario: str,
        sip_rows: list[dict[str, str]],
        *,
        tcp_rows: list[dict[str, str]] | None = None,
        dns_rows: list[dict[str, str]] | None = None,
    ) -> dict[str, object]:
        by_filter = {
            "sip": sip_rows,
            "tcp": tcp_rows or [],
            "dns": dns_rows or [],
        }

        def fields(
            _binary: str,
            _pcap: Path,
            display_filter: str,
            _fields: tuple[str, ...],
        ) -> list[dict[str, str]]:
            return [dict(item) for item in by_filter[display_filter]]

        with mock.patch.object(packet_evidence, "run_fields", side_effect=fields):
            return packet_evidence.analyze_sip(
                self.args(scenario),
                [{"path": str(Path(f"{scenario}--lo0.pcap"))}],
            )

    @staticmethod
    def assertions(analysis: dict[str, object]) -> dict[str, dict[str, object]]:
        return {
            item["name"]: item  # type: ignore[index]
            for item in analysis["assertions"]  # type: ignore[index]
        }

    def test_true_stray_must_arrive_and_have_zero_rvoip_egress(self) -> None:
        rows = [
            row(
                call_id="readiness@example.test",
                marker="stray-response-drop",
                method="OPTIONS",
            ),
            row(
                call_id="readiness@example.test",
                marker="stray-response-drop",
                status="200",
                cseq_method="OPTIONS",
            ),
            row(
                call_id="true-stray-token@example.test",
                marker="stray-response-drop",
                status="200",
                cseq_method="INVITE",
                source=("127.0.0.1", "25080"),
                destination=("127.0.0.1", "25060"),
            ),
        ]
        assertions = self.assertions(self.analyze("stray-response-drop", rows))
        self.assertTrue(assertions["true-stray-arrived-at-rvoip"]["passed"])
        self.assertTrue(assertions["true-stray-had-zero-rvoip-egress"]["passed"])

    def test_forwarded_true_stray_fails_packet_contract(self) -> None:
        rows = [
            row(
                call_id="readiness@example.test",
                marker="stray-response-drop",
                method="OPTIONS",
            ),
            row(
                call_id="readiness@example.test",
                marker="stray-response-drop",
                status="200",
                cseq_method="OPTIONS",
            ),
            row(
                call_id="true-stray-token@example.test",
                marker="stray-response-drop",
                status="200",
                cseq_method="INVITE",
                source=("127.0.0.1", "25080"),
                destination=("127.0.0.1", "25060"),
            ),
            row(
                call_id="true-stray-token@example.test",
                marker="stray-response-drop",
                status="200",
                cseq_method="INVITE",
                source=("127.0.0.1", "25060"),
                destination=("192.0.2.10", "25070"),
            ),
        ]
        assertions = self.assertions(self.analyze("stray-response-drop", rows))
        self.assertFalse(assertions["true-stray-had-zero-rvoip-egress"]["passed"])

    def test_timer_c_calling_is_proven_from_packet_chronology(self) -> None:
        rows = [
            row(
                call_id="timer@example.test",
                marker="timer-c-calling",
                method="INVITE",
                timestamp=10.0,
            ),
            row(
                call_id="timer@example.test",
                marker="timer-c-calling",
                status="408",
                cseq_method="INVITE",
                timestamp=10.5,
            ),
        ]
        assertions = self.assertions(self.analyze("timer-c-calling", rows))
        self.assertTrue(assertions["timer-c-calling-invite-precedes-408"]["passed"])
        self.assertEqual(
            assertions["timer-c-calling-packet-elapsed-within-bounds"]["observed"][
                "elapsed_ms"
            ],
            500.0,
        )

    def test_timer_c_proceeding_requires_180_then_matching_cancel(self) -> None:
        rows = [
            row(
                call_id="timer@example.test",
                marker="timer-c-proceeding",
                method="INVITE",
                timestamp=10.0,
            ),
            row(
                call_id="timer@example.test",
                marker="timer-c-proceeding",
                status="180",
                cseq_method="INVITE",
                timestamp=10.1,
            ),
            row(
                call_id="timer@example.test",
                marker="timer-c-proceeding",
                method="CANCEL",
                timestamp=10.6,
                source=("127.0.0.1", "45000"),
                destination=("127.0.0.1", "25080"),
                vias=("127.0.0.1", "25060"),
                branches="z9hG4bK-rvoip",
            ),
            row(
                call_id="timer@example.test",
                marker="timer-c-proceeding",
                status="200",
                cseq_method="CANCEL",
                timestamp=10.61,
            ),
            row(
                call_id="timer@example.test",
                marker="timer-c-proceeding",
                status="487",
                cseq_method="INVITE",
                timestamp=10.62,
            ),
            row(
                call_id="timer@example.test",
                marker="timer-c-proceeding",
                method="ACK",
                timestamp=10.63,
                source=("127.0.0.1", "45000"),
                destination=("127.0.0.1", "25080"),
                vias=("127.0.0.1", "25060"),
                branches="z9hG4bK-rvoip",
            ),
        ]
        assertions = self.assertions(self.analyze("timer-c-proceeding", rows))
        self.assertTrue(assertions["timer-c-proceeding-180-precedes-cancel"]["passed"])
        self.assertTrue(
            assertions["timer-c-proceeding-cancel-reuses-invite-branch"]["passed"]
        )
        self.assertTrue(assertions["required-methods-observed"]["passed"])

    def test_timer_c_proceeding_rejects_non_generated_cancel_shapes(self) -> None:
        base_rows = [
            row(
                call_id="timer@example.test",
                marker="timer-c-proceeding",
                method="INVITE",
                timestamp=10.0,
            ),
            row(
                call_id="timer@example.test",
                marker="timer-c-proceeding",
                status="180",
                cseq_method="INVITE",
                timestamp=10.1,
            ),
        ]
        invalid_cancels = {
            "missing-rvoip-via": row(
                call_id="timer@example.test",
                marker="timer-c-proceeding",
                method="CANCEL",
                timestamp=10.6,
                destination=("127.0.0.1", "25080"),
                vias=("192.0.2.10", "25070"),
                branches="z9hG4bK-rvoip",
            ),
            "extra-peer-via": row(
                call_id="timer@example.test",
                marker="timer-c-proceeding",
                method="CANCEL",
                timestamp=10.6,
                destination=("127.0.0.1", "25080"),
                vias=("127.0.0.1|192.0.2.10", "25060|25070"),
                branches="z9hG4bK-rvoip|z9hG4bK-peer",
            ),
            "wrong-destination": row(
                call_id="timer@example.test",
                marker="timer-c-proceeding",
                method="CANCEL",
                timestamp=10.6,
                destination=("127.0.0.1", "25081"),
                vias=("127.0.0.1", "25060"),
                branches="z9hG4bK-rvoip",
            ),
        }
        for name, cancel in invalid_cancels.items():
            with self.subTest(name=name):
                assertions = self.assertions(
                    self.analyze("timer-c-proceeding", [*base_rows, cancel])
                )
                self.assertFalse(
                    assertions["timer-c-proceeding-180-precedes-cancel"]["passed"]
                )
                self.assertFalse(
                    assertions[
                        "timer-c-proceeding-cancel-reuses-invite-branch"
                    ]["passed"]
                )

    def test_transport_failure_proves_dead_syn_and_no_normal_invite(self) -> None:
        rows = [
            row(
                call_id="probe@example.test",
                marker="transport-failure",
                method="OPTIONS",
            ),
            row(
                call_id="probe@example.test",
                marker="transport-failure",
                status="200",
                cseq_method="OPTIONS",
            ),
            row(
                call_id="failure@example.test",
                marker="transport-failure",
                method="INVITE",
            ),
            row(
                call_id="failure@example.test",
                marker="transport-failure",
                status="500",
                cseq_method="INVITE",
            ),
        ]
        assertions = self.assertions(
            self.analyze(
                "transport-failure",
                rows,
                tcp_rows=[tcp_row(destination_port=25998)],
            )
        )
        self.assertTrue(
            assertions["transport-failure-dead-endpoint-syn-observed"]["passed"]
        )
        self.assertTrue(
            assertions["transport-failure-dead-endpoint-received-zero-sip"]["passed"]
        )
        self.assertTrue(
            assertions["transport-failure-failed-call-never-reached-normal-target"][
                "passed"
            ]
        )

    def test_transport_failure_rejects_normal_target_invite(self) -> None:
        rows = [
            row(
                call_id="probe@example.test",
                marker="transport-failure",
                method="OPTIONS",
            ),
            row(
                call_id="probe@example.test",
                marker="transport-failure",
                status="200",
                cseq_method="OPTIONS",
            ),
            row(
                call_id="failure@example.test",
                marker="transport-failure",
                method="INVITE",
                destination=("127.0.0.1", "25080"),
            ),
            row(
                call_id="failure@example.test",
                marker="transport-failure",
                status="500",
                cseq_method="INVITE",
            ),
        ]
        assertions = self.assertions(
            self.analyze(
                "transport-failure",
                rows,
                tcp_rows=[tcp_row(destination_port=25998)],
            )
        )
        self.assertFalse(
            assertions["transport-failure-failed-call-never-reached-normal-target"][
                "passed"
            ]
        )

    def test_transport_failure_rejects_sip_at_dead_endpoint(self) -> None:
        rows = [
            row(
                call_id="probe@example.test",
                marker="transport-failure",
                method="OPTIONS",
            ),
            row(
                call_id="probe@example.test",
                marker="transport-failure",
                status="200",
                cseq_method="OPTIONS",
            ),
            row(
                call_id="failure@example.test",
                marker="transport-failure",
                method="INVITE",
                destination=("127.0.0.1", "25998"),
            ),
            row(
                call_id="failure@example.test",
                marker="transport-failure",
                status="500",
                cseq_method="INVITE",
            ),
        ]
        assertions = self.assertions(
            self.analyze(
                "transport-failure",
                rows,
                tcp_rows=[tcp_row(destination_port=25998)],
            )
        )
        self.assertFalse(
            assertions["transport-failure-dead-endpoint-received-zero-sip"][
                "passed"
            ]
        )

    def rfc3263_dns_rows(self) -> list[dict[str, str]]:
        zone = "failover.interop.test"
        return [
            dns_row(name=f"_sip._tcp.{zone}", query_type=33),
            dns_row(
                name=f"_sip._tcp.{zone}",
                query_type=33,
                response=True,
                priorities="10|20",
                ports="25998|25081",
                targets=f"dead.{zone}|live.{zone}",
            ),
            dns_row(name=f"dead.{zone}", query_type=1),
            dns_row(name=f"live.{zone}", query_type=1),
        ]

    def test_rfc3263_proves_dns_dead_attempt_and_live_invite(self) -> None:
        rows = [
            row(
                call_id="dns@example.test",
                marker="rfc3263-failover",
                method="INVITE",
                destination=("127.0.0.1", "25081"),
            ),
            row(
                call_id="dns@example.test",
                marker="rfc3263-failover",
                status="200",
                cseq_method="INVITE",
            ),
        ]
        assertions = self.assertions(
            self.analyze(
                "rfc3263-failover",
                rows,
                tcp_rows=[tcp_row(destination_port=25998)],
                dns_rows=self.rfc3263_dns_rows(),
            )
        )
        for name in (
            "rfc3263-srv-query-observed",
            "rfc3263-exact-ordered-srv-answers",
            "rfc3263-both-candidate-a-queries-observed",
            "rfc3263-dead-candidate-syn-observed",
            "rfc3263-live-candidate-invite-observed",
            "rfc3263-dead-candidate-received-zero-invites",
        ):
            with self.subTest(name=name):
                self.assertTrue(assertions[name]["passed"])

    def test_routing_requires_exact_ruri_route_and_record_route(self) -> None:
        rows = [
            row(
                call_id="route@example.test",
                marker="route-strict",
                method="INVITE",
                request_uri="sip:routed@interop.test",
                routes="sip:next-hop@127.0.0.1:25081;transport=tcp;lr",
                record_routes=("sip:rvoip@127.0.0.1:25060;transport=tcp;lr"),
            ),
            row(
                call_id="route@example.test",
                marker="route-strict",
                status="200",
                cseq_method="INVITE",
                record_routes=("sip:rvoip@127.0.0.1:25060;transport=tcp;lr"),
            ),
        ]
        assertions = self.assertions(self.analyze("route-strict", rows))
        for name in (
            "route-strict-exact-downstream-request-uri",
            "route-strict-exact-downstream-route-set",
            "route-strict-local-route-removed",
            "route-strict-record-route-round-trip",
        ):
            with self.subTest(name=name):
                self.assertTrue(assertions[name]["passed"])

        rows[0]["sip.r-uri"] = "sip:wrong@interop.test"
        assertions = self.assertions(self.analyze("route-strict", rows))
        self.assertFalse(
            assertions["route-strict-exact-downstream-request-uri"]["passed"]
        )

    def test_capacity_evidence_partitions_forwarded_and_rejected_calls(self) -> None:
        rows: list[dict[str, str]] = []
        for index in range(2):
            call = f"held-{index}@example.test"
            rows.extend(
                (
                    row(
                        call_id=call,
                        marker="capacity-overload",
                        method="INVITE",
                    ),
                    row(
                        call_id=call,
                        marker="capacity-overload",
                        status="486",
                        cseq_method="INVITE",
                    ),
                )
            )
        rows.extend(
            (
                row(
                    call_id="overloaded@example.test",
                    marker="capacity-overload",
                    method="INVITE",
                    vias=("192.0.2.10", "25070"),
                ),
                row(
                    call_id="overloaded@example.test",
                    marker="capacity-overload",
                    status="503",
                    cseq_method="INVITE",
                    source=("127.0.0.1", "25060"),
                    destination=("192.0.2.10", "45000"),
                    vias=("127.0.0.1", "25060"),
                ),
                row(
                    call_id="overloaded@example.test",
                    marker="capacity-overload",
                    status="500",
                    cseq_method="INVITE",
                    source=("192.0.2.10", "25070"),
                    destination=("127.0.0.1", "25090"),
                    vias=("192.0.2.10", "25070"),
                ),
            )
        )
        assertions = self.assertions(self.analyze("capacity-overload", rows))
        for name in (
            "capacity-overload-single-rejected-call",
            "capacity-overload-upstream-status-for-order",
            "capacity-overload-forwarded-calls-all-finished-486",
            "capacity-overload-rejected-call-had-zero-downstream-egress",
            "capacity-overload-exact-final-call-partition",
        ):
            with self.subTest(name=name):
                self.assertTrue(assertions[name]["passed"])

        rows = [
            item
            for item in rows
            if not (
                item.get("sip.Status-Code") == "500"
                and item.get("sip.CSeq.method") == "INVITE"
            )
        ]
        assertions = self.assertions(self.analyze("capacity-overload", rows))
        self.assertFalse(
            assertions["capacity-overload-upstream-status-for-order"]["passed"]
        )

    def test_multiple_2xx_acks_follow_contact_and_reversed_routes(self) -> None:
        routes = "sip:rvoip.test:5060;lr|sip:peer.test:5070;lr"
        reversed_routes = "sip:peer.test:5070;lr|sip:rvoip.test:5060;lr"
        rows: list[dict[str, str]] = []
        for index, tag in enumerate(("dialog-a", "dialog-b")):
            identity = "multiple@example.test"
            contact = f"sip:{tag}@127.0.0.1:{25080 + index};transport=udp"
            rows.extend(
                (
                    row(
                        call_id=identity,
                        marker="multiple-2xx",
                        method="INVITE",
                    ),
                    row(
                        call_id=identity,
                        marker="multiple-2xx",
                        status="200",
                        cseq_method="INVITE",
                        to_tag=tag,
                        contact=contact,
                        record_routes=routes,
                    ),
                    row(
                        call_id=identity,
                        marker="multiple-2xx",
                        method="ACK",
                        to_tag=tag,
                        request_uri=contact,
                        routes=reversed_routes,
                        vias=("caller.test", "5090"),
                    ),
                    row(
                        call_id=identity,
                        marker="multiple-2xx",
                        method="ACK",
                        to_tag=tag,
                        request_uri=contact,
                    ),
                )
            )
        rows.append(
            row(
                call_id="multiple@example.test",
                marker="multiple-2xx",
                status="486",
                cseq_method="INVITE",
            )
        )
        assertions = self.assertions(self.analyze("multiple-2xx", rows))
        for name in (
            "multiple-2xx-two-dialog-contacts-and-record-route-sets",
            "multiple-2xx-uac-acks-use-contact-and-reversed-route-set",
            "multiple-2xx-downstream-acks-reach-contact-with-routes-consumed",
        ):
            with self.subTest(name=name):
                self.assertTrue(assertions[name]["passed"])

    def test_late_2xx_packet_timing_is_within_accepted_and_each_is_acked(
        self,
    ) -> None:
        identity = "late@example.test"
        rows = [
            row(
                call_id=identity,
                marker="late-2xx",
                method="INVITE",
                timestamp=9.0,
            ),
            row(
                call_id=identity,
                marker="late-2xx",
                status="480",
                cseq_method="INVITE",
                timestamp=9.1,
            ),
            row(
                call_id=identity,
                marker="late-2xx",
                status="486",
                cseq_method="INVITE",
                timestamp=9.2,
            ),
            row(
                call_id=identity,
                marker="late-2xx",
                status="200",
                cseq_method="INVITE",
                timestamp=10.0,
                to_tag="same-dialog",
                vias=("caller.test", "5090"),
            ),
            row(
                call_id=identity,
                marker="late-2xx",
                method="ACK",
                timestamp=10.01,
                to_tag="same-dialog",
                vias=("caller.test", "5090"),
            ),
            row(
                call_id=identity,
                marker="late-2xx",
                status="200",
                cseq_method="INVITE",
                timestamp=10.75,
                to_tag="same-dialog",
                vias=("caller.test", "5090"),
            ),
            row(
                call_id=identity,
                marker="late-2xx",
                method="ACK",
                timestamp=10.76,
                to_tag="same-dialog",
                vias=("caller.test", "5090"),
            ),
        ]
        assertions = self.assertions(self.analyze("late-2xx", rows))
        self.assertTrue(
            assertions["late-2xx-each-forwarding-event-has-dialog-ack"]["passed"]
        )
        timing = assertions["late-2xx-packet-delay-within-rfc6026-accepted-window"]
        self.assertTrue(timing["passed"])
        self.assertFalse(timing["observed"]["post_transaction_termination_claimed"])

    def test_pooled_tls_scenario_requires_application_data_not_new_handshakes(
        self,
    ) -> None:
        rows = [
            tls_row(source_port=45000, destination_port=25060),
            tls_row(source_port=45001, destination_port=25070),
            tls_row(source_port=45002, destination_port=25180),
        ]
        with mock.patch.object(
            packet_evidence, "run_fields", return_value=rows
        ):
            analysis = packet_evidence.analyze_tls(
                self.args("options-readiness"),
                [{"path": "options-readiness--lo0.pcap"}],
            )
        assertions = self.assertions(analysis)
        self.assertTrue(assertions["tls-packets-observed"]["passed"])
        self.assertTrue(
            assertions["tls-application-data-on-every-encrypted-hop"]["passed"]
        )
        self.assertTrue(
            assertions["tls-handshake-sni-valid-when-observed"]["passed"]
        )
        self.assertTrue(
            assertions[
                "tls-handshake-certificates-observed-when-initiated"
            ]["passed"]
        )
        self.assertEqual(analysis["observed_sni"], [])

    def test_tls_scenario_rejects_missing_application_data_hop(self) -> None:
        rows = [
            tls_row(source_port=45000, destination_port=25060),
            tls_row(source_port=45001, destination_port=25070),
        ]
        with mock.patch.object(
            packet_evidence, "run_fields", return_value=rows
        ):
            analysis = packet_evidence.analyze_tls(
                self.args("options-readiness"),
                [{"path": "options-readiness--lo0.pcap"}],
            )
        assertions = self.assertions(analysis)
        self.assertFalse(
            assertions["tls-application-data-on-every-encrypted-hop"]["passed"]
        )

    def test_tls_scenario_rejects_unexpected_observed_sni(self) -> None:
        rows = [
            tls_row(
                source_port=45000,
                destination_port=25060,
                content_type="22|23",
                handshake_type="1",
                sni="attacker.example",
                certificate="01:02:03:04",
            ),
            tls_row(source_port=45001, destination_port=25070),
            tls_row(source_port=45002, destination_port=25180),
        ]
        with mock.patch.object(
            packet_evidence, "run_fields", return_value=rows
        ):
            analysis = packet_evidence.analyze_tls(
                self.args("options-readiness"),
                [{"path": "options-readiness--lo0.pcap"}],
            )
        assertions = self.assertions(analysis)
        self.assertFalse(
            assertions["tls-handshake-sni-valid-when-observed"]["passed"]
        )

    def test_invite_dialog_uses_contact_and_reversed_route_set(self) -> None:
        call_id = "invite-route@example.test"
        contact = "sip:agent@127.0.0.1:25080;transport=tls"
        response_routes = (
            "sip:rvoip@127.0.0.1:25060;transport=tls;lr"
            "|sip:peer@192.0.2.10:25070;transport=tls;lr"
        )
        uac_routes = "|".join(reversed(response_routes.split("|")))
        rows = [
            row(
                call_id=call_id,
                marker="invite-success",
                method="INVITE",
            ),
            row(
                call_id=call_id,
                marker="invite-success",
                status="180",
                cseq_method="INVITE",
            ),
            row(
                call_id=call_id,
                marker="invite-success",
                status="200",
                cseq_method="INVITE",
                contact=contact,
                record_routes=response_routes,
                to_tag="dialog-a",
            ),
            *[
                row(
                    call_id=call_id,
                    marker="invite-success",
                    method=method,
                    request_uri=contact,
                    routes=uac_routes,
                    to_tag="dialog-a",
                    vias=("caller.test", "25090"),
                )
                for method in ("ACK", "BYE")
            ],
            *[
                row(
                    call_id=call_id,
                    marker="invite-success",
                    method=method,
                    request_uri=contact,
                    routes="",
                    to_tag="dialog-a",
                )
                for method in ("ACK", "BYE")
            ],
        ]
        assertions = self.assertions(self.analyze("invite-success", rows))
        for name in (
            "invite-dialog-response-contact-and-record-route-set",
            "invite-dialog-uac-ack-bye-use-contact-and-reversed-route-set",
            "invite-dialog-downstream-ack-bye-reach-contact-with-routes-consumed",
        ):
            with self.subTest(name=name):
                self.assertTrue(assertions[name]["passed"])

    def test_invite_dialog_rejects_bye_without_route_set(self) -> None:
        call_id = "invite-route-negative@example.test"
        contact = "sip:agent@127.0.0.1:25080;transport=tls"
        route = "sip:peer@192.0.2.10:25070;transport=tls;lr"
        rows = [
            row(
                call_id=call_id,
                marker="invite-success",
                method="INVITE",
            ),
            row(
                call_id=call_id,
                marker="invite-success",
                status="180",
                cseq_method="INVITE",
            ),
            row(
                call_id=call_id,
                marker="invite-success",
                status="200",
                cseq_method="INVITE",
                contact=contact,
                record_routes=route,
                to_tag="dialog-a",
            ),
            row(
                call_id=call_id,
                marker="invite-success",
                method="ACK",
                request_uri=contact,
                routes=route,
                to_tag="dialog-a",
                vias=("caller.test", "25090"),
            ),
            row(
                call_id=call_id,
                marker="invite-success",
                method="BYE",
                request_uri=contact,
                routes="",
                to_tag="dialog-a",
                vias=("caller.test", "25090"),
            ),
            *[
                row(
                    call_id=call_id,
                    marker="invite-success",
                    method=method,
                    request_uri=contact,
                    to_tag="dialog-a",
                )
                for method in ("ACK", "BYE")
            ],
        ]
        assertions = self.assertions(self.analyze("invite-success", rows))
        self.assertFalse(
            assertions[
                "invite-dialog-uac-ack-bye-use-contact-and-reversed-route-set"
            ]["passed"]
        )

    def test_sips_routing_combines_plaintext_uri_and_tls_proof(self) -> None:
        rows = [
            row(
                call_id="sips@example.test",
                marker="sips-routing",
                method="OPTIONS",
                request_uri="sips:probe@example.test",
                source=("127.0.0.1", "25090"),
                destination=("127.0.0.1", "25190"),
                vias=("127.0.0.1", "25090"),
            ),
            row(
                call_id="sips@example.test",
                marker="sips-routing",
                method="OPTIONS",
                request_uri="sips:probe@example.test",
                source=("127.0.0.1", "46000"),
                destination=("127.0.0.1", "25080"),
            ),
            row(
                call_id="sips@example.test",
                marker="sips-routing",
                status="200",
                cseq_method="OPTIONS",
                source=("127.0.0.1", "25190"),
                destination=("127.0.0.1", "25090"),
            ),
        ]
        tls = {
            "selected_packet_count": 12,
            "observed_sni": [
                "peer.proxy.test",
                "rvoip.proxy.test",
                "sipp.proxy.test",
            ],
            "observed_handshake_types": ["1", "2"],
            "observed_certificate_sha256": ["7" * 64],
            "observed_tls_application_listener_ports": [
                "25060",
                "25070",
                "25180",
            ],
            "observed_alerts": [],
            "assertions": [
                {
                    "name": "tls-packets-observed",
                    "passed": True,
                    "observed": 12,
                },
                {
                    "name": "tls-application-data-on-every-encrypted-hop",
                    "passed": True,
                    "observed": ["25060", "25070", "25180"],
                },
                {
                    "name": "tls-handshake-sni-valid-when-observed",
                    "passed": True,
                    "observed": {
                        "required": ["peer.proxy.test"],
                        "observed": [
                            "peer.proxy.test",
                            "rvoip.proxy.test",
                            "sipp.proxy.test",
                        ],
                    },
                },
                {
                    "name": "tls-handshake-certificates-observed-when-initiated",
                    "passed": True,
                    "observed": ["7" * 64],
                },
            ],
        }
        with (
            mock.patch.object(packet_evidence, "collect_fields", return_value=rows),
            mock.patch.object(packet_evidence, "analyze_tls", return_value=tls),
        ):
            analysis = packet_evidence.analyze_sips_routing(
                self.args("sips-routing"),
                [{"path": "sips-routing--lo0.pcap"}],
            )
        assertions = self.assertions(analysis)
        for name in (
            "sips-request-uri-at-uac-boundary",
            "sips-request-uri-at-uas-boundary",
            "sips-request-preserved-end-to-end",
            "sips-both-proxy-vias-observed",
            "no-plaintext-sip-on-external-tls-ports",
            "sips-options-success-observed",
        ):
            with self.subTest(name=name):
                self.assertTrue(assertions[name]["passed"])

    def test_sips_routing_rejects_plaintext_on_external_tls_port(self) -> None:
        insecure = row(
            call_id="sips@example.test",
            marker="sips-routing",
            method="OPTIONS",
            request_uri="sips:probe@example.test",
            source=("127.0.0.1", "25090"),
            destination=("127.0.0.1", "25060"),
        )
        tls = {
            "selected_packet_count": 1,
            "observed_sni": ["rvoip.proxy.test"],
            "observed_handshake_types": ["1"],
            "observed_certificate_sha256": ["7" * 64],
            "observed_tls_application_listener_ports": ["25060"],
            "observed_alerts": [],
            "assertions": [],
        }
        with (
            mock.patch.object(
                packet_evidence, "collect_fields", return_value=[insecure]
            ),
            mock.patch.object(packet_evidence, "analyze_tls", return_value=tls),
        ):
            analysis = packet_evidence.analyze_sips_routing(
                self.args("sips-routing"),
                [{"path": "sips-routing--lo0.pcap"}],
            )
        assertions = self.assertions(analysis)
        self.assertFalse(assertions["no-plaintext-sip-on-external-tls-ports"]["passed"])


if __name__ == "__main__":
    unittest.main()
