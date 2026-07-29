#!/usr/bin/env python3
"""Focused tests for external SIP trace evidence parsing."""

from __future__ import annotations

import argparse
import hashlib
import json
import tempfile
import unittest
from pathlib import Path

import raw_cancel_retransmission
import scenario_evidence


RVOIP = "rvoip.test:5060"
PEER = "peer.test:5070"


def request(
    method: str,
    scenario: str,
    identity: str,
    *,
    external_path: bool,
    phase: str | None = None,
    request_uri: str = "sip:agent@example.test",
    to_tag: str | None = None,
    headers: tuple[str, ...] = (),
) -> str:
    vias = (
        [
            f"Via: SIP/2.0/UDP {RVOIP};branch=z9hG4bK-rvoip",
            f"Via: SIP/2.0/UDP {PEER};branch=z9hG4bK-peer",
        ]
        if external_path
        else []
    )
    to_value = "<sip:agent@example.test>"
    if to_tag is not None:
        to_value += f";tag={to_tag}"
    lines = [
        f"{method} {request_uri} SIP/2.0",
        *vias,
        "Via: SIP/2.0/UDP caller.test:5090;branch=z9hG4bK-caller",
        "From: <sip:caller@example.test>;tag=caller",
        f"To: {to_value}",
        f"Call-ID: {identity}",
        f"CSeq: 1 {method}",
        f"X-Interop-Scenario: {scenario}",
        *headers,
    ]
    if phase is not None:
        lines.append(f"X-Interop-Phase: {phase}")
    lines.extend(("Content-Length: 0", "", ""))
    return "\r\n".join(lines)


def response(
    status: int,
    method: str,
    identity: str,
    *,
    tag: str = "uas",
    headers: tuple[str, ...] = (),
    scenario_marker: str | None = None,
) -> str:
    lines = [
        f"SIP/2.0 {status} Result",
        "Via: SIP/2.0/UDP caller.test:5090;branch=z9hG4bK-caller",
        "From: <sip:caller@example.test>;tag=caller",
        f"To: <sip:agent@example.test>;tag={tag}",
        f"Call-ID: {identity}",
        f"CSeq: 1 {method}",
        *headers,
    ]
    if scenario_marker is not None:
        lines.append(f"X-Interop-Scenario: {scenario_marker}")
    lines.extend(("Content-Length: 0", "", ""))
    return "\r\n".join(lines)


def message(
    direction: str, payload: str, transport: str = "udp"
) -> scenario_evidence.SipMessage:
    return scenario_evidence.SipMessage(direction, transport, payload)


def write_trace(path: Path, messages: list[scenario_evidence.SipMessage]) -> None:
    with path.open("w", encoding="iso-8859-1", newline="") as stream:
        for item in messages:
            wire = item.payload.encode("iso-8859-1")
            stream.write(
                f"{item.transport.upper()} message {item.direction} "
                f"[{len(wire)}] bytes:\n\n"
            )
            stream.write(item.payload)
            stream.write("\n--------------------\n")


def valid_packet(scenario: str, transport: str) -> dict[str, object]:
    methods, statuses = scenario_evidence.ADVANCED_PACKET_REQUIREMENTS[scenario]
    observed_statuses = set(statuses)
    if scenario == "transport-failure":
        observed_statuses.add(503)
    return {
        "schema": scenario_evidence.PACKET_SCHEMA,
        "scenario": scenario,
        "status": "PASS",
        "peer": "kamailio",
        "order": "peer-first",
        "transport": transport,
        "analyzer": {"tshark": "TShark 4.6.6", "libpcap": "libpcap 1.10"},
        "captures": [
            {
                "filename": f"{scenario}.pcap",
                "sha256": "a" * 64,
                "bytes": 1024,
            }
        ],
        "selected_call_ids": [f"{scenario}@example.test"],
        "selected_packet_count": 4,
        "observed_methods": sorted(methods),
        "observed_statuses": sorted(observed_statuses),
        "via_sent_by_addresses": ["peer.test", "rvoip.test"],
        "via_sent_by_ports": ["5060", "5070"],
        "assertions": [
            {"name": name, "passed": True, "observed": "fixture"}
            for name in (
                "scenario-call-id-observed",
                "required-methods-observed",
                "required-statuses-observed",
                "rvoip-via-observed",
                "peer-via-observed",
                *sorted(
                    set()
                    if scenario == "stray-response-drop"
                    else scenario_evidence.SCENARIO_PACKET_ASSERTIONS.get(
                        scenario, set()
                    ),
                ),
            )
        ],
    }


def tls_sips_fixture(
    root: Path,
) -> tuple[argparse.Namespace, Path, Path]:
    row = root / "row"
    directory = row / "scenarios/sips-routing"
    directory.mkdir(parents=True)
    (row / "tls-verifier-result.json").write_text(
        json.dumps(
            {
                "schema": "rvoip-sip-proxy-interop-tls-evidence-v2",
                "result": "PASS",
                "peer": "kamailio",
                "order": "rvoip-first",
                "transport": "tls",
            }
        )
    )
    for name in (
        "rvoip.log",
        "peer.log",
        "tls-rvoip-to-peer-positive.log",
        "tls-peer-to-rvoip-positive.log",
        "tls-rvoip-to-peer-wrong-name.log",
        "tls-rvoip-to-peer-wrong-ca.log",
        "tls-peer-to-rvoip-wrong-name.log",
        "tls-peer-to-rvoip-wrong-ca.log",
        "tls-peer-rejects-untrusted-client.log",
        "tls-rvoip-rejects-untrusted-client.log",
        "tls-boundary-client.log",
        "tls-boundary-server.log",
        "tls-kamailio-outbound-boundary.log",
        "peer-runtime.json",
    ):
        (row / name).write_text(f"fixture {name}\n")
    request_wire = "\r\n".join(
        (
            "OPTIONS sips:probe@example.test SIP/2.0",
            "Via: SIP/2.0/TCP caller.test:5090;branch=caller",
            "From: <sips:uac@caller.test;transport=tls>;tag=caller",
            "To: <sips:probe@example.test>",
            "Call-ID: sips-routing@example.test",
            "CSeq: 1 OPTIONS",
            "Contact: <sips:uac@caller.test;transport=tls>",
            "X-Interop-Scenario: sips-routing",
            "Content-Length: 0",
            "",
            "",
        )
    )
    downstream_wire = request_wire.replace(
        "Via: SIP/2.0/TCP caller.test:5090;branch=caller",
        "Via: SIP/2.0/TCP peer.test:5070;branch=peer\r\n"
        "Via: SIP/2.0/TCP rvoip.test:5060;branch=rvoip\r\n"
        "Via: SIP/2.0/TCP caller.test:5090;branch=caller",
    )
    response_wire = "\r\n".join(
        (
            "SIP/2.0 200 OK",
            "Via: SIP/2.0/TCP caller.test:5090;branch=caller",
            "From: <sips:uac@caller.test;transport=tls>;tag=caller",
            "To: <sips:probe@example.test>;tag=uas",
            "Call-ID: sips-routing@example.test",
            "CSeq: 1 OPTIONS",
            "Contact: <sips:probe@uas.test;transport=tls>",
            "Content-Length: 0",
            "",
            "",
        )
    )
    write_trace(
        directory / "uac-messages.log",
        [
            message("sent", request_wire, "tcp"),
            message("received", response_wire, "tcp"),
        ],
    )
    write_trace(
        directory / "uas-messages.log",
        [
            message("received", downstream_wire, "tcp"),
            message("sent", response_wire, "tcp"),
        ],
    )
    (directory / "uac-stats.csv").write_text("SuccessfulCall(C);1\n")
    (directory / "uas-stats.csv").write_text("SuccessfulCall(C);1\n")
    packet_assertion_names = {
        "scenario-call-id-observed",
        "required-methods-observed",
        "required-statuses-observed",
        "rvoip-via-observed",
        "peer-via-observed",
        "tls-packets-observed",
        "tls-application-data-on-every-encrypted-hop",
        "tls-handshake-sni-valid-when-observed",
        "tls-handshake-certificates-observed-when-initiated",
    } | scenario_evidence.SCENARIO_PACKET_ASSERTIONS["sips-routing"]
    (directory / "packet-evidence.json").write_text(
        json.dumps(
            {
                "schema": scenario_evidence.PACKET_SCHEMA,
                "scenario": "sips-routing",
                "status": "PASS",
                "peer": "kamailio",
                "order": "rvoip-first",
                "transport": "tls",
                "analyzer": {
                    "tshark": "TShark fixture",
                    "libpcap": "libpcap fixture",
                },
                "captures": [
                    {
                        "filename": "sips-routing--lo0.pcap",
                        "sha256": "a" * 64,
                        "bytes": 128,
                    }
                ],
                "selected_call_ids": ["sips-routing@example.test"],
                "selected_packet_count": 12,
                "observed_methods": ["OPTIONS"],
                "observed_statuses": [200],
                "via_sent_by_addresses": ["rvoip.test", "peer.test"],
                "via_sent_by_ports": ["5060", "5070"],
                "insecure_external_sip_packet_count": 0,
                "assertions": [
                    {
                        "name": name,
                        "passed": True,
                        "observed": (
                            0
                            if name
                            == "no-plaintext-sip-on-external-tls-ports"
                            else "fixture"
                        ),
                    }
                    for name in sorted(packet_assertion_names)
                ],
            },
            indent=2,
            sort_keys=True,
        )
        + "\n"
    )
    return (
        argparse.Namespace(
            directory=directory,
            row_directory=row,
            peer="kamailio",
            order="rvoip-first",
            expected_rvoip_sent_by=RVOIP,
            expected_peer_sent_by=PEER,
        ),
        row,
        directory,
    )


def branch_request(
    method: str, scenario: str, identity: str
) -> scenario_evidence.SipMessage:
    return message("received", request(method, scenario, identity, external_path=True))


def branch_ack(
    scenario: str,
    identity: str,
    *,
    request_uri: str = "sip:agent@example.test",
    to_tag: str | None = None,
) -> scenario_evidence.SipMessage:
    return message(
        "received",
        request(
            "ACK",
            scenario,
            identity,
            external_path=True,
            request_uri=request_uri,
            to_tag=to_tag,
        ),
    )


def branch_response(
    status: int,
    method: str,
    identity: str,
    *,
    tag: str = "uas",
    headers: tuple[str, ...] = (),
) -> scenario_evidence.SipMessage:
    return message("sent", response(status, method, identity, tag=tag, headers=headers))


def udp_fixture(
    directory: Path, scenario: str
) -> tuple[argparse.Namespace, dict[str, object]]:
    identity = f"{scenario}@example.test"
    upstream_request_method = (
        "OPTIONS" if scenario == "stray-response-drop" else "INVITE"
    )
    uac = [
        message(
            "sent",
            request(
                upstream_request_method,
                scenario,
                identity,
                external_path=False,
            ),
        )
    ]
    branches: dict[str, list[scenario_evidence.SipMessage]] = {
        "primary": [],
        "aux1": [],
        "aux2": [],
    }
    raw: dict[str, object] = {
        "schema": scenario_evidence.UDP_RAW_SCHEMA,
        "scenario": scenario,
        "status": "PASS",
        "transport": "udp",
        "external_peer_path_observed": True,
    }

    if scenario == "sequential-fork":
        branches["primary"] = [
            branch_request("INVITE", scenario, identity),
            branch_response(486, "INVITE", identity, tag="primary"),
            branch_ack(scenario, identity),
        ]
        branches["aux1"] = [
            branch_request("INVITE", scenario, identity),
            branch_response(180, "INVITE", identity, tag="aux1"),
            branch_response(200, "INVITE", identity, tag="aux1"),
            branch_ack(scenario, identity),
        ]
        uac.extend(
            (
                message("received", response(180, "INVITE", identity, tag="aux1")),
                message("received", response(200, "INVITE", identity, tag="aux1")),
                message(
                    "sent", request("ACK", scenario, identity, external_path=False)
                ),
            )
        )
        raw.update(
            branch_order=["primary", "aux1"],
            branch_finals={"primary": 486, "aux1": 200},
            unused_branches=["aux2"],
            upstream_final=200,
        )
    elif scenario == "parallel-fork":
        finals = {"primary": 486, "aux1": 480, "aux2": 200}
        for branch, status in finals.items():
            branches[branch] = [
                branch_request("INVITE", scenario, identity),
                *(
                    [branch_response(180, "INVITE", identity, tag=branch)]
                    if branch == "aux2"
                    else []
                ),
                branch_response(status, "INVITE", identity, tag=branch),
                branch_ack(scenario, identity),
            ]
        uac.extend(
            (
                message("received", response(180, "INVITE", identity, tag="aux2")),
                message("received", response(200, "INVITE", identity, tag="aux2")),
                message(
                    "sent", request("ACK", scenario, identity, external_path=False)
                ),
            )
        )
        raw.update(
            parallel_invite_branches=["primary", "aux1", "aux2"],
            branch_finals=finals,
            upstream_final=200,
        )
    elif scenario == "multiple-2xx":
        record_routes = (
            "Record-Route: <sip:rvoip.test:5060;transport=udp;lr>",
            "Record-Route: <sip:peer.test:5070;transport=udp;lr>",
        )
        ack_routes = (
            "Route: <sip:peer.test:5070;transport=udp;lr>",
            "Route: <sip:rvoip.test:5060;transport=udp;lr>",
        )
        dialog_ack_routes = []
        for branch, status in {"primary": 200, "aux1": 200, "aux2": 486}.items():
            contact = f"sip:{branch}@127.0.0.1:{39080 if branch == 'primary' else 39081};transport=udp"
            response_headers = (
                (*record_routes, f"Contact: <{contact}>") if status == 200 else ()
            )
            branches[branch] = [
                branch_request("INVITE", scenario, identity),
                branch_response(
                    status,
                    "INVITE",
                    identity,
                    tag=branch,
                    headers=response_headers,
                ),
                branch_ack(
                    scenario,
                    identity,
                    request_uri=contact if status == 200 else "sip:agent@example.test",
                    to_tag=branch,
                ),
            ]
            if status == 200:
                dialog_ack_routes.append(
                    {
                        "branch": branch,
                        "response_contact_uri": contact,
                        "response_record_route_set": [
                            "sip:rvoip.test:5060;transport=udp;lr",
                            "sip:peer.test:5070;transport=udp;lr",
                        ],
                        "uac_ack_request_uri": contact,
                        "uac_ack_route_set": [
                            "sip:peer.test:5070;transport=udp;lr",
                            "sip:rvoip.test:5060;transport=udp;lr",
                        ],
                        "downstream_ack_request_uri": contact,
                        "downstream_ack_route_set": [],
                        "route_values_consumed_by_proxies": True,
                    }
                )
        uac.extend(
            (
                message(
                    "received",
                    response(
                        200,
                        "INVITE",
                        identity,
                        tag="primary",
                        headers=(
                            *record_routes,
                            "Contact: <sip:primary@127.0.0.1:39080;transport=udp>",
                        ),
                    ),
                ),
                message(
                    "sent",
                    request(
                        "ACK",
                        scenario,
                        identity,
                        external_path=False,
                        request_uri=("sip:primary@127.0.0.1:39080;transport=udp"),
                        to_tag="primary",
                        headers=ack_routes,
                    ),
                ),
                message(
                    "received",
                    response(
                        200,
                        "INVITE",
                        identity,
                        tag="aux1",
                        headers=(
                            *record_routes,
                            "Contact: <sip:aux1@127.0.0.1:39081;transport=udp>",
                        ),
                    ),
                ),
                message(
                    "sent",
                    request(
                        "ACK",
                        scenario,
                        identity,
                        external_path=False,
                        request_uri="sip:aux1@127.0.0.1:39081;transport=udp",
                        to_tag="aux1",
                        headers=ack_routes,
                    ),
                ),
            )
        )
        raw.update(
            distinct_2xx_to_tags=2,
            upstream_invite_2xx=2,
            acked_2xx_branches=["primary", "aux1"],
            dialog_ack_routes=dialog_ack_routes,
            failure_branches={"aux2": 486},
        )
    elif scenario == "late-2xx":
        contact = "sip:primary@127.0.0.1:39080;transport=udp"
        record_routes = (
            "Record-Route: <sip:rvoip.test:5060;transport=udp;lr>",
            "Record-Route: <sip:peer.test:5070;transport=udp;lr>",
        )
        response_headers = (*record_routes, f"Contact: <{contact}>")
        ack_route_headers = (
            "Route: <sip:peer.test:5070;transport=udp;lr>",
            "Route: <sip:rvoip.test:5060;transport=udp;lr>",
        )
        branches["primary"] = [
            branch_request("INVITE", scenario, identity),
            branch_response(
                200,
                "INVITE",
                identity,
                tag="primary",
                headers=response_headers,
            ),
            branch_ack(
                scenario,
                identity,
                request_uri=contact,
                to_tag="primary",
            ),
            branch_response(
                200,
                "INVITE",
                identity,
                tag="primary",
                headers=response_headers,
            ),
            branch_ack(
                scenario,
                identity,
                request_uri=contact,
                to_tag="primary",
            ),
        ]
        for branch, status in {"aux1": 480, "aux2": 486}.items():
            branches[branch] = [
                branch_request("INVITE", scenario, identity),
                branch_response(status, "INVITE", identity, tag=branch),
                branch_ack(scenario, identity),
            ]
        uac.extend(
            (
                message(
                    "received",
                    response(
                        200,
                        "INVITE",
                        identity,
                        tag="primary",
                        headers=response_headers,
                    ),
                ),
                message(
                    "sent",
                    request(
                        "ACK",
                        scenario,
                        identity,
                        external_path=False,
                        request_uri=contact,
                        to_tag="primary",
                        headers=ack_route_headers,
                    ),
                ),
                message(
                    "received",
                    response(
                        200,
                        "INVITE",
                        identity,
                        tag="primary",
                        headers=response_headers,
                    ),
                ),
                message(
                    "sent",
                    request(
                        "ACK",
                        scenario,
                        identity,
                        external_path=False,
                        request_uri=contact,
                        to_tag="primary",
                        headers=ack_route_headers,
                    ),
                ),
            )
        )
        late_ack_route = {
            "branch": "primary",
            "response_contact_uri": contact,
            "response_record_route_set": [
                "sip:rvoip.test:5060;transport=udp;lr",
                "sip:peer.test:5070;transport=udp;lr",
            ],
            "uac_ack_request_uri": contact,
            "uac_ack_route_set": [
                "sip:peer.test:5070;transport=udp;lr",
                "sip:rvoip.test:5060;transport=udp;lr",
            ],
            "downstream_ack_request_uri": contact,
            "downstream_ack_route_set": [],
            "route_values_consumed_by_proxies": True,
        }
        raw.update(
            late_2xx_delay_seconds=0.75,
            late_2xx_timing={
                "phase": "rfc6026-accepted",
                "requested_delay_seconds": 0.75,
                "observed_forwarding_interval_seconds": 0.752,
                "first_forward_latency_seconds": 0.002,
                "second_forward_latency_seconds": 0.002,
                "accepted_window_seconds": 32.0,
                "within_accepted_window": True,
                "post_transaction_termination_claimed": False,
            },
            same_dialog_upstream_2xx=2,
            acked_2xx_branches=["primary", "primary"],
            late_dialog_ack_routes=[late_ack_route, late_ack_route],
            failure_branches={"aux1": 480, "aux2": 486},
        )
    elif scenario == "sixxx-cancel":
        branches["primary"] = [
            branch_request("INVITE", scenario, identity),
            branch_response(180, "INVITE", identity, tag="primary"),
            branch_response(603, "INVITE", identity, tag="primary"),
            branch_ack(scenario, identity),
        ]
        for branch in ("aux1", "aux2"):
            branches[branch] = [
                branch_request("INVITE", scenario, identity),
                branch_response(180, "INVITE", identity, tag=branch),
                branch_request("CANCEL", scenario, identity),
                branch_response(200, "CANCEL", identity, tag=branch),
                branch_response(487, "INVITE", identity, tag=branch),
                branch_ack(scenario, identity),
            ]
        uac.extend(
            (
                message("received", response(180, "INVITE", identity, tag="primary")),
                message("received", response(603, "INVITE", identity, tag="primary")),
                message(
                    "sent", request("ACK", scenario, identity, external_path=False)
                ),
            )
        )
        raw.update(
            global_failure=603,
            cancelled_proceeding_branches=["aux1", "aux2"],
            cancel_requests_per_branch={"primary": 0, "aux1": 1, "aux2": 1},
            upstream_final=603,
        )
    elif scenario == "stray-response-drop":
        stray_identity = "true-stray@example.test"
        branches["primary"] = [
            branch_request("OPTIONS", scenario, identity),
            branch_response(200, "OPTIONS", identity),
            message(
                "sent",
                response(
                    200,
                    "INVITE",
                    stray_identity,
                    scenario_marker=scenario,
                ),
            ),
        ]
        uac.append(message("received", response(200, "OPTIONS", identity)))
        raw.update(
            call_id_sha256=hashlib.sha256(identity.encode()).hexdigest(),
            readiness_options_status=200,
            stray_call_id=stray_identity,
            stray_call_id_sha256=hashlib.sha256(stray_identity.encode()).hexdigest(),
            stray_observation_seconds=0.5,
            stray_upstream_responses=0,
        )
    else:
        raise AssertionError(scenario)

    raw["call_id_sha256"] = hashlib.sha256(identity.encode()).hexdigest()
    raw["branch_request_counts"] = {
        name: scenario_evidence.request_counts(items, "received")
        for name, items in branches.items()
    }
    aggregate_uas = [item for items in branches.values() for item in items]
    write_trace(directory / "uac-messages.log", uac)
    write_trace(directory / "uas-messages.log", aggregate_uas)
    for name, items in branches.items():
        if items:
            write_trace(directory / f"uas-{name}-messages.log", items)
    (directory / "raw-wire.json").write_text(json.dumps(raw))
    packet = valid_packet(scenario, "udp")
    packet["selected_call_ids"] = sorted(
        {
            scenario_evidence.call_id(item)
            for item in scenario_evidence.scenario_messages(
                [*uac, *aggregate_uas], scenario
            )
            if scenario_evidence.call_id(item)
        }
    )
    if scenario == "stray-response-drop":
        packet["assertions"].extend(
            (
                {
                    "name": "one-true-stray-call-observed",
                    "passed": True,
                    "observed": ["true-stray@example.test"],
                },
                {
                    "name": "true-stray-arrived-at-rvoip",
                    "passed": True,
                    "observed": 1,
                },
                {
                    "name": "true-stray-had-zero-rvoip-egress",
                    "passed": True,
                    "observed": 0,
                },
            )
        )
    (directory / "packet-evidence.json").write_text(json.dumps(packet))
    args = argparse.Namespace(
        scenario=scenario,
        directory=directory,
        peer="kamailio",
        order="peer-first",
        transport="udp",
        expected_rvoip_sent_by=RVOIP,
        expected_peer_sent_by=PEER,
    )
    return args, raw


def tcp_fixture(
    directory: Path, scenario: str
) -> tuple[argparse.Namespace, dict[str, object]]:
    identity = f"{scenario}@example.test"
    external = {
        "expected_rvoip_sent_by": RVOIP,
        "expected_peer_sent_by": PEER,
        "observed_proxy_via_order": [RVOIP, PEER],
        "all_via_sent_bys": [RVOIP, PEER, "caller.test:5090"],
    }
    uac: list[scenario_evidence.SipMessage] = []
    uas: list[scenario_evidence.SipMessage] = []
    observations: dict[str, object] = {"external_vias": external}
    observations["uac_binding"] = {
        "requested": "127.0.0.1:0",
        "effective": "127.0.0.1:43123",
        "ephemeral": True,
    }

    def sent(method: str, call: str = identity) -> None:
        uac.append(
            message("sent", request(method, scenario, call, external_path=False), "tcp")
        )

    def downstream(method: str, call: str = identity) -> None:
        uas.append(
            message(
                "received", request(method, scenario, call, external_path=True), "tcp"
            )
        )

    def upstream(
        status: int, method: str, call: str = identity, headers: tuple[str, ...] = ()
    ) -> None:
        uac.append(
            message("received", response(status, method, call, headers=headers), "tcp")
        )

    def downstream_response(
        status: int, method: str, call: str = identity, headers: tuple[str, ...] = ()
    ) -> None:
        uas.append(
            message("sent", response(status, method, call, headers=headers), "tcp")
        )

    if scenario == "timer-c-calling":
        sent("INVITE")
        downstream("INVITE")
        upstream(408, "INVITE")
        observations.update(
            timer_c={
                "configured_ms": 500,
                "elapsed_ms": 510,
                "minimum_ms": 250,
                "maximum_ms": 1500,
            },
            upstream_final_status=408,
            downstream_provisional_responses=0,
        )
    elif scenario == "timer-c-proceeding":
        sent("INVITE")
        downstream("INVITE")
        downstream_response(180, "INVITE")
        upstream(180, "INVITE")
        downstream("CANCEL")
        downstream_response(200, "CANCEL")
        downstream_response(487, "INVITE")
        downstream("ACK")
        upstream(487, "INVITE")
        observations.update(
            timer_c={
                "configured_ms": 500,
                "elapsed_ms": 510,
                "minimum_ms": 250,
                "maximum_ms": 1500,
            },
            upstream_final_status=487,
            downstream_cancel_requests=1,
            cancel_branch="z9hG4bK-rvoip",
        )
    elif scenario == "transport-failure":
        options_identity = f"{scenario}-probe@example.test"
        sent("OPTIONS", options_identity)
        downstream("OPTIONS", options_identity)
        downstream_response(200, "OPTIONS", options_identity)
        upstream(200, "OPTIONS", options_identity)
        sent("INVITE")
        upstream(500, "INVITE")
        observations.update(
            upstream_final_status=500,
            normal_target_received_failed_invite=False,
        )
    elif scenario == "rfc3263-failover":
        sent("INVITE")
        downstream("INVITE")
        downstream_response(200, "INVITE")
        upstream(200, "INVITE")
        observations.update(
            upstream_final_status=200,
            live_candidate_index=2,
            live_target_received_invites=1,
        )
    elif scenario == "capacity-overload":
        second = f"{scenario}-second@example.test"
        sent("INVITE")
        downstream("INVITE")
        sent("INVITE", second)
        upstream(500, "INVITE", second)
        downstream_response(486, "INVITE")
        upstream(486, "INVITE")
        observations.update(
            overload_status=500,
            retry_after=[],
            retry_after_present=False,
            held_contexts_created=1,
            held_calls_released=1,
            capacity_fill_limit=2,
            overloaded_call_reached_target=False,
        )
    elif scenario in {"route-strict", "route-loose-record-route"}:
        sent("INVITE")
        downstream("INVITE")
        downstream_response(200, "INVITE")
        upstream(200, "INVITE")
        observations.update(
            upstream_final_status=200,
            strict_router_recovery=scenario == "route-strict",
            routing={
                "downstream_request_uri": "sip:agent@destination.invalid;transport=tcp",
                "downstream_routes": ["sip:next-hop.invalid;transport=tcp;lr"],
                "downstream_record_routes": ["sip:rvoip.invalid;transport=tcp;lr"],
                "upstream_record_routes": ["sip:rvoip.invalid;transport=tcp;lr"],
                "original_request_uri_preserved": True,
                "local_route_removed": True,
                "next_hop_route_preserved": True,
                "record_route_round_trip": True,
            },
        )
    elif scenario == "auth-aggregation":
        www = (
            'Digest realm="one"',
            'Digest realm="two"',
            'Bearer realm="three"',
        )
        proxy = ('Digest realm="proxy-one"', 'Bearer realm="proxy-two"')
        sent("INVITE")
        for headers in (
            tuple(f"WWW-Authenticate: {value}" for value in www[:2]),
            (f"WWW-Authenticate: {www[2]}",),
            tuple(f"Proxy-Authenticate: {value}" for value in proxy),
        ):
            downstream("INVITE")
            downstream_response(
                407 if headers[0].startswith("Proxy-") else 401,
                "INVITE",
                headers=headers,
            )
        upstream(
            401,
            "INVITE",
            headers=(
                *(f"WWW-Authenticate: {value}" for value in www),
                *(f"Proxy-Authenticate: {value}" for value in proxy),
            ),
        )
        observations.update(
            external_vias=[external, external, external],
            upstream_final_status=401,
            downstream_final_statuses=[401, 401, 407],
            downstream_branch_count=3,
            authentication={
                "www_authenticate": list(www),
                "proxy_authenticate": list(proxy),
                "www_authenticate_count": 3,
                "proxy_authenticate_count": 2,
                "mixed_401_407_aggregation": True,
            },
        )
    else:
        raise AssertionError(scenario)

    write_trace(directory / "uac-messages.log", uac)
    write_trace(directory / "uas-messages.log", uas)
    trace_bytes = {
        path.name: path.stat().st_size
        for path in sorted(directory.glob("*-messages.log"))
    }
    raw: dict[str, object] = {
        "schema": scenario_evidence.TCP_RAW_SCHEMA,
        "scenario": scenario,
        "status": "PASS",
        "transport": "tcp",
        "external_peer_exercised": True,
        "trace_messages": len(uac) + len(uas),
        "trace_bytes": trace_bytes,
        "observations": observations,
    }
    (directory / "raw-wire.json").write_text(json.dumps(raw))
    packet = valid_packet(scenario, "tcp")
    packet["selected_call_ids"] = sorted(
        {
            scenario_evidence.call_id(item)
            for item in scenario_evidence.scenario_messages([*uac, *uas], scenario)
            if scenario_evidence.call_id(item)
        }
    )
    (directory / "packet-evidence.json").write_text(json.dumps(packet))
    args = argparse.Namespace(
        scenario=scenario,
        directory=directory,
        peer="kamailio",
        order="peer-first",
        transport="tcp",
        expected_rvoip_sent_by=RVOIP,
        expected_peer_sent_by=PEER,
    )
    return args, raw


class ScenarioEvidenceTests(unittest.TestCase):
    def test_cancel_retransmission_contract_is_transport_specific(self) -> None:
        self.assertEqual(
            raw_cancel_retransmission.cancel_request_count_for_transport("udp"), 2
        )
        self.assertEqual(
            raw_cancel_retransmission.cancel_request_count_for_transport("tcp"), 1
        )
        self.assertEqual(
            raw_cancel_retransmission.cancel_request_count_for_transport("tls"), 1
        )

        for transport, expected_count, unreliable in (
            ("udp", 2, True),
            ("tcp", 1, False),
            ("tls", 1, False),
        ):
            with self.subTest(transport=transport):
                identity = f"cancel-retransmission-{transport}@example.test"
                uac: list[scenario_evidence.SipMessage] = []
                for _ in range(expected_count):
                    uac.append(
                        message(
                            "sent",
                            request(
                                "CANCEL",
                                "cancel-retransmission",
                                identity,
                                external_path=False,
                            ),
                            transport,
                        )
                    )
                    uac.append(
                        message(
                            "received",
                            response(200, "CANCEL", identity),
                            transport,
                        )
                    )
                raw_wire = {
                    "schema": "rvoip-sip-proxy-interop-raw-wire-v1",
                    "scenario": "cancel-retransmission",
                    "status": "PASS",
                    "transport": transport,
                    "transaction_retransmission_exercised": unreliable,
                    "timer_j_replay_expected": unreliable,
                    "upstream_cancel_requests": expected_count,
                    "upstream_cancel_200_responses": expected_count,
                }
                checks: list[dict[str, object]] = []
                scenario_evidence.validate_cancel_retransmission_contract(
                    checks,
                    transport=transport,
                    raw_wire=raw_wire,
                    uac=uac,
                )
                self.assertTrue(all(item["passed"] for item in checks), checks)
                names = {str(item["name"]) for item in checks}
                if unreliable:
                    self.assertIn(
                        "udp-cancel-retransmission-received-cached-200", names
                    )
                else:
                    self.assertIn(
                        "reliable-transport-used-single-cancel-transaction", names
                    )
                    self.assertNotIn(
                        "udp-cancel-retransmission-received-cached-200", names
                    )

    def test_tcp_cancel_contract_rejects_false_retransmission_claim(self) -> None:
        identity = "cancel-retransmission-tcp@example.test"
        uac = [
            message(
                "sent",
                request(
                    "CANCEL",
                    "cancel-retransmission",
                    identity,
                    external_path=False,
                ),
                "tcp",
            ),
            message("received", response(200, "CANCEL", identity), "tcp"),
        ]
        raw_wire = {
            "schema": "rvoip-sip-proxy-interop-raw-wire-v1",
            "scenario": "cancel-retransmission",
            "status": "PASS",
            "transport": "tcp",
            "transaction_retransmission_exercised": True,
            "timer_j_replay_expected": True,
            "upstream_cancel_requests": 2,
            "upstream_cancel_200_responses": 2,
        }
        checks: list[dict[str, object]] = []
        scenario_evidence.validate_cancel_retransmission_contract(
            checks,
            transport="tcp",
            raw_wire=raw_wire,
            uac=uac,
        )
        failed = {item["name"] for item in checks if not item["passed"]}
        self.assertEqual(
            failed,
            {"cancel-wire-contract-declared-for-transport"},
        )

    def test_original_via_without_rport_ignores_intermediate_rport(self) -> None:
        messages = [
            scenario_evidence.SipMessage(
                direction="received",
                transport="udp",
                payload=(
                    "OPTIONS sip:agent@example.test SIP/2.0\r\n"
                    "Via: SIP/2.0/UDP peer.test:5070;branch=peer\r\n"
                    "Via: SIP/2.0/UDP rvoip.test:5060;branch=rvoip;rport=5060\r\n"
                    "Via: SIP/2.0/UDP caller.test:5090;branch=caller\r\n\r\n"
                ),
            )
        ]
        self.assertTrue(
            scenario_evidence.via_sent_by_port_omits_rport(messages, "received", 5090)
        )
        self.assertFalse(
            scenario_evidence.via_sent_by_port_omits_rport(messages, "received", 5060)
        )

    def test_originating_via_excludes_proxy_hops_and_omits_rport(self) -> None:
        messages = [
            scenario_evidence.SipMessage(
                direction="received",
                transport="tcp",
                payload=(
                    "OPTIONS sip:probe@example.test SIP/2.0\r\n"
                    "Via: SIP/2.0/TCP rvoip.test:5060;branch=rvoip;rport\r\n"
                    "Via: SIP/2.0/TCP peer.test:5070;branch=peer\r\n"
                    "Via: SIP/2.0/TCP caller.test:55001;branch=caller\r\n"
                    "X-Interop-Scenario: via-response-destination\r\n\r\n"
                ),
            )
        ]
        passed, observation = scenario_evidence.originating_vias_omit_rport(
            messages,
            "received",
            "OPTIONS",
            "via-response-destination",
            {"rvoip.test:5060", "peer.test:5070"},
        )
        self.assertTrue(passed)
        self.assertEqual(observation["originating_via_count_per_request"], [1])
        self.assertEqual(observation["originating_via_has_rport"], [False])

    def test_originating_via_rejects_rport_and_ambiguous_non_proxy_hops(
        self,
    ) -> None:
        messages = [
            scenario_evidence.SipMessage(
                direction="received",
                transport="tcp",
                payload=(
                    "OPTIONS sip:probe@example.test SIP/2.0\r\n"
                    "Via: SIP/2.0/TCP rvoip.test:5060;branch=rvoip\r\n"
                    "Via: SIP/2.0/TCP caller.test:55001;branch=caller;rport\r\n"
                    "Via: SIP/2.0/TCP unexpected.test:55002;branch=extra\r\n"
                    "X-Interop-Scenario: via-response-destination\r\n\r\n"
                ),
            )
        ]
        passed, observation = scenario_evidence.originating_vias_omit_rport(
            messages,
            "received",
            "OPTIONS",
            "via-response-destination",
            {"rvoip.test:5060", "peer.test:5070"},
        )
        self.assertFalse(passed)
        self.assertEqual(observation["originating_via_count_per_request"], [2])
        self.assertIn(True, observation["originating_via_has_rport"])

    def test_exact_body_includes_declared_trailing_crlf(self) -> None:
        body = b"bridgefu-interop-request-body-0123456789\r\n"
        wire = (
            b"UDP message received [123] bytes:\n\n"
            b"MESSAGE sip:agent@example.test SIP/2.0\r\n"
            b"Content-Length: 42\r\n\r\n" + body + b"\n--------------------\n"
        )
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "messages.log"
            path.write_bytes(wire)
            exact, observed = scenario_evidence.exact_body_matches(path, body)
        self.assertTrue(exact)
        self.assertEqual(observed["declared_bytes"], len(body))
        self.assertEqual(observed["observed_bytes"], len(body))

    def test_every_udp_advanced_contract_passes_exact_wire_fixture(self) -> None:
        for scenario in sorted(scenario_evidence.UDP_ADVANCED_SCENARIOS):
            with (
                self.subTest(scenario=scenario),
                tempfile.TemporaryDirectory() as temporary,
            ):
                args, _raw = udp_fixture(Path(temporary), scenario)
                self.assertEqual(scenario_evidence.validate_raw(args), 0)
                result = json.loads((Path(temporary) / "result.json").read_text())
                self.assertEqual(result["status"], "PASS")
                self.assertTrue(result["external_peer_exercised"])

    def test_every_tcp_advanced_contract_passes_exact_wire_fixture(self) -> None:
        for scenario in sorted(scenario_evidence.TCP_ADVANCED_SCENARIOS):
            with (
                self.subTest(scenario=scenario),
                tempfile.TemporaryDirectory() as temporary,
            ):
                args, _raw = tcp_fixture(Path(temporary), scenario)
                self.assertEqual(scenario_evidence.validate_raw(args), 0)
                result = json.loads((Path(temporary) / "result.json").read_text())
                self.assertEqual(result["status"], "PASS")
                self.assertTrue(result["external_peer_exercised"])

    def test_timer_c_proceeding_requires_exactly_one_downstream_ack(self) -> None:
        for name, ack_count in (("missing", 0), ("extra", 2)):
            with (
                self.subTest(name=name),
                tempfile.TemporaryDirectory() as temporary,
            ):
                directory = Path(temporary)
                args, raw = tcp_fixture(directory, "timer-c-proceeding")
                uas_path = directory / "uas-messages.log"
                messages = scenario_evidence.read_messages(uas_path)
                ack = next(
                    item for item in messages if item.start_line.startswith("ACK ")
                )
                messages = [
                    item for item in messages if not item.start_line.startswith("ACK ")
                ]
                messages.extend([ack] * ack_count)
                write_trace(uas_path, messages)
                uac_messages = scenario_evidence.read_messages(
                    directory / "uac-messages.log"
                )
                raw["trace_messages"] = len(uac_messages) + len(messages)
                raw["trace_bytes"] = {
                    path.name: path.stat().st_size
                    for path in sorted(directory.glob("*-messages.log"))
                }
                (directory / "raw-wire.json").write_text(json.dumps(raw))

                self.assertEqual(scenario_evidence.validate_raw(args), 1)
                result = json.loads((directory / "result.json").read_text())
                failed = {
                    item["name"]
                    for item in result["assertions"]
                    if item["passed"] is False
                }
                self.assertIn("tcp-exact-downstream-request-shape", failed)

    def test_raw_evidence_rejects_missing_external_peer_via(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            args, _raw = udp_fixture(directory, "multiple-2xx")
            for path in directory.glob("uas*-messages.log"):
                path.write_text(
                    path.read_text().replace(
                        f"Via: SIP/2.0/UDP {PEER};branch=z9hG4bK-peer\n", ""
                    )
                )
            self.assertEqual(scenario_evidence.validate_raw(args), 1)
            result = json.loads((directory / "result.json").read_text())
            failed = {
                item["name"] for item in result["assertions"] if item["passed"] is False
            }
            self.assertIn("external-peer-and-rvoip-vias-on-wire", failed)

    def test_raw_evidence_rejects_claimed_pass_with_wrong_scenario_invariant(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            args, raw = udp_fixture(directory, "sixxx-cancel")
            raw["cancel_requests_per_branch"] = {
                "primary": 0,
                "aux1": 0,
                "aux2": 0,
            }
            (directory / "raw-wire.json").write_text(json.dumps(raw))
            self.assertEqual(scenario_evidence.validate_raw(args), 1)
            result = json.loads((directory / "result.json").read_text())
            failed = {
                item["name"] for item in result["assertions"] if item["passed"] is False
            }
            self.assertIn("sixxx-cancel-counts", failed)

    def test_raw_evidence_rejects_packet_pass_without_named_via_assertion(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            args, _raw = udp_fixture(directory, "stray-response-drop")
            packet_path = directory / "packet-evidence.json"
            packet = json.loads(packet_path.read_text())
            packet["assertions"] = [
                item
                for item in packet["assertions"]
                if item["name"] != "peer-via-observed"
            ]
            packet_path.write_text(json.dumps(packet))
            self.assertEqual(scenario_evidence.validate_raw(args), 1)
            result = json.loads((directory / "result.json").read_text())
            failed = {
                item["name"] for item in result["assertions"] if item["passed"] is False
            }
            self.assertIn("packet-analysis-passed", failed)

    def test_raw_evidence_rejects_packet_from_other_peer_row(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            args, _raw = tcp_fixture(directory, "timer-c-calling")
            packet_path = directory / "packet-evidence.json"
            packet = json.loads(packet_path.read_text())
            packet["peer"] = "opensips"
            packet_path.write_text(json.dumps(packet))
            self.assertEqual(scenario_evidence.validate_raw(args), 1)
            result = json.loads((directory / "result.json").read_text())
            failed = {
                item["name"] for item in result["assertions"] if item["passed"] is False
            }
            self.assertIn("packet-evidence-identity", failed)

    def test_multiple_2xx_rejects_ack_without_reversed_route_set(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            args, _raw = udp_fixture(directory, "multiple-2xx")
            path = directory / "uac-messages.log"
            path.write_text(
                path.read_text().replace(
                    "Route: <sip:peer.test:5070;transport=udp;lr>\n",
                    "",
                    1,
                )
            )
            self.assertEqual(scenario_evidence.validate_raw(args), 1)
            result = json.loads((directory / "result.json").read_text())
            failed = {
                item["name"] for item in result["assertions"] if item["passed"] is False
            }
            self.assertIn(
                "multiple-2xx-end-to-end-ack-route-sets-on-wire",
                failed,
            )

    def test_late_2xx_rejects_post_transaction_termination_claim(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            args, raw = udp_fixture(directory, "late-2xx")
            raw["late_2xx_timing"]["post_transaction_termination_claimed"] = True
            (directory / "raw-wire.json").write_text(json.dumps(raw))
            self.assertEqual(scenario_evidence.validate_raw(args), 1)
            result = json.loads((directory / "result.json").read_text())
            failed = {
                item["name"] for item in result["assertions"] if item["passed"] is False
            }
            self.assertIn(
                "late-2xx-delayed-within-rfc6026-accepted-window",
                failed,
            )

    def test_raw_evidence_rejects_missing_scenario_specific_packet_assertion(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            args, _raw = tcp_fixture(directory, "timer-c-calling")
            packet_path = directory / "packet-evidence.json"
            packet = json.loads(packet_path.read_text())
            packet["assertions"] = [
                item
                for item in packet["assertions"]
                if item["name"] != "timer-c-calling-invite-precedes-408"
            ]
            packet_path.write_text(json.dumps(packet))
            self.assertEqual(scenario_evidence.validate_raw(args), 1)
            result = json.loads((directory / "result.json").read_text())
            failed = {
                item["name"] for item in result["assertions"] if item["passed"] is False
            }
            self.assertIn("packet-analysis-passed", failed)

    def test_tls_sips_evidence_binds_live_request_and_verified_tls(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            args, _row, directory = tls_sips_fixture(Path(temporary))
            self.assertEqual(scenario_evidence.validate_tls(args), 0)
            result = json.loads((directory / "result.json").read_text())
            self.assertEqual(result["status"], "PASS")
            self.assertTrue(result["external_peer_exercised"])
            names = {item["name"] for item in result["assertions"]}
            self.assertTrue(
                {
                    "actual-sips-request-on-boundary-plaintext",
                    "sips-request-uri-preserved",
                    "both-real-proxy-vias",
                    "external-proxy-hops-mtls-only",
                    "independent-tls-verifier",
                    "scenario-owned-packet-capture",
                }
                <= names
            )

    def test_tls_sips_evidence_accepts_buffered_uac_send_with_packet_proof(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            args, _row, directory = tls_sips_fixture(Path(temporary))
            uac_trace = directory / "uac-messages.log"
            write_trace(
                uac_trace,
                [
                    item
                    for item in scenario_evidence.read_messages(uac_trace)
                    if item.direction == "received"
                ],
            )
            self.assertEqual(scenario_evidence.validate_tls(args), 0)
            result = json.loads((directory / "result.json").read_text())
            self.assertEqual(result["status"], "PASS")

    def test_tls_sips_evidence_rejects_downgraded_request_uri(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            args, _row, directory = tls_sips_fixture(Path(temporary))
            uas_trace = directory / "uas-messages.log"
            uas_trace.write_text(
                uas_trace.read_text().replace(
                    "OPTIONS sips:probe@example.test SIP/2.0",
                    "OPTIONS sip:probe@example.test SIP/2.0",
                    1,
                )
            )
            self.assertEqual(scenario_evidence.validate_tls(args), 1)
            result = json.loads((directory / "result.json").read_text())
            failed = {
                item["name"]
                for item in result["assertions"]
                if item["passed"] is False
            }
            self.assertIn(
                "actual-sips-request-on-boundary-plaintext", failed
            )
            self.assertIn("sips-request-uri-preserved", failed)


if __name__ == "__main__":
    unittest.main()
