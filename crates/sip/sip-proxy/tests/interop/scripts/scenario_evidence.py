#!/usr/bin/env python3
"""Validate external SIPp/raw scenarios and record honest supplemental evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Any


SCHEMA = "rvoip-sip-proxy-interop-scenario-v1"
PACKET_SCHEMA = "rvoip-sip-proxy-interop-packet-evidence-v1"
UDP_RAW_SCHEMA = "rvoip-sip-proxy-interop-raw-wire-v1"
TCP_RAW_SCHEMA = "rvoip-sip-proxy-interop-advanced-raw-wire-v1"

UDP_ADVANCED_SCENARIOS = {
    "sequential-fork",
    "parallel-fork",
    "multiple-2xx",
    "late-2xx",
    "sixxx-cancel",
    "stray-response-drop",
}
TCP_ADVANCED_SCENARIOS = {
    "timer-c-calling",
    "timer-c-proceeding",
    "transport-failure",
    "rfc3263-failover",
    "capacity-overload",
    "route-strict",
    "route-loose-record-route",
    "auth-aggregation",
}
ADVANCED_SCENARIOS = UDP_ADVANCED_SCENARIOS | TCP_ADVANCED_SCENARIOS

ADVANCED_PACKET_REQUIREMENTS: dict[str, tuple[set[str], set[int]]] = {
    "sequential-fork": ({"INVITE", "ACK"}, {180, 200, 486}),
    "parallel-fork": ({"INVITE", "ACK"}, {180, 200, 480, 486}),
    "multiple-2xx": ({"INVITE", "ACK"}, {200, 486}),
    "late-2xx": ({"INVITE", "ACK"}, {200, 480, 486}),
    "sixxx-cancel": ({"INVITE", "CANCEL", "ACK"}, {180, 200, 487, 603}),
    "stray-response-drop": ({"OPTIONS"}, {200}),
    "timer-c-calling": ({"INVITE"}, {408}),
    "timer-c-proceeding": ({"INVITE", "CANCEL", "ACK"}, {180, 200, 487}),
    "transport-failure": ({"OPTIONS", "INVITE"}, {200, 500}),
    "rfc3263-failover": ({"INVITE"}, {200}),
    "capacity-overload": ({"INVITE"}, {486, 503}),
    "route-strict": ({"INVITE"}, {200}),
    "route-loose-record-route": ({"INVITE"}, {200}),
    "auth-aggregation": ({"INVITE"}, {401, 407}),
    "sips-routing": ({"OPTIONS"}, {200}),
}
SCENARIO_PACKET_ASSERTIONS: dict[str, set[str]] = {
    "invite-success": {
        "invite-dialog-response-contact-and-record-route-set",
        "invite-dialog-uac-ack-bye-use-contact-and-reversed-route-set",
        "invite-dialog-downstream-ack-bye-reach-contact-with-routes-consumed",
    },
    "multiple-2xx": {
        "multiple-2xx-two-dialog-contacts-and-record-route-sets",
        "multiple-2xx-uac-acks-use-contact-and-reversed-route-set",
        "multiple-2xx-downstream-acks-reach-contact-with-routes-consumed",
    },
    "late-2xx": {
        "late-2xx-two-upstream-forwarding-events",
        "late-2xx-same-dialog-to-tag",
        "late-2xx-each-forwarding-event-has-dialog-ack",
        "late-2xx-packet-delay-within-rfc6026-accepted-window",
    },
    "stray-response-drop": {
        "one-true-stray-call-observed",
        "true-stray-arrived-at-rvoip",
        "true-stray-had-zero-rvoip-egress",
    },
    "timer-c-calling": {
        "timer-c-calling-single-downstream-invite-call",
        "timer-c-calling-invite-precedes-408",
        "timer-c-calling-zero-provisional-responses",
        "timer-c-calling-packet-elapsed-within-bounds",
    },
    "timer-c-proceeding": {
        "timer-c-proceeding-single-downstream-invite-call",
        "timer-c-proceeding-180-precedes-cancel",
        "timer-c-proceeding-cancel-reuses-invite-branch",
        "timer-c-proceeding-packet-elapsed-within-bounds",
    },
    "transport-failure": {
        "transport-failure-single-500-upstream-invite-call",
        "transport-failure-dead-endpoint-syn-observed",
        "transport-failure-dead-endpoint-received-zero-sip",
        "transport-failure-failed-call-never-reached-normal-target",
    },
    "rfc3263-failover": {
        "rfc3263-srv-query-observed",
        "rfc3263-exact-ordered-srv-answers",
        "rfc3263-both-candidate-a-queries-observed",
        "rfc3263-dead-candidate-syn-observed",
        "rfc3263-live-candidate-invite-observed",
        "rfc3263-dead-candidate-received-zero-invites",
    },
    "route-strict": {
        "route-strict-exact-downstream-request-uri",
        "route-strict-exact-downstream-route-set",
        "route-strict-local-route-removed",
        "route-strict-record-route-round-trip",
    },
    "route-loose-record-route": {
        "route-loose-record-route-exact-downstream-request-uri",
        "route-loose-record-route-exact-downstream-route-set",
        "route-loose-record-route-local-route-removed",
        "route-loose-record-route-record-route-round-trip",
    },
    "capacity-overload": {
        "capacity-overload-single-rejected-call",
        "capacity-overload-upstream-status-for-order",
        "capacity-overload-forwarded-calls-all-finished-486",
        "capacity-overload-rejected-call-had-zero-downstream-egress",
        "capacity-overload-exact-final-call-partition",
    },
    "sips-routing": {
        "sips-request-uri-at-uac-boundary",
        "sips-request-uri-at-uas-boundary",
        "sips-request-preserved-end-to-end",
        "sips-both-proxy-vias-observed",
        "no-plaintext-sip-on-external-tls-ports",
        "sips-options-success-observed",
    },
}


@dataclass(frozen=True)
class SipMessage:
    direction: str
    transport: str
    payload: str

    @property
    def start_line(self) -> str:
        return next(
            (line.strip() for line in self.payload.splitlines() if line.strip()), ""
        )

    @property
    def cseq(self) -> str:
        match = re.search(r"(?im)^CSeq:\s*(.+?)\s*$", self.payload)
        return match.group(1).strip() if match else ""


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def read_messages(path: Path) -> list[SipMessage]:
    if not path.is_file():
        return []
    text = path.read_text(encoding="iso-8859-1", errors="replace").replace("\r\n", "\n")
    lines = text.splitlines()
    messages: list[SipMessage] = []
    index = 0
    # SIPp has emitted both `received [577] bytes :` and
    # `sent (497 bytes):` across supported releases. Accept those wire-log
    # headings without weakening any of the SIP assertions that follow.
    header = re.compile(
        r"^(UDP|TCP|TLS) message (sent|received)\s+"
        r"(?:\[\d+\]\s+bytes|\(\d+\s+bytes\))\s*:$",
        re.IGNORECASE,
    )
    while index < len(lines):
        match = header.match(lines[index].strip())
        if not match:
            index += 1
            continue
        transport, direction = match.groups()
        index += 1
        while index < len(lines) and not lines[index].strip():
            index += 1
        payload: list[str] = []
        while index < len(lines) and not lines[index].startswith(
            "--------------------"
        ):
            payload.append(lines[index])
            index += 1
        messages.append(
            SipMessage(
                direction=direction.lower(),
                transport=transport.lower(),
                payload="\n".join(payload).strip() + "\n",
            )
        )
    return messages


def request_count(messages: list[SipMessage], direction: str, method: str) -> int:
    prefix = f"{method} "
    return sum(
        item.direction == direction and item.start_line.startswith(prefix)
        for item in messages
    )


def response_count(
    messages: list[SipMessage],
    direction: str,
    status: int,
    method: str | None = None,
) -> int:
    prefix = f"SIP/2.0 {status} "
    return sum(
        item.direction == direction
        and item.start_line.startswith(prefix)
        and (method is None or item.cseq.upper().endswith(f" {method.upper()}"))
        for item in messages
    )


def contains(messages: list[SipMessage], direction: str, needle: str) -> bool:
    return any(
        item.direction == direction and needle in item.payload for item in messages
    )


def contains_via_sent_by(
    messages: list[SipMessage], direction: str, authority: str
) -> bool:
    pattern = re.compile(
        r"(?im)(?:^Via:\s*|,\s*)SIP/2\.0/[A-Z0-9]+\s+"
        + re.escape(authority)
        + r"(?=[;,\s]|$)"
    )
    return any(
        item.direction == direction and pattern.search(item.payload) is not None
        for item in messages
    )


def via_sent_by_port_omits_rport(
    messages: list[SipMessage], direction: str, port: int
) -> bool:
    sent_by = re.compile(rf"\s[^;,\s]+:{port}(?=[;,\s]|$)")
    for item in messages:
        if item.direction != direction:
            continue
        for header in re.findall(r"(?im)^Via:\s*(.+?)\s*$", item.payload):
            for value in header.split(","):
                if (
                    sent_by.search(value)
                    and re.search(r"(?:^|;)rport(?:[=;]|$)", value, re.IGNORECASE)
                    is None
                ):
                    return True
    return False


def exact_body_matches(path: Path, expected: bytes) -> tuple[bool, dict[str, Any]]:
    if not path.is_file():
        return False, {"reason": "message log missing"}
    data = path.read_bytes()
    marker = data.find(expected)
    if marker < 0:
        return False, {"reason": "expected body not found"}
    delimiter = b"\r\n\r\n"
    header_end = data.rfind(delimiter, 0, marker)
    if header_end < 0:
        delimiter = b"\n\n"
        header_end = data.rfind(delimiter, 0, marker)
    if header_end < 0:
        return False, {"reason": "SIP header/body delimiter not found"}
    header_start = max(0, data.rfind(b"message ", 0, header_end))
    header = data[header_start:header_end]
    lengths = re.findall(rb"(?im)^Content-Length:\s*(\d+)\r?$", header)
    if not lengths:
        return False, {"reason": "Content-Length missing"}
    declared = int(lengths[-1])
    body_start = header_end + len(delimiter)
    actual = data[body_start : body_start + declared]
    return actual == expected, {
        "declared_bytes": declared,
        "observed_bytes": len(actual),
        "body_sha256": hashlib.sha256(actual).hexdigest(),
        "expected_sha256": hashlib.sha256(expected).hexdigest(),
    }


def assertion(
    checks: list[dict[str, Any]], name: str, passed: bool, observed: Any
) -> None:
    checks.append({"name": name, "passed": bool(passed), "observed": observed})


def validate_cancel_retransmission_contract(
    checks: list[dict[str, Any]],
    *,
    transport: str,
    raw_wire: dict[str, Any],
    uac: list[SipMessage],
) -> None:
    """Validate the RFC 3261 transport-specific CANCEL transaction evidence."""
    unreliable = transport == "udp"
    expected_count = 2 if unreliable else 1
    observed_requests = request_count(uac, "sent", "CANCEL")
    observed_responses = response_count(uac, "received", 200, "CANCEL")
    assertion(
        checks,
        "cancel-wire-contract-declared-for-transport",
        raw_wire.get("schema") == "rvoip-sip-proxy-interop-raw-wire-v1"
        and raw_wire.get("scenario") == "cancel-retransmission"
        and raw_wire.get("status") == "PASS"
        and raw_wire.get("transport") == transport
        and raw_wire.get("transaction_retransmission_exercised") is unreliable
        and raw_wire.get("timer_j_replay_expected") is unreliable
        and raw_wire.get("upstream_cancel_requests") == expected_count
        and raw_wire.get("upstream_cancel_200_responses") == expected_count,
        raw_wire,
    )
    if unreliable:
        assertion(
            checks,
            "udp-uac-sent-cancel-transaction-retransmission",
            observed_requests == 2,
            observed_requests,
        )
        assertion(
            checks,
            "udp-cancel-retransmission-received-cached-200",
            observed_responses == 2,
            observed_responses,
        )
    else:
        assertion(
            checks,
            "reliable-transport-used-single-cancel-transaction",
            observed_requests == 1,
            observed_requests,
        )
        assertion(
            checks,
            "reliable-transport-cancel-received-immediate-200",
            observed_responses == 1,
            observed_responses,
        )


def header_values(message: SipMessage, name: str) -> list[str]:
    return [
        match.strip()
        for match in re.findall(
            rf"(?im)^{re.escape(name)}:\s*(.+?)\s*$",
            message.payload,
        )
    ]


def first_header(message: SipMessage, name: str) -> str:
    values = header_values(message, name)
    return values[0] if values else ""


def split_header_entries(message: SipMessage, name: str) -> list[str]:
    entries: list[str] = []
    for value in header_values(message, name):
        start = 0
        angle_depth = 0
        quoted = False
        escaped = False
        for index, character in enumerate(value):
            if escaped:
                escaped = False
                continue
            if quoted and character == "\\":
                escaped = True
            elif character == '"':
                quoted = not quoted
            elif not quoted and character == "<":
                angle_depth += 1
            elif not quoted and character == ">":
                angle_depth = max(0, angle_depth - 1)
            elif not quoted and angle_depth == 0 and character == ",":
                entry = value[start:index].strip()
                if entry:
                    entries.append(entry)
                start = index + 1
        entry = value[start:].strip()
        if entry:
            entries.append(entry)
    return entries


def entry_uri(value: str) -> str:
    match = re.search(r"<\s*([^>]+?)\s*>", value)
    if match is not None:
        return match.group(1).strip()
    return value.strip().split(None, 1)[0]


def uri_header_values(message: SipMessage, name: str) -> list[str]:
    return [entry_uri(value) for value in split_header_entries(message, name)]


def request_uri(message: SipMessage) -> str:
    fields = message.start_line.split()
    if len(fields) == 3 and fields[-1].upper() == "SIP/2.0":
        return fields[1]
    return ""


def to_tag(message: SipMessage) -> str:
    value = first_header(message, "To")
    match = re.search(r"(?:^|;)tag=([^;>\s]+)", value, re.IGNORECASE)
    return match.group(1) if match else ""


def call_id(message: SipMessage) -> str:
    return first_header(message, "Call-ID")


def scenario_messages(messages: list[SipMessage], scenario: str) -> list[SipMessage]:
    marker = re.compile(rf"(?im)^X-Interop-Scenario:\s*{re.escape(scenario)}\s*$")
    return [item for item in messages if marker.search(item.payload) is not None]


def correlated_scenario_messages(
    messages: list[SipMessage], scenario: str
) -> list[SipMessage]:
    marked = scenario_messages(messages, scenario)
    identities = {call_id(item) for item in marked if call_id(item)}
    return [
        item
        for item in messages
        if item in marked or (call_id(item) and call_id(item) in identities)
    ]


def via_sent_bys(message: SipMessage) -> list[str]:
    result: list[str] = []
    for header in header_values(message, "Via"):
        for value in header.split(","):
            match = re.match(
                r"\s*SIP/2\.0/[A-Z0-9]+\s+([^;,\s]+)",
                value,
                re.IGNORECASE,
            )
            if match is not None:
                result.append(match.group(1))
    return result


def originating_via_entries(
    message: SipMessage,
    excluded_sent_bys: set[str],
) -> list[str]:
    """Return Via entries that do not belong to either tested proxy hop."""

    result: list[str] = []
    for entry in split_header_entries(message, "Via"):
        match = re.match(
            r"\s*SIP/2\.0/[A-Z0-9]+\s+([^;,\s]+)",
            entry,
            re.IGNORECASE,
        )
        if match is not None and match.group(1) not in excluded_sent_bys:
            result.append(entry)
    return result


def originating_vias_omit_rport(
    messages: list[SipMessage],
    direction: str,
    method: str,
    scenario: str,
    excluded_sent_bys: set[str],
) -> tuple[bool, dict[str, Any]]:
    """Prove each scenario request has one originating Via without rport."""

    requests = [
        message
        for message in scenario_messages(messages, scenario)
        if message.direction == direction
        and message.start_line.startswith(f"{method} ")
    ]
    entries = [
        originating_via_entries(message, excluded_sent_bys)
        for message in requests
    ]
    flat_entries = [entry for group in entries for entry in group]
    has_rport = [
        re.search(r"(?:^|;)rport(?:[=;]|$)", entry, re.IGNORECASE) is not None
        for entry in flat_entries
    ]
    passed = (
        bool(requests)
        and all(len(group) == 1 for group in entries)
        and not any(has_rport)
    )
    return passed, {
        "request_count": len(requests),
        "originating_via_entries": flat_entries,
        "originating_via_count_per_request": [len(group) for group in entries],
        "originating_via_has_rport": has_rport,
        "excluded_proxy_sent_bys": sorted(excluded_sent_bys),
    }


def expected_proxy_via_order(
    order: str, expected_rvoip: str, expected_peer: str
) -> list[str]:
    if order == "rvoip-first":
        return [expected_peer, expected_rvoip]
    if order == "peer-first":
        return [expected_rvoip, expected_peer]
    return []


def messages_with_external_path(
    messages: list[SipMessage],
    scenario: str,
    order: str,
    expected_rvoip: str,
    expected_peer: str,
) -> tuple[list[SipMessage], list[dict[str, Any]]]:
    candidates = [
        item
        for item in scenario_messages(messages, scenario)
        if item.direction == "received"
        and (
            item.start_line.startswith("INVITE ")
            or item.start_line.startswith("OPTIONS ")
        )
    ]
    expected_order = expected_proxy_via_order(order, expected_rvoip, expected_peer)
    observations: list[dict[str, Any]] = []
    valid: list[SipMessage] = []
    for item in candidates:
        sent_bys = via_sent_bys(item)
        observed_proxy_order = [
            value for value in sent_bys if value in {expected_rvoip, expected_peer}
        ]
        passed = (
            sent_bys.count(expected_rvoip) == 1
            and sent_bys.count(expected_peer) == 1
            and observed_proxy_order == expected_order
        )
        observations.append(
            {
                "start_line": item.start_line,
                "call_id_sha256": (
                    hashlib.sha256(call_id(item).encode()).hexdigest()
                    if call_id(item)
                    else ""
                ),
                "all_via_sent_bys": sent_bys,
                "observed_proxy_via_order": observed_proxy_order,
                "expected_proxy_via_order": expected_order,
                "passed": passed,
            }
        )
        if passed:
            valid.append(item)
    return valid, observations


def response_messages(
    messages: list[SipMessage],
    direction: str,
    status: int,
    method: str,
) -> list[SipMessage]:
    prefix = f"SIP/2.0 {status} "
    return [
        item
        for item in messages
        if item.direction == direction
        and item.start_line.startswith(prefix)
        and item.cseq.upper().endswith(f" {method.upper()}")
    ]


def request_messages(
    messages: list[SipMessage], direction: str, method: str
) -> list[SipMessage]:
    prefix = f"{method.upper()} "
    return [
        item
        for item in messages
        if item.direction == direction and item.start_line.startswith(prefix)
    ]


def request_counts(messages: list[SipMessage], direction: str) -> dict[str, int]:
    result: dict[str, int] = {}
    for item in messages:
        if item.direction != direction or item.start_line.startswith("SIP/2.0 "):
            continue
        method = item.start_line.split(" ", 1)[0].upper()
        if re.fullmatch(r"[A-Z][A-Z0-9.!%*_+`'~-]*", method):
            result[method] = result.get(method, 0) + 1
    return dict(sorted(result.items()))


def hashed_call_ids(messages: list[SipMessage]) -> set[str]:
    return {
        hashlib.sha256(value.encode()).hexdigest()
        for item in messages
        if (value := call_id(item))
    }


def to_tags(messages: list[SipMessage]) -> list[str]:
    result = []
    for item in messages:
        value = first_header(item, "To")
        match = re.search(r"(?:^|;)tag=([^;>\s]+)", value, re.IGNORECASE)
        result.append(match.group(1) if match else "")
    return result


def packet_contract_checks(
    checks: list[dict[str, Any]],
    packet: dict[str, Any],
    *,
    scenario: str,
    peer: str,
    order: str,
    transport: str,
    expected_rvoip: str,
    expected_peer: str,
) -> None:
    captures = packet.get("captures")
    captures = captures if isinstance(captures, list) else []
    capture_names = [
        item.get("filename") for item in captures if isinstance(item, dict)
    ]
    capture_metadata_valid = bool(captures) and all(
        isinstance(item, dict)
        and isinstance(item.get("filename"), str)
        and bool(item["filename"])
        and Path(item["filename"]).name == item["filename"]
        and isinstance(item.get("sha256"), str)
        and re.fullmatch(r"[0-9a-f]{64}", item["sha256"]) is not None
        and isinstance(item.get("bytes"), int)
        and not isinstance(item.get("bytes"), bool)
        and item["bytes"] > 24
        for item in captures
    )
    assertion(
        checks,
        "packet-evidence-identity",
        packet.get("schema") == PACKET_SCHEMA
        and packet.get("scenario") == scenario
        and packet.get("peer") == peer
        and packet.get("order") == order
        and packet.get("transport") == transport,
        {
            key: packet.get(key)
            for key in ("schema", "scenario", "peer", "order", "transport")
        },
    )
    assertion(
        checks,
        "packet-capture-metadata",
        capture_metadata_valid and len(capture_names) == len(set(capture_names)),
        captures,
    )
    analyzer = packet.get("analyzer")
    assertion(
        checks,
        "packet-analyzer-versioned",
        isinstance(analyzer, dict)
        and bool(analyzer.get("tshark"))
        and bool(analyzer.get("libpcap")),
        analyzer,
    )
    packet_assertions = packet.get("assertions")
    packet_assertions = packet_assertions if isinstance(packet_assertions, list) else []
    by_name = {
        item.get("name"): item
        for item in packet_assertions
        if isinstance(item, dict) and isinstance(item.get("name"), str)
    }
    required_assertions = {
        "scenario-call-id-observed",
        "required-methods-observed",
        "required-statuses-observed",
        "rvoip-via-observed",
        "peer-via-observed",
    }
    required_assertions |= SCENARIO_PACKET_ASSERTIONS.get(scenario, set())
    assertion(
        checks,
        "packet-analysis-passed",
        packet.get("status") == "PASS"
        and bool(packet_assertions)
        and all(item.get("passed") is True for item in packet_assertions)
        and required_assertions <= set(by_name)
        and all(by_name[name].get("passed") is True for name in required_assertions),
        {
            "status": packet.get("status"),
            "assertion_names": sorted(by_name),
            "failed": [
                item.get("name")
                for item in packet_assertions
                if isinstance(item, dict) and item.get("passed") is not True
            ],
        },
    )
    selected_call_ids = packet.get("selected_call_ids")
    selected_call_ids = selected_call_ids if isinstance(selected_call_ids, list) else []
    assertion(
        checks,
        "packet-scenario-selection",
        bool(selected_call_ids)
        and all(isinstance(value, str) and bool(value) for value in selected_call_ids)
        and len(selected_call_ids) == len(set(selected_call_ids))
        and isinstance(packet.get("selected_packet_count"), int)
        and not isinstance(packet.get("selected_packet_count"), bool)
        and packet["selected_packet_count"] > 0,
        {
            "selected_call_ids": selected_call_ids,
            "selected_packet_count": packet.get("selected_packet_count"),
        },
    )
    required_methods, base_required_statuses = ADVANCED_PACKET_REQUIREMENTS[scenario]
    observed_methods = packet.get("observed_methods")
    observed_methods = (
        set(observed_methods) if isinstance(observed_methods, list) else set()
    )
    observed_status_values = packet.get("observed_statuses")
    observed_statuses = (
        set(observed_status_values)
        if isinstance(observed_status_values, list)
        and all(
            isinstance(value, int) and not isinstance(value, bool)
            for value in observed_status_values
        )
        else set()
    )
    required_statuses = set(base_required_statuses)
    assertion(
        checks,
        "packet-required-methods",
        required_methods <= observed_methods,
        {
            "required": sorted(required_methods),
            "observed": sorted(observed_methods),
        },
    )
    assertion(
        checks,
        "packet-required-statuses",
        required_statuses <= observed_statuses,
        {
            "required": sorted(required_statuses),
            "observed": sorted(observed_statuses),
        },
    )
    expected_vias = []
    for authority in (expected_rvoip, expected_peer):
        host, separator, port = authority.rpartition(":")
        if separator and host and port.isdigit():
            expected_vias.append((host.strip("[]"), port))
        else:
            expected_vias.append((authority.strip("[]"), ""))
    via_addresses = packet.get("via_sent_by_addresses")
    via_ports = packet.get("via_sent_by_ports")
    via_addresses = set(via_addresses) if isinstance(via_addresses, list) else set()
    via_ports = set(via_ports) if isinstance(via_ports, list) else set()
    assertion(
        checks,
        "packet-exact-proxy-via-authorities",
        all(
            host in via_addresses and (not port or port in via_ports)
            for host, port in expected_vias
        ),
        {
            "expected": expected_vias,
            "addresses": sorted(via_addresses),
            "ports": sorted(via_ports),
        },
    )


def branch_messages(directory: Path, branch: str, scenario: str) -> list[SipMessage]:
    return correlated_scenario_messages(
        read_messages(directory / f"uas-{branch}-messages.log"), scenario
    )


def assert_raw_value(
    checks: list[dict[str, Any]],
    name: str,
    observed: Any,
    expected: Any,
) -> None:
    assertion(
        checks, name, observed == expected, {"expected": expected, "observed": observed}
    )


def validate_udp_raw_contract(
    checks: list[dict[str, Any]],
    *,
    scenario: str,
    directory: Path,
    raw: dict[str, Any],
    packet: dict[str, Any],
    uac: list[SipMessage],
    uas: list[SipMessage],
) -> None:
    raw_counts = raw.get("branch_request_counts")
    raw_counts = raw_counts if isinstance(raw_counts, dict) else {}
    observed_counts = {
        branch: request_counts(branch_messages(directory, branch, scenario), "received")
        for branch in ("primary", "aux1", "aux2")
    }
    observed_counts = {
        branch: counts
        for branch, counts in observed_counts.items()
        if counts or branch in raw_counts
    }
    assertion(
        checks,
        "branch-request-counts-match-logs",
        raw_counts == observed_counts,
        {"raw": raw_counts, "logs": observed_counts},
    )

    scenario_uac = correlated_scenario_messages(uac, scenario)
    scenario_uas = correlated_scenario_messages(uas, scenario)
    sent_invites = request_messages(scenario_uac, "sent", "INVITE")
    if scenario != "stray-response-drop":
        assertion(
            checks,
            "one-upstream-invite",
            len(sent_invites) == 1,
            len(sent_invites),
        )
        expected_hash = raw.get("call_id_sha256")
        assertion(
            checks,
            "raw-call-id-matches-wire",
            isinstance(expected_hash, str)
            and re.fullmatch(r"[0-9a-f]{64}", expected_hash) is not None
            and hashed_call_ids(sent_invites) == {expected_hash},
            {
                "raw": expected_hash,
                "wire": sorted(hashed_call_ids(sent_invites)),
            },
        )

    expected_branch_counts: dict[str, dict[str, int]]
    if scenario == "sequential-fork":
        expected_branch_counts = {
            "primary": {"ACK": 1, "INVITE": 1},
            "aux1": {"ACK": 1, "INVITE": 1},
            "aux2": {},
        }
        assert_raw_value(
            checks,
            "sequential-branch-order",
            raw.get("branch_order"),
            ["primary", "aux1"],
        )
        assert_raw_value(
            checks,
            "sequential-branch-finals",
            raw.get("branch_finals"),
            {"primary": 486, "aux1": 200},
        )
        assert_raw_value(
            checks, "sequential-unused-branch", raw.get("unused_branches"), ["aux2"]
        )
        assert_raw_value(
            checks, "sequential-upstream-final", raw.get("upstream_final"), 200
        )
        expected_sent_responses = {
            "primary": {(486, "INVITE"): 1},
            "aux1": {(180, "INVITE"): 1, (200, "INVITE"): 1},
            "aux2": {},
        }
    elif scenario == "parallel-fork":
        expected_branch_counts = {
            branch: {"ACK": 1, "INVITE": 1} for branch in ("primary", "aux1", "aux2")
        }
        assert_raw_value(
            checks,
            "parallel-invite-branches",
            raw.get("parallel_invite_branches"),
            ["primary", "aux1", "aux2"],
        )
        assert_raw_value(
            checks,
            "parallel-branch-finals",
            raw.get("branch_finals"),
            {"primary": 486, "aux1": 480, "aux2": 200},
        )
        assert_raw_value(
            checks, "parallel-upstream-final", raw.get("upstream_final"), 200
        )
        expected_sent_responses = {
            "primary": {(486, "INVITE"): 1},
            "aux1": {(480, "INVITE"): 1},
            "aux2": {(180, "INVITE"): 1, (200, "INVITE"): 1},
        }
    elif scenario == "multiple-2xx":
        expected_branch_counts = {
            branch: {"ACK": 1, "INVITE": 1} for branch in ("primary", "aux1", "aux2")
        }
        assert_raw_value(
            checks, "multiple-distinct-to-tags", raw.get("distinct_2xx_to_tags"), 2
        )
        assert_raw_value(
            checks, "multiple-upstream-2xx", raw.get("upstream_invite_2xx"), 2
        )
        assert_raw_value(
            checks,
            "multiple-acked-branches",
            raw.get("acked_2xx_branches"),
            ["primary", "aux1"],
        )
        assert_raw_value(
            checks,
            "multiple-failure-branch",
            raw.get("failure_branches"),
            {"aux2": 486},
        )
        expected_sent_responses = {
            "primary": {(200, "INVITE"): 1},
            "aux1": {(200, "INVITE"): 1},
            "aux2": {(486, "INVITE"): 1},
        }
        upstream_2xx = response_messages(scenario_uac, "received", 200, "INVITE")
        assertion(
            checks,
            "two-distinct-upstream-2xx-dialogs",
            len(upstream_2xx) == 2
            and len(set(to_tags(upstream_2xx))) == 2
            and "" not in to_tags(upstream_2xx),
            to_tags(upstream_2xx),
        )
        upstream_acks = request_messages(scenario_uac, "sent", "ACK")
        route_observations: list[dict[str, Any]] = []
        route_checks_passed = len(upstream_2xx) == len(upstream_acks) == 2
        for response in upstream_2xx:
            tag = to_tag(response)
            contacts = uri_header_values(response, "Contact")
            record_routes = uri_header_values(response, "Record-Route")
            matching_upstream_acks = [
                item for item in upstream_acks if to_tag(item) == tag
            ]
            downstream_matches: list[tuple[str, SipMessage]] = []
            for branch in ("primary", "aux1", "aux2"):
                downstream_matches.extend(
                    (branch, item)
                    for item in branch_messages(directory, branch, scenario)
                    if item.direction == "received"
                    and item.start_line.startswith("ACK ")
                    and to_tag(item) == tag
                )
            expected_routes = list(reversed(record_routes))
            upstream_ack = (
                matching_upstream_acks[0] if len(matching_upstream_acks) == 1 else None
            )
            downstream = downstream_matches[0] if len(downstream_matches) == 1 else None
            passed = (
                bool(tag)
                and len(contacts) == 1
                and len(record_routes) >= 2
                and upstream_ack is not None
                and request_uri(upstream_ack) == contacts[0]
                and uri_header_values(upstream_ack, "Route") == expected_routes
                and downstream is not None
                and request_uri(downstream[1]) == contacts[0]
                and not uri_header_values(downstream[1], "Route")
            )
            route_checks_passed = route_checks_passed and passed
            route_observations.append(
                {
                    "branch": downstream[0] if downstream is not None else "",
                    "response_contact_uri": contacts[0] if len(contacts) == 1 else "",
                    "response_record_route_set": record_routes,
                    "uac_ack_request_uri": (
                        request_uri(upstream_ack) if upstream_ack is not None else ""
                    ),
                    "uac_ack_route_set": (
                        uri_header_values(upstream_ack, "Route")
                        if upstream_ack is not None
                        else []
                    ),
                    "downstream_ack_request_uri": (
                        request_uri(downstream[1]) if downstream is not None else ""
                    ),
                    "downstream_ack_route_set": (
                        uri_header_values(downstream[1], "Route")
                        if downstream is not None
                        else []
                    ),
                    "route_values_consumed_by_proxies": (
                        downstream is not None
                        and not uri_header_values(downstream[1], "Route")
                    ),
                }
            )
        assertion(
            checks,
            "multiple-2xx-end-to-end-ack-route-sets-on-wire",
            route_checks_passed,
            route_observations,
        )
        assert_raw_value(
            checks,
            "multiple-2xx-raw-ack-routes-match-wire",
            raw.get("dialog_ack_routes"),
            route_observations,
        )
    elif scenario == "late-2xx":
        expected_branch_counts = {
            "primary": {"ACK": 2, "INVITE": 1},
            "aux1": {"ACK": 1, "INVITE": 1},
            "aux2": {"ACK": 1, "INVITE": 1},
        }
        assertion(
            checks,
            "late-2xx-delay-positive",
            isinstance(raw.get("late_2xx_delay_seconds"), (int, float))
            and not isinstance(raw.get("late_2xx_delay_seconds"), bool)
            and raw["late_2xx_delay_seconds"] > 0,
            raw.get("late_2xx_delay_seconds"),
        )
        timing = raw.get("late_2xx_timing")
        timing = timing if isinstance(timing, dict) else {}
        requested_delay = timing.get("requested_delay_seconds")
        observed_interval = timing.get("observed_forwarding_interval_seconds")
        accepted_window = timing.get("accepted_window_seconds")
        assertion(
            checks,
            "late-2xx-delayed-within-rfc6026-accepted-window",
            timing.get("phase") == "rfc6026-accepted"
            and timing.get("post_transaction_termination_claimed") is False
            and timing.get("within_accepted_window") is True
            and isinstance(requested_delay, (int, float))
            and not isinstance(requested_delay, bool)
            and requested_delay >= 0.5
            and requested_delay == raw.get("late_2xx_delay_seconds")
            and isinstance(observed_interval, (int, float))
            and not isinstance(observed_interval, bool)
            and observed_interval >= requested_delay * 0.8
            and isinstance(accepted_window, (int, float))
            and not isinstance(accepted_window, bool)
            and observed_interval < accepted_window <= 64.0,
            timing,
        )
        assert_raw_value(
            checks, "late-same-dialog-count", raw.get("same_dialog_upstream_2xx"), 2
        )
        assert_raw_value(
            checks,
            "late-every-2xx-acked",
            raw.get("acked_2xx_branches"),
            ["primary", "primary"],
        )
        assert_raw_value(
            checks,
            "late-failure-branches",
            raw.get("failure_branches"),
            {"aux1": 480, "aux2": 486},
        )
        expected_sent_responses = {
            "primary": {(200, "INVITE"): 2},
            "aux1": {(480, "INVITE"): 1},
            "aux2": {(486, "INVITE"): 1},
        }
        upstream_2xx = response_messages(scenario_uac, "received", 200, "INVITE")
        assertion(
            checks,
            "late-2xx-forwarded-same-dialog",
            len(upstream_2xx) == 2
            and len(set(to_tags(upstream_2xx))) == 1
            and to_tags(upstream_2xx)[0] != "",
            to_tags(upstream_2xx),
        )
        upstream_acks = request_messages(scenario_uac, "sent", "ACK")
        primary_acks = request_messages(
            branch_messages(directory, "primary", scenario),
            "received",
            "ACK",
        )
        contacts = {tuple(uri_header_values(item, "Contact")) for item in upstream_2xx}
        response_routes = {
            tuple(uri_header_values(item, "Record-Route")) for item in upstream_2xx
        }
        contact = (
            next(iter(contacts))[0]
            if contacts and len(next(iter(contacts))) == 1
            else ""
        )
        routes = list(next(iter(response_routes))) if len(response_routes) == 1 else []
        late_ack_wire_ok = (
            len(upstream_acks) == len(primary_acks) == 2
            and len(contacts) == 1
            and bool(contact)
            and len(routes) >= 2
            and all(
                request_uri(item) == contact
                and uri_header_values(item, "Route") == list(reversed(routes))
                for item in upstream_acks
            )
            and all(
                request_uri(item) == contact and not uri_header_values(item, "Route")
                for item in primary_acks
            )
        )
        assertion(
            checks,
            "late-2xx-every-forwarding-event-acked-on-wire",
            late_ack_wire_ok,
            {
                "upstream_ack_count": len(upstream_acks),
                "downstream_ack_count": len(primary_acks),
                "contact": contact,
                "response_record_routes": routes,
                "upstream_ack_routes": [
                    uri_header_values(item, "Route") for item in upstream_acks
                ],
                "downstream_ack_routes": [
                    uri_header_values(item, "Route") for item in primary_acks
                ],
            },
        )
        late_raw_ack_routes = raw.get("late_dialog_ack_routes")
        assertion(
            checks,
            "late-2xx-raw-ack-count-and-consumption",
            isinstance(late_raw_ack_routes, list)
            and len(late_raw_ack_routes) == 2
            and all(
                isinstance(item, dict)
                and item.get("response_contact_uri") == contact
                and item.get("uac_ack_request_uri") == contact
                and item.get("uac_ack_route_set") == list(reversed(routes))
                and item.get("downstream_ack_request_uri") == contact
                and item.get("downstream_ack_route_set") == []
                and item.get("route_values_consumed_by_proxies") is True
                for item in late_raw_ack_routes
            ),
            late_raw_ack_routes,
        )
    elif scenario == "sixxx-cancel":
        expected_branch_counts = {
            "primary": {"ACK": 1, "INVITE": 1},
            "aux1": {"ACK": 1, "CANCEL": 1, "INVITE": 1},
            "aux2": {"ACK": 1, "CANCEL": 1, "INVITE": 1},
        }
        assert_raw_value(checks, "sixxx-global-failure", raw.get("global_failure"), 603)
        assert_raw_value(
            checks,
            "sixxx-cancelled-branches",
            raw.get("cancelled_proceeding_branches"),
            ["aux1", "aux2"],
        )
        assert_raw_value(
            checks,
            "sixxx-cancel-counts",
            raw.get("cancel_requests_per_branch"),
            {"primary": 0, "aux1": 1, "aux2": 1},
        )
        assert_raw_value(checks, "sixxx-upstream-final", raw.get("upstream_final"), 603)
        expected_sent_responses = {
            "primary": {(180, "INVITE"): 1, (603, "INVITE"): 1},
            "aux1": {
                (180, "INVITE"): 1,
                (200, "CANCEL"): 1,
                (487, "INVITE"): 1,
            },
            "aux2": {
                (180, "INVITE"): 1,
                (200, "CANCEL"): 1,
                (487, "INVITE"): 1,
            },
        }
        assertion(
            checks,
            "sixxx-reached-upstream",
            len(response_messages(scenario_uac, "received", 603, "INVITE")) == 1,
            response_count(scenario_uac, "received", 603, "INVITE"),
        )
    elif scenario == "stray-response-drop":
        expected_branch_counts = {"primary": {"OPTIONS": 1}}
        assert_raw_value(
            checks, "stray-readiness-status", raw.get("readiness_options_status"), 200
        )
        assert_raw_value(
            checks,
            "stray-upstream-response-count",
            raw.get("stray_upstream_responses"),
            0,
        )
        assertion(
            checks,
            "stray-observation-window-positive",
            isinstance(raw.get("stray_observation_seconds"), (int, float))
            and not isinstance(raw.get("stray_observation_seconds"), bool)
            and raw["stray_observation_seconds"] > 0,
            raw.get("stray_observation_seconds"),
        )
        expected_sent_responses = {"primary": {(200, "OPTIONS"): 1, (200, "INVITE"): 1}}
        readiness_requests = request_messages(scenario_uac, "sent", "OPTIONS")
        stray_responses = response_messages(scenario_uas, "sent", 200, "INVITE")
        stray_identity = raw.get("stray_call_id")
        stray_hash = raw.get("stray_call_id_sha256")
        packet_identities = packet.get("selected_call_ids")
        packet_identities = (
            set(packet_identities) if isinstance(packet_identities, list) else set()
        )
        packet_assertions = {
            item.get("name"): item
            for item in packet.get("assertions", [])
            if isinstance(item, dict) and isinstance(item.get("name"), str)
        }
        packet_stray_identity = packet_assertions.get(
            "one-true-stray-call-observed", {}
        ).get("observed")
        packet_stray_arrivals = packet_assertions.get(
            "true-stray-arrived-at-rvoip", {}
        ).get("observed")
        packet_stray_egress = packet_assertions.get(
            "true-stray-had-zero-rvoip-egress", {}
        ).get("observed")
        assertion(
            checks,
            "stray-wire-identities",
            len(readiness_requests) == 1
            and len(stray_responses) == 1
            and isinstance(stray_identity, str)
            and bool(stray_identity)
            and isinstance(stray_hash, str)
            and re.fullmatch(r"[0-9a-f]{64}", stray_hash) is not None
            and hashlib.sha256(stray_identity.encode()).hexdigest() == stray_hash
            and {call_id(item) for item in stray_responses} == {stray_identity}
            and hashed_call_ids(stray_responses) == {stray_hash}
            and stray_identity in packet_identities
            and packet_stray_identity == [stray_identity]
            and isinstance(packet_stray_arrivals, int)
            and not isinstance(packet_stray_arrivals, bool)
            and packet_stray_arrivals > 0
            and packet_stray_egress == 0
            and stray_hash not in hashed_call_ids(scenario_uac),
            {
                "readiness_requests": len(readiness_requests),
                "stray_raw_call_id": stray_identity,
                "stray_raw_hash": stray_hash,
                "stray_uas_hashes": sorted(hashed_call_ids(stray_responses)),
                "packet_call_ids": sorted(packet_identities),
                "packet_stray_identity": packet_stray_identity,
                "packet_stray_arrivals": packet_stray_arrivals,
                "packet_stray_egress": packet_stray_egress,
                "uac_hashes": sorted(hashed_call_ids(scenario_uac)),
            },
        )
    else:
        raise ValueError(f"unsupported UDP advanced scenario: {scenario}")

    assertion(
        checks,
        "exact-branch-request-shape",
        {branch: observed_counts.get(branch, {}) for branch in expected_branch_counts}
        == expected_branch_counts,
        {
            "expected": expected_branch_counts,
            "observed": observed_counts,
        },
    )
    observed_sent_responses: dict[str, dict[tuple[int, str], int]] = {}
    for branch in expected_sent_responses:
        messages = branch_messages(directory, branch, scenario)
        counts: dict[tuple[int, str], int] = {}
        for item in messages:
            if item.direction != "sent" or not item.start_line.startswith("SIP/2.0 "):
                continue
            fields = item.start_line.split()
            if len(fields) < 2 or not fields[1].isdigit():
                continue
            key = (int(fields[1]), item.cseq.split()[-1].upper() if item.cseq else "")
            counts[key] = counts.get(key, 0) + 1
        observed_sent_responses[branch] = counts
    assertion(
        checks,
        "exact-downstream-response-shape",
        observed_sent_responses == expected_sent_responses,
        {
            "expected": {
                branch: {
                    f"{status}/{method}": count
                    for (status, method), count in values.items()
                }
                for branch, values in expected_sent_responses.items()
            },
            "observed": {
                branch: {
                    f"{status}/{method}": count
                    for (status, method), count in values.items()
                }
                for branch, values in observed_sent_responses.items()
            },
        },
    )


def timer_contract_valid(timer: Any) -> bool:
    if not isinstance(timer, dict):
        return False
    values = [
        timer.get("configured_ms"),
        timer.get("elapsed_ms"),
        timer.get("minimum_ms"),
        timer.get("maximum_ms"),
    ]
    if not all(
        isinstance(value, (int, float)) and not isinstance(value, bool) and value > 0
        for value in values
    ):
        return False
    return (
        timer["minimum_ms"] <= timer["elapsed_ms"] <= timer["maximum_ms"]
        and timer["minimum_ms"] <= timer["configured_ms"] <= timer["maximum_ms"]
    )


def validate_tcp_raw_contract(
    checks: list[dict[str, Any]],
    *,
    scenario: str,
    directory: Path,
    raw: dict[str, Any],
    uac: list[SipMessage],
    uas: list[SipMessage],
    order: str,
    expected_rvoip: str,
    expected_peer: str,
) -> None:
    observations = raw.get("observations")
    observations = observations if isinstance(observations, dict) else {}
    trace_bytes = raw.get("trace_bytes")
    trace_bytes = trace_bytes if isinstance(trace_bytes, dict) else {}
    actual_trace_bytes = {
        path.name: path.stat().st_size
        for path in sorted(directory.glob("*-messages.log"))
        if path.is_file() and not path.is_symlink()
    }
    assertion(
        checks,
        "tcp-trace-byte-accounting",
        bool(actual_trace_bytes)
        and trace_bytes == actual_trace_bytes
        and all(isinstance(value, int) and value > 0 for value in trace_bytes.values()),
        {"raw": trace_bytes, "actual": actual_trace_bytes},
    )
    assertion(
        checks,
        "tcp-trace-message-accounting",
        isinstance(raw.get("trace_messages"), int)
        and not isinstance(raw.get("trace_messages"), bool)
        and raw["trace_messages"] == len(uac) + len(uas)
        and raw["trace_messages"] > 0,
        {
            "raw": raw.get("trace_messages"),
            "uac_plus_aggregate_uas": len(uac) + len(uas),
        },
    )

    scenario_uac = correlated_scenario_messages(uac, scenario)
    scenario_uas = correlated_scenario_messages(uas, scenario)
    external = observations.get("external_vias")
    external_items = external if isinstance(external, list) else [external]
    expected_order = expected_proxy_via_order(order, expected_rvoip, expected_peer)
    assertion(
        checks,
        "raw-external-via-observations",
        bool(external_items)
        and all(
            isinstance(item, dict)
            and item.get("expected_rvoip_sent_by") == expected_rvoip
            and item.get("expected_peer_sent_by") == expected_peer
            and item.get("observed_proxy_via_order") == expected_order
            and isinstance(item.get("all_via_sent_bys"), list)
            and item["all_via_sent_bys"].count(expected_rvoip) == 1
            and item["all_via_sent_bys"].count(expected_peer) == 1
            for item in external_items
        ),
        external,
    )
    binding = observations.get("uac_binding")
    binding = binding if isinstance(binding, dict) else {}
    requested_binding = binding.get("requested")
    effective_binding = binding.get("effective")
    requested_host, requested_separator, requested_port = (
        requested_binding.rpartition(":")
        if isinstance(requested_binding, str)
        else ("", "", "")
    )
    effective_host, effective_separator, effective_port = (
        effective_binding.rpartition(":")
        if isinstance(effective_binding, str)
        else ("", "", "")
    )
    assertion(
        checks,
        "tcp-uac-used-explicit-ephemeral-binding",
        binding.get("ephemeral") is True
        and requested_separator == ":"
        and requested_host
        and requested_port == "0"
        and effective_separator == ":"
        and effective_host
        and effective_port.isdigit()
        and int(effective_port) > 0,
        binding,
    )

    if scenario == "timer-c-calling":
        assert_raw_value(
            checks,
            "timer-c-calling-final",
            observations.get("upstream_final_status"),
            408,
        )
        assert_raw_value(
            checks,
            "timer-c-calling-no-provisional",
            observations.get("downstream_provisional_responses"),
            0,
        )
        assertion(
            checks,
            "timer-c-calling-bounds",
            timer_contract_valid(observations.get("timer_c")),
            observations.get("timer_c"),
        )
        expected_uas_requests = {"INVITE": 1}
        expected_uac_statuses = {(408, "INVITE"): 1}
    elif scenario == "timer-c-proceeding":
        assert_raw_value(
            checks,
            "timer-c-proceeding-final",
            observations.get("upstream_final_status"),
            487,
        )
        assert_raw_value(
            checks,
            "timer-c-proceeding-one-cancel",
            observations.get("downstream_cancel_requests"),
            1,
        )
        assertion(
            checks,
            "timer-c-proceeding-cancel-branch",
            isinstance(observations.get("cancel_branch"), str)
            and observations["cancel_branch"].startswith("z9hG4bK"),
            observations.get("cancel_branch"),
        )
        assertion(
            checks,
            "timer-c-proceeding-bounds",
            timer_contract_valid(observations.get("timer_c")),
            observations.get("timer_c"),
        )
        expected_uas_requests = {"ACK": 1, "CANCEL": 1, "INVITE": 1}
        expected_uac_statuses = {(180, "INVITE"): 1, (487, "INVITE"): 1}
    elif scenario == "transport-failure":
        failure_status = observations.get("upstream_final_status")
        assert_raw_value(
            checks,
            "transport-failure-upstream-aggregate-status",
            failure_status,
            500,
        )
        assert_raw_value(
            checks,
            "transport-failure-bypassed-live-target",
            observations.get("normal_target_received_failed_invite"),
            False,
        )
        expected_uas_requests = {"OPTIONS": 1}
        expected_uac_statuses = {(200, "OPTIONS"): 1, (failure_status, "INVITE"): 1}
    elif scenario == "rfc3263-failover":
        assert_raw_value(
            checks, "failover-final", observations.get("upstream_final_status"), 200
        )
        assert_raw_value(
            checks, "failover-live-index", observations.get("live_candidate_index"), 2
        )
        assert_raw_value(
            checks,
            "failover-live-invite-count",
            observations.get("live_target_received_invites"),
            1,
        )
        expected_uas_requests = {"INVITE": 1}
        expected_uac_statuses = {(200, "INVITE"): 1}
    elif scenario == "capacity-overload":
        expected_upstream_overload = 500 if order == "peer-first" else 503
        assert_raw_value(
            checks,
            "overload-upstream-status-for-order",
            observations.get("overload_status"),
            expected_upstream_overload,
        )
        retry_after = observations.get("retry_after")
        assertion(
            checks,
            "overload-optional-retry-after-well-formed",
            retry_after is None
            or (
                isinstance(retry_after, list)
                and all(
                    isinstance(value, str) and value.strip() for value in retry_after
                )
            ),
            retry_after,
        )
        assert_raw_value(
            checks,
            "overload-retry-after-presence-recorded",
            observations.get("retry_after_present"),
            bool(retry_after),
        )
        capacity_limit = observations.get("capacity_fill_limit")
        held_created = observations.get("held_contexts_created")
        held_released = observations.get("held_calls_released")
        assertion(
            checks,
            "overload-filled-and-released-capacity",
            isinstance(capacity_limit, int)
            and not isinstance(capacity_limit, bool)
            and capacity_limit > 0
            and isinstance(held_created, int)
            and not isinstance(held_created, bool)
            and 1 <= held_created < capacity_limit
            and held_released == held_created,
            {
                "capacity_fill_limit": capacity_limit,
                "held_contexts_created": held_created,
                "held_calls_released": held_released,
            },
        )
        assert_raw_value(
            checks,
            "overload-not-forwarded",
            observations.get("overloaded_call_reached_target"),
            False,
        )
        expected_uas_requests = {"INVITE": held_created}
        expected_uac_statuses = {
            (486, "INVITE"): held_created,
            (expected_upstream_overload, "INVITE"): 1,
        }
        assertion(
            checks,
            "overload-used-distinct-calls",
            isinstance(held_created, int)
            and not isinstance(held_created, bool)
            and len(
                {
                    call_id(item)
                    for item in request_messages(scenario_uac, "sent", "INVITE")
                    if call_id(item)
                }
            )
            == held_created + 1,
            [
                call_id(item)
                for item in request_messages(scenario_uac, "sent", "INVITE")
            ],
        )
    elif scenario in {"route-strict", "route-loose-record-route"}:
        assert_raw_value(
            checks, "routing-final", observations.get("upstream_final_status"), 200
        )
        assert_raw_value(
            checks,
            "strict-router-mode",
            observations.get("strict_router_recovery"),
            scenario == "route-strict",
        )
        routing = observations.get("routing")
        routing = routing if isinstance(routing, dict) else {}
        for field in (
            "original_request_uri_preserved",
            "local_route_removed",
            "next_hop_route_preserved",
            "record_route_round_trip",
        ):
            assert_raw_value(
                checks, f"routing-{field.replace('_', '-')}", routing.get(field), True
            )
        downstream_routes = routing.get("downstream_routes")
        downstream_record_routes = routing.get("downstream_record_routes")
        upstream_record_routes = routing.get("upstream_record_routes")
        assertion(
            checks,
            "routing-wire-shape",
            isinstance(routing.get("downstream_request_uri"), str)
            and routing["downstream_request_uri"].lower().startswith(("sip:", "sips:"))
            and isinstance(downstream_routes, list)
            and len(downstream_routes) == 1
            and isinstance(downstream_record_routes, list)
            and bool(downstream_record_routes)
            and isinstance(upstream_record_routes, list)
            and bool(set(downstream_record_routes) & set(upstream_record_routes)),
            routing,
        )
        expected_uas_requests = {"INVITE": 1}
        expected_uac_statuses = {(200, "INVITE"): 1}
    elif scenario == "auth-aggregation":
        final_status = observations.get("upstream_final_status")
        assertion(checks, "auth-final-status", final_status in (401, 407), final_status)
        assert_raw_value(
            checks,
            "auth-downstream-statuses",
            observations.get("downstream_final_statuses"),
            [401, 401, 407],
        )
        assert_raw_value(
            checks,
            "auth-downstream-branches",
            observations.get("downstream_branch_count"),
            3,
        )
        auth = observations.get("authentication")
        auth = auth if isinstance(auth, dict) else {}
        assert_raw_value(
            checks,
            "auth-mixed-aggregation",
            auth.get("mixed_401_407_aggregation"),
            True,
        )
        assert_raw_value(
            checks, "auth-www-count", auth.get("www_authenticate_count"), 3
        )
        assert_raw_value(
            checks, "auth-proxy-count", auth.get("proxy_authenticate_count"), 2
        )
        downstream_www = sorted(
            value
            for item in scenario_uas
            if item.direction == "sent" and item.start_line.startswith("SIP/2.0 401 ")
            for value in header_values(item, "WWW-Authenticate")
        )
        downstream_proxy = sorted(
            value
            for item in scenario_uas
            if item.direction == "sent" and item.start_line.startswith("SIP/2.0 407 ")
            for value in header_values(item, "Proxy-Authenticate")
        )
        upstream_finals = [
            item
            for item in scenario_uac
            if item.direction == "received"
            and item.start_line.startswith(("SIP/2.0 401 ", "SIP/2.0 407 "))
        ]
        upstream_www = sorted(
            value
            for item in upstream_finals
            for value in header_values(item, "WWW-Authenticate")
        )
        upstream_proxy = sorted(
            value
            for item in upstream_finals
            for value in header_values(item, "Proxy-Authenticate")
        )
        assertion(
            checks,
            "auth-headers-aggregated-on-wire",
            len(upstream_finals) == 1
            and downstream_www == upstream_www
            and downstream_proxy == upstream_proxy
            and auth.get("www_authenticate") is not None
            and sorted(auth.get("www_authenticate", [])) == upstream_www
            and sorted(auth.get("proxy_authenticate", [])) == upstream_proxy,
            {
                "downstream_www": downstream_www,
                "upstream_www": upstream_www,
                "downstream_proxy": downstream_proxy,
                "upstream_proxy": upstream_proxy,
            },
        )
        expected_uas_requests = {"INVITE": 3}
        expected_uac_statuses = {(final_status, "INVITE"): 1}
    else:
        raise ValueError(f"unsupported TCP advanced scenario: {scenario}")

    observed_uas_requests = request_counts(scenario_uas, "received")
    assertion(
        checks,
        "tcp-exact-downstream-request-shape",
        observed_uas_requests == expected_uas_requests,
        {"expected": expected_uas_requests, "observed": observed_uas_requests},
    )
    observed_uac_statuses: dict[tuple[Any, str], int] = {}
    for (status, method), _expected_count in expected_uac_statuses.items():
        observed_uac_statuses[(status, method)] = len(
            response_messages(scenario_uac, "received", status, method)
        )
    assertion(
        checks,
        "tcp-exact-upstream-response-shape",
        observed_uac_statuses == expected_uac_statuses,
        {
            "expected": {
                f"{status}/{method}": count
                for (status, method), count in expected_uac_statuses.items()
            },
            "observed": {
                f"{status}/{method}": count
                for (status, method), count in observed_uac_statuses.items()
            },
        },
    )


def validate_raw(args: argparse.Namespace) -> int:
    directory = args.directory
    checks: list[dict[str, Any]] = []
    scenario = args.scenario
    if scenario not in ADVANCED_SCENARIOS:
        raise SystemExit(f"unsupported external raw scenario: {scenario}")
    if (scenario in UDP_ADVANCED_SCENARIOS) != (args.transport == "udp"):
        raise SystemExit(
            f"scenario {scenario} is not defined for transport {args.transport}"
        )
    if (scenario in TCP_ADVANCED_SCENARIOS) != (args.transport == "tcp"):
        raise SystemExit(
            f"scenario {scenario} is not defined for transport {args.transport}"
        )

    raw_path = directory / "raw-wire.json"
    packet_path = directory / "packet-evidence.json"
    try:
        raw = json.loads(raw_path.read_text()) if raw_path.is_file() else {}
    except (OSError, json.JSONDecodeError):
        raw = {}
    try:
        packet = json.loads(packet_path.read_text()) if packet_path.is_file() else {}
    except (OSError, json.JSONDecodeError):
        packet = {}
    raw = raw if isinstance(raw, dict) else {}
    packet = packet if isinstance(packet, dict) else {}

    expected_raw_schema = UDP_RAW_SCHEMA if args.transport == "udp" else TCP_RAW_SCHEMA
    raw_path_flag = (
        raw.get("external_peer_path_observed")
        if args.transport == "udp"
        else raw.get("external_peer_exercised")
    )
    assertion(
        checks,
        "raw-wire-identity",
        raw.get("schema") == expected_raw_schema
        and raw.get("scenario") == scenario
        and raw.get("status") == "PASS"
        and raw.get("transport") == args.transport
        and raw_path_flag is True
        and not (directory / "raw-wire-error.txt").exists(),
        {
            "schema": raw.get("schema"),
            "scenario": raw.get("scenario"),
            "status": raw.get("status"),
            "transport": raw.get("transport"),
            "external_path": raw_path_flag,
            "error_artifact_present": (directory / "raw-wire-error.txt").exists(),
        },
    )
    packet_contract_checks(
        checks,
        packet,
        scenario=scenario,
        peer=args.peer,
        order=args.order,
        transport=args.transport,
        expected_rvoip=args.expected_rvoip_sent_by,
        expected_peer=args.expected_peer_sent_by,
    )

    uac_path = directory / "uac-messages.log"
    uas_path = directory / "uas-messages.log"
    uac = read_messages(uac_path)
    uas = read_messages(uas_path)
    assertion(
        checks,
        "raw-message-logs-present",
        uac_path.is_file()
        and not uac_path.is_symlink()
        and uac_path.stat().st_size > 0
        and uas_path.is_file()
        and not uas_path.is_symlink()
        and uas_path.stat().st_size > 0
        and bool(uac)
        and bool(uas),
        {"uac_messages": len(uac), "uas_messages": len(uas)},
    )
    packet_call_ids = packet.get("selected_call_ids")
    packet_call_ids = (
        set(packet_call_ids) if isinstance(packet_call_ids, list) else set()
    )
    wire_call_ids = {
        call_id(item)
        for item in scenario_messages([*uac, *uas], scenario)
        if call_id(item)
    }
    assertion(
        checks,
        "packet-call-ids-match-raw-wire",
        bool(wire_call_ids) and packet_call_ids == wire_call_ids,
        {
            "packet": sorted(packet_call_ids),
            "raw_wire": sorted(wire_call_ids),
        },
    )
    valid_paths, path_observations = messages_with_external_path(
        uas,
        scenario,
        args.order,
        args.expected_rvoip_sent_by,
        args.expected_peer_sent_by,
    )
    assertion(
        checks,
        "external-peer-and-rvoip-vias-on-wire",
        bool(path_observations)
        and len(valid_paths) == len(path_observations)
        and all(item["passed"] for item in path_observations),
        path_observations,
    )
    if args.transport == "udp":
        validate_udp_raw_contract(
            checks,
            scenario=scenario,
            directory=directory,
            raw=raw,
            packet=packet,
            uac=uac,
            uas=uas,
        )
    else:
        validate_tcp_raw_contract(
            checks,
            scenario=scenario,
            directory=directory,
            raw=raw,
            uac=uac,
            uas=uas,
            order=args.order,
            expected_rvoip=args.expected_rvoip_sent_by,
            expected_peer=args.expected_peer_sent_by,
        )

    inputs: dict[str, dict[str, Any]] = {}
    for path in sorted(directory.glob("*-messages.log")):
        if path.is_file() and not path.is_symlink() and path.stat().st_size > 0:
            inputs[path.name] = {"sha256": sha256(path), "bytes": path.stat().st_size}
    for path in (raw_path, packet_path):
        if path.is_file() and not path.is_symlink() and path.stat().st_size > 0:
            inputs[path.name] = {"sha256": sha256(path), "bytes": path.stat().st_size}
    passed = bool(checks) and all(item["passed"] for item in checks)
    payload = {
        "schema": SCHEMA,
        "scenario": scenario,
        "status": "PASS" if passed else "FAIL",
        "evidence_kind": "external-raw-wire-and-packet-observation",
        "external_peer_exercised": (
            bool(path_observations)
            and len(valid_paths) == len(path_observations)
            and all(item["passed"] for item in path_observations)
        ),
        "peer": args.peer,
        "order": args.order,
        "transport": args.transport,
        "assertions": checks,
        "inputs": inputs,
    }
    (directory / "result.json").write_text(
        json.dumps(payload, indent=2, sort_keys=True) + "\n"
    )
    return 0 if passed else 1


def validate_sipp(args: argparse.Namespace) -> int:
    directory = args.directory
    uac = read_messages(directory / "uac-messages.log")
    uas = read_messages(directory / "uas-messages.log")
    checks: list[dict[str, Any]] = []
    scenario = args.scenario
    packet_path = directory / "packet-evidence.json"
    packet = json.loads(packet_path.read_text()) if packet_path.is_file() else {}
    raw_wire_path = directory / "raw-wire.json"
    raw_wire = json.loads(raw_wire_path.read_text()) if raw_wire_path.is_file() else {}
    packet_assertions = {
        item.get("name"): item
        for item in packet.get("assertions", [])
        if isinstance(item, dict) and isinstance(item.get("name"), str)
    }
    required_packet_assertions = {
        "scenario-call-id-observed",
        "required-methods-observed",
        "required-statuses-observed",
        "rvoip-via-observed",
        "peer-via-observed",
    } | SCENARIO_PACKET_ASSERTIONS.get(scenario, set())
    assertion(
        checks,
        "structured-packet-evidence-passed",
        packet.get("schema") == "rvoip-sip-proxy-interop-packet-evidence-v1"
        and packet.get("scenario") == scenario
        and packet.get("status") == "PASS"
        and bool(packet.get("captures"))
        and all(
            isinstance(item.get("sha256"), str)
            and re.fullmatch(r"[0-9a-f]{64}", item["sha256"])
            and isinstance(item.get("bytes"), int)
            and item["bytes"] > 24
            for item in packet.get("captures", [])
        )
        and required_packet_assertions <= set(packet_assertions)
        and all(
            packet_assertions[name].get("passed") is True
            for name in required_packet_assertions
        ),
        {
            "schema": packet.get("schema"),
            "scenario": packet.get("scenario"),
            "status": packet.get("status"),
            "capture_count": len(packet.get("captures", [])),
            "assertion_names": sorted(packet_assertions),
        },
    )
    rvoip_via_observed = contains_via_sent_by(
        uas, "received", args.expected_rvoip_sent_by
    )
    peer_via_observed = contains_via_sent_by(
        uas, "received", args.expected_peer_sent_by
    )
    assertion(
        checks,
        "rvoip-via-observed-at-downstream-endpoint",
        rvoip_via_observed,
        args.expected_rvoip_sent_by,
    )
    assertion(
        checks,
        "external-peer-via-observed-at-downstream-endpoint",
        peer_via_observed,
        args.expected_peer_sent_by,
    )

    if scenario == "via-response-destination" and args.transport == "udp":
        probe_path = directory / "udp-via-probe.json"
        probe = json.loads(probe_path.read_text()) if probe_path.is_file() else {}
        assertion(checks, "udp-probe-passed", probe.get("status") == "PASS", probe)
        assertion(
            checks,
            "response-used-via-sent-by",
            probe.get("response", {}).get("received_on_via_sent_by") is True,
            probe.get("response", {}),
        )
        assertion(
            checks,
            "response-did-not-use-packet-source",
            probe.get("response", {}).get("received_on_packet_source") is False,
            probe.get("response", {}),
        )
        assertion(
            checks,
            "no-rport-request-reached-uas",
            request_count(uas, "received", "OPTIONS") >= 1
            and contains(
                uas, "received", "X-Interop-Scenario: via-response-destination"
            )
            and via_sent_by_port_omits_rport(
                uas, "received", probe.get("request", {}).get("via_sent_by_port", 0)
            ),
            {
                "uas_options": request_count(uas, "received", "OPTIONS"),
                "via_sent_by_port": probe.get("request", {}).get("via_sent_by_port"),
            },
        )
    elif scenario in {"options-readiness", "via-response-destination"}:
        assertion(
            checks,
            "options-reached-uas",
            request_count(uas, "received", "OPTIONS") >= 1,
            request_count(uas, "received", "OPTIONS"),
        )
        assertion(
            checks,
            "options-200-reached-uac",
            response_count(uac, "received", 200, "OPTIONS") >= 1,
            response_count(uac, "received", 200, "OPTIONS"),
        )
        if scenario == "via-response-destination":
            omits_rport, observation = originating_vias_omit_rport(
                uas,
                "received",
                "OPTIONS",
                scenario,
                {
                    args.expected_rvoip_sent_by,
                    args.expected_peer_sent_by,
                },
            )
            assertion(
                checks,
                "top-via-omits-rport",
                omits_rport,
                observation,
            )
    elif scenario == "invite-success":
        for method in ("INVITE", "ACK", "BYE"):
            assertion(
                checks,
                f"{method.lower()}-reached-uas",
                request_count(uas, "received", method) >= 1,
                request_count(uas, "received", method),
            )
        assertion(
            checks,
            "ringing-reached-uac",
            response_count(uac, "received", 180, "INVITE") >= 1,
            response_count(uac, "received", 180, "INVITE"),
        )
        assertion(
            checks,
            "invite-200-reached-uac",
            response_count(uac, "received", 200, "INVITE") >= 1,
            response_count(uac, "received", 200, "INVITE"),
        )
        assertion(
            checks,
            "bye-200-reached-uac",
            response_count(uac, "received", 200, "BYE") >= 1,
            response_count(uac, "received", 200, "BYE"),
        )
        assertion(
            checks,
            "request-sdp-preserved",
            contains(uas, "received", "m=audio 40000 RTP/AVP 0"),
            "expected SDP line at UAS",
        )
        assertion(
            checks,
            "response-sdp-preserved",
            contains(uac, "received", "m=audio 40002 RTP/AVP 0"),
            "expected SDP line at UAC",
        )
    elif scenario in {
        "matched-cancel-before-provisional",
        "matched-cancel-after-provisional",
        "cancel-retransmission",
    }:
        expected_downstream_cancel = 1
        observed_downstream_cancel = request_count(uas, "received", "CANCEL")
        assertion(
            checks,
            "one-downstream-cancel",
            observed_downstream_cancel == expected_downstream_cancel,
            observed_downstream_cancel,
        )
        assertion(
            checks,
            "cancel-200-reached-uac",
            response_count(uac, "received", 200, "CANCEL") >= 1,
            response_count(uac, "received", 200, "CANCEL"),
        )
        assertion(
            checks,
            "invite-487-reached-uac",
            response_count(uac, "received", 487, "INVITE") >= 1,
            response_count(uac, "received", 487, "INVITE"),
        )
        assertion(
            checks,
            "non2xx-ack-reached-uas-once",
            request_count(uas, "received", "ACK") == 1,
            request_count(uas, "received", "ACK"),
        )
        if scenario == "cancel-retransmission":
            validate_cancel_retransmission_contract(
                checks,
                transport=args.transport,
                raw_wire=raw_wire,
                uac=uac,
            )
        elif scenario == "matched-cancel-before-provisional":
            assertion(
                checks,
                "downstream-cancel-waited-for-provisional",
                raw_wire.get("status") == "PASS"
                and raw_wire.get("ordering") == "before"
                and raw_wire.get("downstream_cancel_before_provisional") is False,
                raw_wire,
            )
        elif scenario == "matched-cancel-after-provisional":
            assertion(
                checks,
                "downstream-cancel-followed-provisional",
                raw_wire.get("status") == "PASS"
                and raw_wire.get("ordering") == "after"
                and raw_wire.get("upstream_provisional_responses", 0) >= 1,
                raw_wire,
            )
    elif scenario == "unmatched-cancel":
        assertion(
            checks,
            "unmatched-cancel-reached-uas",
            request_count(uas, "received", "CANCEL") >= 1,
            request_count(uas, "received", "CANCEL"),
        )
        assertion(
            checks,
            "uas-481-reached-uac",
            response_count(uac, "received", 481, "CANCEL") >= 1
            and contains(uac, "received", "X-Interop-Origin: uas"),
            response_count(uac, "received", 481, "CANCEL"),
        )
    elif scenario == "ack-non2xx":
        assertion(
            checks,
            "busy-reached-uac",
            response_count(uac, "received", 486, "INVITE") >= 1,
            response_count(uac, "received", 486, "INVITE"),
        )
        assertion(
            checks,
            "one-transaction-ack-reached-uas",
            request_count(uas, "received", "ACK") == 1,
            request_count(uas, "received", "ACK"),
        )
    elif scenario == "message-body-content-length":
        request_body = "bridgefu-interop-request-body-0123456789"
        response_body = "bridgefu-interop-response-body-9876543210"
        assertion(
            checks,
            "message-reached-uas",
            request_count(uas, "received", "MESSAGE") >= 1,
            request_count(uas, "received", "MESSAGE"),
        )
        assertion(
            checks,
            "request-body-preserved",
            contains(uas, "received", request_body),
            request_body,
        )
        assertion(
            checks,
            "response-body-preserved",
            contains(uac, "received", response_body),
            response_body,
        )
        for label, path, body in (
            ("request", directory / "uas-messages.log", request_body + "\r\n"),
            ("response", directory / "uac-messages.log", response_body + "\r\n"),
        ):
            exact, observed = exact_body_matches(path, body.encode("ascii"))
            assertion(checks, f"{label}-content-length-exact", exact, observed)
    else:
        raise SystemExit(f"unsupported external SIPp scenario: {scenario}")

    inputs = {}
    for name in (
        "uac-messages.log",
        "uas-messages.log",
        "uac-stats.csv",
        "uas-stats.csv",
        "udp-via-probe.json",
        "raw-wire.json",
        "packet-evidence.json",
    ):
        path = directory / name
        if path.is_file() and path.stat().st_size:
            inputs[name] = {"sha256": sha256(path), "bytes": path.stat().st_size}
    passed = bool(checks) and all(item["passed"] for item in checks)
    payload = {
        "schema": SCHEMA,
        "scenario": scenario,
        "status": "PASS" if passed else "FAIL",
        "evidence_kind": "external-sipp-and-packet-observation",
        "external_peer_exercised": rvoip_via_observed and peer_via_observed,
        "peer": args.peer,
        "order": args.order,
        "transport": args.transport,
        "assertions": checks,
        "inputs": inputs,
    }
    (directory / "result.json").write_text(
        json.dumps(payload, indent=2, sort_keys=True) + "\n"
    )
    return 0 if passed else 1


def validate_supplemental(args: argparse.Namespace) -> int:
    commands = []
    status = "PASS"
    for raw_case in args.case:
        binary, separator, test_filter = raw_case.partition("=")
        if not separator or not binary or not test_filter:
            raise SystemExit(
                f"invalid --case {raw_case!r}; expected binary=test_filter"
            )
        command = [
            "cargo",
            "test",
            "--package",
            "rvoip-sip-proxy",
            "--test",
            binary,
            test_filter,
            "--",
            "--exact",
            "--test-threads=1",
        ]
        completed = subprocess.run(
            command,
            cwd=args.workspace_root,
            env=os.environ.copy(),
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            check=False,
        )
        log_name = f"{len(commands) + 1:02d}-{binary}-{test_filter}.log"
        (args.directory / log_name).write_text(completed.stdout)
        test_passed = (
            completed.returncode == 0
            and re.search(r"test result: ok\. 1 passed; 0 failed", completed.stdout)
            is not None
        )
        if not test_passed:
            status = "FAIL"
        commands.append(
            {
                "argv": command,
                "exit_code": completed.returncode,
                "one_exact_test_passed": test_passed,
                "log": log_name,
                "log_sha256": sha256(args.directory / log_name),
            }
        )
    payload = {
        "schema": SCHEMA,
        "scenario": args.scenario,
        "status": status,
        "evidence_kind": "in-process-rust-conformance-test",
        "external_peer_exercised": False,
        "limitation": (
            "This race/fork/policy scenario is deterministic in-process evidence; "
            "the same row's core scenarios exercise the named external peer."
        ),
        "peer_row": {
            "peer": args.peer,
            "order": args.order,
            "transport": args.transport,
        },
        "source": {
            "sha": args.source_sha,
            "fingerprint_sha256": args.source_fingerprint,
        },
        "commands": commands,
    }
    (args.directory / "result.json").write_text(
        json.dumps(payload, indent=2, sort_keys=True) + "\n"
    )
    return 0 if status == "PASS" else 1


def validate_tls(args: argparse.Namespace) -> int:
    row = args.row_directory
    directory = args.directory
    checks: list[dict[str, Any]] = []
    verifier_result_path = row / "tls-verifier-result.json"
    verifier_result = json.loads(verifier_result_path.read_text())
    if (
        verifier_result.get("schema") != "rvoip-sip-proxy-interop-tls-evidence-v2"
        or verifier_result.get("result") != "PASS"
        or verifier_result.get("peer") != args.peer
        or verifier_result.get("order") != args.order
        or verifier_result.get("transport") != "tls"
    ):
        raise SystemExit("TLS verifier result does not match this interoperability row")

    required = [
        "tls-verifier-result.json",
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
        f"tls-{args.peer}-outbound-boundary.log",
        "peer-runtime.json",
    ]
    if args.peer == "opensips":
        required.append("opensips-tls-image-provenance.json")

    inputs: dict[str, dict[str, Any]] = {}
    for name in required:
        source = row / name
        if not source.is_file() or source.is_symlink() or source.stat().st_size == 0:
            raise SystemExit(f"required TLS evidence is missing or invalid: {source}")
        destination = directory / name
        shutil.copyfile(source, destination)
        inputs[name] = {
            "sha256": sha256(destination),
            "bytes": destination.stat().st_size,
        }

    live_inputs = (
        "packet-evidence.json",
        "uac-messages.log",
        "uas-messages.log",
        "uac-stats.csv",
        "uas-stats.csv",
    )
    for name in live_inputs:
        path = directory / name
        if not path.is_file() or path.is_symlink() or path.stat().st_size == 0:
            raise SystemExit(
                f"actual SIPS scenario evidence is missing or invalid: {path}"
            )
        inputs[name] = {
            "sha256": sha256(path),
            "bytes": path.stat().st_size,
        }

    verifier_source = Path(__file__).with_name("verify_tls_evidence.py")
    if (
        not verifier_source.is_file()
        or verifier_source.is_symlink()
        or verifier_source.stat().st_size == 0
    ):
        raise SystemExit(f"TLS verifier source is missing: {verifier_source}")
    verifier_copy = directory / verifier_source.name
    shutil.copyfile(verifier_source, verifier_copy)
    inputs[verifier_copy.name] = {
        "sha256": sha256(verifier_copy),
        "bytes": verifier_copy.stat().st_size,
    }

    packet = json.loads((directory / "packet-evidence.json").read_text())
    packet_contract_checks(
        checks,
        packet,
        scenario="sips-routing",
        peer=args.peer,
        order=args.order,
        transport="tls",
        expected_rvoip=args.expected_rvoip_sent_by,
        expected_peer=args.expected_peer_sent_by,
    )
    packet_assertions = {
        item.get("name"): item
        for item in packet.get("assertions", [])
        if isinstance(item, dict) and isinstance(item.get("name"), str)
    }
    packet_captures = packet.get("captures")
    packet_captures = packet_captures if isinstance(packet_captures, list) else []
    assertion(
        checks,
        "scenario-owned-packet-capture",
        bool(packet_captures)
        and all(
            isinstance(item, dict)
            and isinstance(item.get("filename"), str)
            and item["filename"].startswith("sips-routing--")
            and item["filename"].endswith(".pcap")
            for item in packet_captures
        ),
        [item.get("filename") for item in packet_captures if isinstance(item, dict)],
    )

    uac_messages = read_messages(directory / "uac-messages.log")
    selected_call_ids = {
        value
        for value in packet.get("selected_call_ids", [])
        if isinstance(value, str) and value
    }
    uac = [
        item
        for item in uac_messages
        if call_id(item) in selected_call_ids
    ] or correlated_scenario_messages(uac_messages, "sips-routing")
    uas = correlated_scenario_messages(
        read_messages(directory / "uas-messages.log"), "sips-routing"
    )
    uac_requests = request_messages(uac, "sent", "OPTIONS")
    uas_requests = request_messages(uas, "received", "OPTIONS")
    expected_request_line = "OPTIONS sips:probe@example.test SIP/2.0"
    expected_to = "<sips:probe@example.test>"
    valid_uac_requests = [
        item
        for item in uac_requests
        if item.start_line == expected_request_line
        and first_header(item, "To").lower() == expected_to
        and first_header(item, "Contact").lower().startswith("<sips:uac@")
    ]
    valid_uas_requests = [
        item
        for item in uas_requests
        if item.start_line == expected_request_line
        and first_header(item, "To").lower() == expected_to
        and first_header(item, "Contact").lower().startswith("<sips:uac@")
    ]
    uac_boundary = packet_assertions.get("sips-request-uri-at-uac-boundary", {})
    preserved_request = packet_assertions.get(
        "sips-request-preserved-end-to-end", {}
    )
    assertion(
        checks,
        "actual-sips-request-on-boundary-plaintext",
        uac_boundary.get("passed") is True and len(valid_uas_requests) == 1,
        {
            "uac_start_lines": [item.start_line for item in uac_requests],
            "uas_start_lines": [item.start_line for item in uas_requests],
            "packet_uac_boundary": uac_boundary.get("observed"),
        },
    )
    assertion(
        checks,
        "sips-request-uri-preserved",
        preserved_request.get("passed") is True
        and len(valid_uas_requests) == 1
        and valid_uas_requests[0].start_line == expected_request_line,
        expected_request_line,
    )
    valid_paths, path_observations = messages_with_external_path(
        uas,
        "sips-routing",
        args.order,
        args.expected_rvoip_sent_by,
        args.expected_peer_sent_by,
    )
    assertion(
        checks,
        "both-real-proxy-vias",
        len(valid_paths) == 1
        and len(path_observations) == 1
        and path_observations[0]["passed"],
        path_observations,
    )
    assertion(
        checks,
        "sipp-full-path-success",
        response_count(uac, "received", 200, "OPTIONS") == 1
        and response_count(uas, "sent", 200, "OPTIONS") == 1
        and any(
            first_header(item, "Contact").lower().startswith("<sips:")
            for item in response_messages(uac, "received", 200, "OPTIONS")
        ),
        {
            "uac_200": response_count(uac, "received", 200, "OPTIONS"),
            "uas_200": response_count(uas, "sent", 200, "OPTIONS"),
        },
    )
    external_tls_assertion = packet_assertions.get(
        "no-plaintext-sip-on-external-tls-ports", {}
    )
    assertion(
        checks,
        "external-proxy-hops-mtls-only",
        external_tls_assertion.get("passed") is True
        and external_tls_assertion.get("observed") == 0
        and packet.get("insecure_external_sip_packet_count") == 0,
        {
            "packet_assertion": external_tls_assertion,
            "insecure_external_sip_packet_count": packet.get(
                "insecure_external_sip_packet_count"
            ),
        },
    )
    assertion(
        checks,
        "independent-tls-verifier",
        verifier_result.get("result") == "PASS",
        "positive and negative controls passed",
    )
    passed = bool(checks) and all(item["passed"] for item in checks)
    payload = {
        "schema": SCHEMA,
        "scenario": "sips-routing",
        "status": "PASS" if passed else "FAIL",
        "evidence_kind": "verified-external-tls",
        "external_peer_exercised": bool(valid_paths)
        and len(valid_paths) == len(path_observations)
        and all(item["passed"] for item in path_observations),
        "peer": args.peer,
        "order": args.order,
        "transport": "tls",
        "assertions": checks,
        "inputs": inputs,
    }
    (directory / "result.json").write_text(
        json.dumps(payload, indent=2, sort_keys=True) + "\n"
    )
    return 0 if passed else 1


def build_parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    subparsers = result.add_subparsers(dest="command", required=True)

    sipp = subparsers.add_parser("sipp")
    sipp.add_argument("--scenario", required=True)
    sipp.add_argument("--directory", required=True, type=Path)
    sipp.add_argument("--peer", required=True)
    sipp.add_argument("--order", required=True)
    sipp.add_argument("--transport", required=True)
    sipp.add_argument("--expected-rvoip-sent-by", required=True)
    sipp.add_argument("--expected-peer-sent-by", required=True)
    sipp.set_defaults(func=validate_sipp)

    raw = subparsers.add_parser("raw")
    raw.add_argument("--scenario", required=True, choices=sorted(ADVANCED_SCENARIOS))
    raw.add_argument("--directory", required=True, type=Path)
    raw.add_argument("--peer", required=True, choices=("kamailio", "opensips"))
    raw.add_argument("--order", required=True, choices=("rvoip-first", "peer-first"))
    raw.add_argument("--transport", required=True, choices=("udp", "tcp"))
    raw.add_argument("--expected-rvoip-sent-by", required=True)
    raw.add_argument("--expected-peer-sent-by", required=True)
    raw.set_defaults(func=validate_raw)

    supplemental = subparsers.add_parser("supplemental")
    supplemental.add_argument("--scenario", required=True)
    supplemental.add_argument("--directory", required=True, type=Path)
    supplemental.add_argument("--peer", required=True)
    supplemental.add_argument("--order", required=True)
    supplemental.add_argument("--transport", required=True)
    supplemental.add_argument("--source-sha", required=True)
    supplemental.add_argument("--source-fingerprint", required=True)
    supplemental.add_argument("--workspace-root", required=True, type=Path)
    supplemental.add_argument("--case", action="append", required=True)
    supplemental.set_defaults(func=validate_supplemental)

    tls = subparsers.add_parser("tls")
    tls.add_argument("--directory", required=True, type=Path)
    tls.add_argument("--row-directory", required=True, type=Path)
    tls.add_argument("--peer", required=True, choices=("kamailio", "opensips"))
    tls.add_argument("--order", required=True, choices=("rvoip-first", "peer-first"))
    tls.add_argument("--expected-rvoip-sent-by", required=True)
    tls.add_argument("--expected-peer-sent-by", required=True)
    tls.set_defaults(func=validate_tls)
    return result


def main() -> int:
    args = build_parser().parse_args()
    args.directory.mkdir(parents=True, exist_ok=True)
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
