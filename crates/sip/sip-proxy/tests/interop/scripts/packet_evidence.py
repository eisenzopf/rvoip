#!/usr/bin/env python3
"""Derive structured, scenario-bound packet evidence with TShark."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any


SCHEMA = "rvoip-sip-proxy-interop-packet-evidence-v1"
DNS_QUERY_LOG_SCHEMA = "rvoip-sip-proxy-rfc3263-dns-v1"
SIP_FIELDS = (
    "frame.number",
    "frame.time_epoch",
    "ip.src",
    "ipv6.src",
    "udp.srcport",
    "tcp.srcport",
    "ip.dst",
    "ipv6.dst",
    "udp.dstport",
    "tcp.dstport",
    "sip.Method",
    "sip.Status-Code",
    "sip.Call-ID",
    "sip.CSeq.method",
    "sip.Via.sent-by.address",
    "sip.Via.sent-by.port",
    "sip.Via.branch",
    "sip.Request-Line",
    "sip.r-uri",
    "sip.Route.uri",
    "sip.Record-Route.uri",
    "sip.contact.uri",
    "sip.to.tag",
    "sip.msg_hdr",
)
TCP_FIELDS = (
    "frame.number",
    "frame.time_epoch",
    "ip.src",
    "ipv6.src",
    "tcp.srcport",
    "ip.dst",
    "ipv6.dst",
    "tcp.dstport",
    "tcp.flags.syn",
    "tcp.flags.ack",
    "tcp.flags.reset",
)
DNS_FIELDS = (
    "frame.number",
    "frame.time_epoch",
    "ip.src",
    "ipv6.src",
    "udp.srcport",
    "ip.dst",
    "ipv6.dst",
    "udp.dstport",
    "dns.flags.response",
    "dns.qry.name",
    "dns.qry.type",
    "dns.srv.priority",
    "dns.srv.port",
    "dns.srv.target",
    "dns.a",
)
TLS_FIELDS = (
    "frame.number",
    "frame.time_epoch",
    "ip.src",
    "ipv6.src",
    "tcp.srcport",
    "ip.dst",
    "ipv6.dst",
    "tcp.dstport",
    "tls.record.content_type",
    "tls.handshake.type",
    "tls.handshake.version",
    "tls.handshake.extensions_server_name",
    "tls.handshake.certificate",
    "tls.alert_message.level",
    "tls.alert_message.desc",
)


class EvidenceError(RuntimeError):
    """Raised when a capture cannot prove the requested scenario."""


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def tshark_version(binary: str) -> dict[str, str]:
    result = subprocess.run(
        [binary, "--version"], text=True, capture_output=True, check=False
    )
    if result.returncode:
        raise EvidenceError(result.stderr.strip() or "tshark --version failed")
    lines = result.stdout.splitlines()
    pcap = next((line.strip() for line in lines if "libpcap " in line), "")
    return {"tshark": lines[0].strip() if lines else "", "libpcap": pcap}


def run_fields(
    binary: str, pcap: Path, display_filter: str, fields: tuple[str, ...]
) -> list[dict[str, str]]:
    command = [binary]
    if display_filter == "sip":
        # These scenario captures are already bounded to the configured test
        # ports and every SIP assertion is correlated by an opaque Call-ID
        # carrying the scenario marker. Force decode here because TShark has
        # unrelated heuristic dissectors on arbitrary high UDP ports (for
        # example MANOLITO) that otherwise hide valid SIP packets when a test
        # selects such a port. TLS and DNS analysis use independent invocations
        # without these decode-as selectors.
        command.extend(
            [
                "-d",
                "udp.port==1-65535,sip",
                "-d",
                "tcp.port==1-65535,sip",
            ]
        )
    command.extend(
        [
            "-r",
            str(pcap),
            "-Y",
            display_filter,
            "-T",
            "fields",
            "-E",
            "header=y",
            "-E",
            "separator=,",
            "-E",
            "quote=d",
            "-E",
            "occurrence=a",
            "-E",
            "aggregator=|",
        ]
    )
    for field in fields:
        command.extend(["-e", field])
    result = subprocess.run(command, text=True, capture_output=True, check=False)
    if result.returncode:
        raise EvidenceError(
            f"tshark failed for {pcap.name}: {result.stderr.strip() or result.stdout.strip()}"
        )
    return list(csv.DictReader(result.stdout.splitlines()))


def split_occurrences(value: str | None) -> set[str]:
    return {item for item in (value or "").split("|") if item}


def occurrence_list(value: str | None) -> list[str]:
    return [item for item in (value or "").split("|") if item]


def endpoint(record: dict[str, str], side: str) -> tuple[str, str]:
    address = record.get(f"ip.{side}") or record.get(f"ipv6.{side}") or ""
    port = record.get(f"udp.{side}port") or record.get(f"tcp.{side}port") or ""
    return address, port


def check(
    assertions: list[dict[str, Any]], name: str, passed: bool, observed: Any
) -> None:
    assertions.append({"name": name, "passed": bool(passed), "observed": observed})


def collect_fields(
    binary: str,
    captures: list[dict[str, Any]],
    display_filter: str,
    fields: tuple[str, ...],
) -> list[dict[str, str]]:
    rows: list[dict[str, str]] = []
    for capture in captures:
        capture_rows = run_fields(binary, Path(capture["path"]), display_filter, fields)
        for row in capture_rows:
            row["capture"] = Path(capture["path"]).name
        rows.extend(capture_rows)
    return rows


def frame_time(row: dict[str, str]) -> float | None:
    try:
        return float(row.get("frame.time_epoch", ""))
    except ValueError:
        return None


def request_has_method(row: dict[str, str], method: str) -> bool:
    return method in split_occurrences(row.get("sip.Method"))


def response_has_status(
    row: dict[str, str], status: int, method: str | None = None
) -> bool:
    statuses = split_occurrences(row.get("sip.Status-Code"))
    if str(status) not in statuses:
        return False
    return method is None or method in split_occurrences(row.get("sip.CSeq.method"))


def call_id(row: dict[str, str]) -> str:
    return row.get("sip.Call-ID", "")


def row_call_ids(row: dict[str, str]) -> set[str]:
    """Return every SIP message identity decoded from one packet row.

    TShark reports repeated SIP PDUs in a coalesced TCP frame as pipe-separated
    field occurrences. Treating that field as one opaque Call-ID loses the
    individual messages and can make complete burst traffic look incomplete.
    """
    return split_occurrences(row.get("sip.Call-ID"))


def call_ids_for_request(row: dict[str, str], method: str) -> set[str]:
    identities = occurrence_list(row.get("sip.Call-ID"))
    methods = occurrence_list(row.get("sip.Method"))
    if len(identities) == len(methods):
        return {
            identity
            for identity, observed_method in zip(identities, methods, strict=True)
            if observed_method == method
        }
    if len(identities) == 1 and method in methods:
        return set(identities)
    return set()


def call_ids_for_response(
    row: dict[str, str], status: int, method: str
) -> set[str]:
    identities = occurrence_list(row.get("sip.Call-ID"))
    statuses = occurrence_list(row.get("sip.Status-Code"))
    cseq_methods = occurrence_list(row.get("sip.CSeq.method"))
    if len(identities) == len(statuses) == len(cseq_methods):
        return {
            identity
            for identity, observed_status, observed_method in zip(
                identities, statuses, cseq_methods, strict=True
            )
            if observed_status == str(status) and observed_method == method
        }
    if (
        len(identities) == 1
        and str(status) in statuses
        and method in cseq_methods
    ):
        return set(identities)
    return set()


def call_ids_with_status(
    rows: list[dict[str, str]], status: int, method: str
) -> set[str]:
    return set().union(
        *(call_ids_for_response(row, status, method) for row in rows)
    )


def row_has_via(row: dict[str, str], address: str, port: int) -> bool:
    return address in split_occurrences(row.get("sip.Via.sent-by.address")) and str(
        port
    ) in split_occurrences(row.get("sip.Via.sent-by.port"))


def downstream_requests(
    rows: list[dict[str, str]], args: argparse.Namespace, method: str
) -> list[dict[str, str]]:
    return [
        row
        for row in rows
        if request_has_method(row, method)
        and row_has_via(row, args.rvoip_address, args.rvoip_port)
        and row_has_via(row, args.peer_address, args.peer_port)
    ]


def proxy_generated_downstream_requests(
    rows: list[dict[str, str]],
    args: argparse.Namespace,
    method: str,
    target_port: int,
) -> list[dict[str, str]]:
    expected_destination = (args.rvoip_address, str(target_port))
    expected_via_addresses = [args.rvoip_address]
    expected_via_ports = [str(args.rvoip_port)]
    return [
        row
        for row in rows
        if request_has_method(row, method)
        and endpoint(row, "src")[0] == args.rvoip_address
        and endpoint(row, "dst") == expected_destination
        and occurrence_list(row.get("sip.Via.sent-by.address"))
        == expected_via_addresses
        and occurrence_list(row.get("sip.Via.sent-by.port")) == expected_via_ports
    ]


def first_time(rows: list[dict[str, str]]) -> float | None:
    values = [value for row in rows if (value := frame_time(row)) is not None]
    return min(values) if values else None


def canonical_uri(value: str) -> str:
    value = value.strip()
    if value.startswith("<") and value.endswith(">"):
        value = value[1:-1]
    return re.sub(r"\s+", "", value).lower()


def uri_occurrences(row: dict[str, str], field: str) -> list[str]:
    return [
        canonical_uri(value)
        for value in occurrence_list(row.get(field))
        if canonical_uri(value)
    ]


def tcp_syn_rows_to(
    rows: list[dict[str, str]], address: str, port: int
) -> list[dict[str, str]]:
    return [
        row
        for row in rows
        if endpoint(row, "dst") == (address, str(port))
        and row.get("tcp.flags.syn", "").lower() in {"1", "true"}
        and row.get("tcp.flags.ack", "").lower() not in {"1", "true"}
    ]


def require_positive_port(args: argparse.Namespace, name: str) -> int:
    value = getattr(args, name, None)
    if not isinstance(value, int) or isinstance(value, bool) or not 0 < value <= 65535:
        raise EvidenceError(
            f"--{name.replace('_', '-')} is required for {args.scenario}"
        )
    return value


def check_timer_c_packet_contract(
    assertions: list[dict[str, Any]],
    rows: list[dict[str, str]],
    args: argparse.Namespace,
) -> None:
    configured_ms = getattr(args, "timer_c_ms", None)
    if (
        not isinstance(configured_ms, int)
        or isinstance(configured_ms, bool)
        or configured_ms <= 0
    ):
        raise EvidenceError("--timer-c-ms must be positive")
    invites = downstream_requests(rows, args, "INVITE")
    invite_call_ids = {call_id(row) for row in invites if call_id(row)}
    check(
        assertions,
        f"{args.scenario}-single-downstream-invite-call",
        len(invite_call_ids) == 1,
        sorted(invite_call_ids),
    )
    if args.scenario == "timer-c-calling":
        finals = [row for row in rows if response_has_status(row, 408, "INVITE")]
        final_call_ids = {call_id(row) for row in finals if call_id(row)}
        provisionals = [
            row
            for row in rows
            if any(
                str(status) in split_occurrences(row.get("sip.Status-Code"))
                for status in range(101, 200)
            )
            and "INVITE" in split_occurrences(row.get("sip.CSeq.method"))
        ]
        started = first_time(invites)
        completed = first_time(finals)
        elapsed_ms = (
            (completed - started) * 1000
            if started is not None and completed is not None
            else None
        )
        check(
            assertions,
            "timer-c-calling-invite-precedes-408",
            invite_call_ids == final_call_ids
            and len(final_call_ids) == 1
            and elapsed_ms is not None
            and elapsed_ms > 0,
            {
                "invite_call_ids": sorted(invite_call_ids),
                "final_call_ids": sorted(final_call_ids),
                "elapsed_ms": elapsed_ms,
            },
        )
        check(
            assertions,
            "timer-c-calling-zero-provisional-responses",
            not provisionals,
            len(provisionals),
        )
    else:
        provisionals = [row for row in rows if response_has_status(row, 180, "INVITE")]
        normal_target_port = require_positive_port(args, "normal_target_port")
        cancels = proxy_generated_downstream_requests(
            rows, args, "CANCEL", normal_target_port
        )
        provisional_call_ids = {call_id(row) for row in provisionals if call_id(row)}
        cancel_call_ids = {call_id(row) for row in cancels if call_id(row)}
        provisional_at = first_time(provisionals)
        cancel_at = first_time(cancels)
        elapsed_ms = (
            (cancel_at - provisional_at) * 1000
            if provisional_at is not None and cancel_at is not None
            else None
        )
        invite_branches = {
            occurrence_list(row.get("sip.Via.branch"))[0]
            for row in invites
            if occurrence_list(row.get("sip.Via.branch"))
        }
        cancel_branches = {
            occurrence_list(row.get("sip.Via.branch"))[0]
            for row in cancels
            if occurrence_list(row.get("sip.Via.branch"))
        }
        check(
            assertions,
            "timer-c-proceeding-180-precedes-cancel",
            invite_call_ids == provisional_call_ids == cancel_call_ids
            and len(cancel_call_ids) == 1
            and elapsed_ms is not None
            and elapsed_ms > 0,
            {
                "invite_call_ids": sorted(invite_call_ids),
                "provisional_call_ids": sorted(provisional_call_ids),
                "cancel_call_ids": sorted(cancel_call_ids),
                "elapsed_ms": elapsed_ms,
            },
        )
        check(
            assertions,
            "timer-c-proceeding-cancel-reuses-invite-branch",
            len(invite_branches) == 1 and invite_branches == cancel_branches,
            {
                "invite_branches": sorted(invite_branches),
                "cancel_branches": sorted(cancel_branches),
            },
        )
    minimum_ms = configured_ms * 0.5
    maximum_ms = configured_ms * 5.0
    check(
        assertions,
        f"{args.scenario}-packet-elapsed-within-bounds",
        elapsed_ms is not None and minimum_ms <= elapsed_ms <= maximum_ms,
        {
            "configured_ms": configured_ms,
            "elapsed_ms": elapsed_ms,
            "minimum_ms": minimum_ms,
            "maximum_ms": maximum_ms,
        },
    )


def check_transport_failure_packet_contract(
    assertions: list[dict[str, Any]],
    sip_rows: list[dict[str, str]],
    tcp_rows: list[dict[str, str]],
    args: argparse.Namespace,
) -> None:
    dead_port = require_positive_port(args, "dead_target_port")
    normal_port = require_positive_port(args, "normal_target_port")
    failed = call_ids_with_status(sip_rows, 500, "INVITE")
    dead_syns = tcp_syn_rows_to(tcp_rows, args.rvoip_address, dead_port)
    dead_requests = [
        row
        for row in sip_rows
        if call_id(row) in failed
        and request_has_method(row, "INVITE")
        and endpoint(row, "dst") == (args.rvoip_address, str(dead_port))
    ]
    normal_invites = [
        row
        for row in sip_rows
        if call_id(row) in failed
        and request_has_method(row, "INVITE")
        and endpoint(row, "dst") == (args.rvoip_address, str(normal_port))
    ]
    check(
        assertions,
        "transport-failure-single-500-upstream-invite-call",
        len(failed) == 1,
        sorted(failed),
    )
    check(
        assertions,
        "transport-failure-dead-endpoint-syn-observed",
        bool(dead_syns),
        {
            "expected": f"{args.rvoip_address}:{dead_port}",
            "frames": sorted(
                {
                    row.get("frame.number", "")
                    for row in dead_syns
                    if row.get("frame.number")
                }
            ),
        },
    )
    check(
        assertions,
        "transport-failure-dead-endpoint-received-zero-sip",
        len(failed) == 1 and not dead_requests,
        {
            "failed_call_ids": sorted(failed),
            "dead_target": f"{args.rvoip_address}:{dead_port}",
            "matching_invite_frames": [
                row.get("frame.number", "") for row in dead_requests
            ],
        },
    )
    check(
        assertions,
        "transport-failure-failed-call-never-reached-normal-target",
        len(failed) == 1 and not normal_invites,
        {
            "failed_call_ids": sorted(failed),
            "normal_target": f"{args.rvoip_address}:{normal_port}",
            "matching_invite_frames": [
                row.get("frame.number", "") for row in normal_invites
            ],
        },
    )


def normalized_dns_name(value: str) -> str:
    return value.rstrip(".").lower()


def authoritative_dns_query_pairs(
    path: Path | None,
) -> tuple[set[tuple[str, str]], dict[str, Any]]:
    if path is None:
        return set(), {"present": False}
    if not path.is_file() or path.is_symlink() or path.stat().st_size == 0:
        raise EvidenceError(f"invalid RFC 3263 DNS query log: {path}")

    pairs: set[tuple[str, str]] = set()
    row_count = 0
    with path.open(encoding="utf-8") as stream:
        for line_number, line in enumerate(stream, start=1):
            if not line.strip():
                continue
            try:
                row = json.loads(line)
            except json.JSONDecodeError as error:
                raise EvidenceError(
                    f"invalid RFC 3263 DNS query log JSON at line {line_number}: {error}"
                ) from error
            if row.get("schema") != DNS_QUERY_LOG_SCHEMA:
                raise EvidenceError(
                    f"invalid RFC 3263 DNS query log schema at line {line_number}"
                )
            query_name = row.get("query_name")
            query_type = row.get("query_type")
            if (
                not isinstance(query_name, str)
                or not isinstance(query_type, int)
                or isinstance(query_type, bool)
            ):
                raise EvidenceError(
                    f"invalid RFC 3263 DNS query identity at line {line_number}"
                )
            pairs.add((normalized_dns_name(query_name), str(query_type)))
            row_count += 1

    if row_count == 0:
        raise EvidenceError("RFC 3263 DNS query log contains no records")
    return pairs, {
        "present": True,
        "filename": path.name,
        "sha256": sha256(path),
        "bytes": path.stat().st_size,
        "row_count": row_count,
        "observed": sorted(pairs),
    }


def check_rfc3263_packet_contract(
    assertions: list[dict[str, Any]],
    sip_rows: list[dict[str, str]],
    tcp_rows: list[dict[str, str]],
    dns_rows: list[dict[str, str]],
    args: argparse.Namespace,
) -> None:
    dns_port = require_positive_port(args, "dns_port")
    dead_port = require_positive_port(args, "dead_target_port")
    live_port = require_positive_port(args, "live_target_port")
    zone = normalized_dns_name(args.rfc3263_zone)
    service = f"_sip._tcp.{zone}"
    dead_name = f"dead.{zone}"
    live_name = f"live.{zone}"
    queries = [
        row
        for row in dns_rows
        if endpoint(row, "dst") == (args.rvoip_address, str(dns_port))
        and row.get("dns.flags.response", "").lower() not in {"1", "true"}
    ]
    query_pairs = {
        (normalized_dns_name(name), query_type)
        for row in queries
        for name in occurrence_list(row.get("dns.qry.name"))
        for query_type in occurrence_list(row.get("dns.qry.type"))
    }
    authoritative_query_pairs, authoritative_query_log = authoritative_dns_query_pairs(
        getattr(args, "dns_query_log", None)
    )
    srv_answers: set[tuple[int, int, str]] = set()
    srv_answer_sequences: list[list[tuple[int, int, str]]] = []
    for row in dns_rows:
        priorities = occurrence_list(row.get("dns.srv.priority"))
        ports = occurrence_list(row.get("dns.srv.port"))
        targets = occurrence_list(row.get("dns.srv.target"))
        if len(priorities) != len(ports) or len(ports) != len(targets):
            continue
        sequence: list[tuple[int, int, str]] = []
        for priority, port, target in zip(priorities, ports, targets):
            if priority.isdigit() and port.isdigit():
                answer = (
                    int(priority),
                    int(port),
                    normalized_dns_name(target),
                )
                srv_answers.add(answer)
                sequence.append(answer)
        if sequence:
            srv_answer_sequences.append(sequence)
    expected_answers = {
        (10, dead_port, dead_name),
        (20, live_port, live_name),
    }
    dead_syns = tcp_syn_rows_to(tcp_rows, args.rvoip_address, dead_port)
    live_invites = [
        row
        for row in downstream_requests(sip_rows, args, "INVITE")
        if endpoint(row, "dst") == (args.rvoip_address, str(live_port))
    ]
    dead_invites = [
        row
        for row in sip_rows
        if request_has_method(row, "INVITE")
        and endpoint(row, "dst") == (args.rvoip_address, str(dead_port))
    ]
    check(
        assertions,
        "rfc3263-srv-query-observed",
        (service, "33") in query_pairs,
        {"required": [service, "33"], "observed": sorted(query_pairs)},
    )
    check(
        assertions,
        "rfc3263-exact-ordered-srv-answers",
        srv_answers == expected_answers
        and [
            (10, dead_port, dead_name),
            (20, live_port, live_name),
        ]
        in srv_answer_sequences,
        {
            "required": sorted(expected_answers),
            "observed": sorted(srv_answers),
            "wire_sequences": srv_answer_sequences,
        },
    )
    check(
        assertions,
        "rfc3263-both-candidate-a-queries-observed",
        {(dead_name, "1"), (live_name, "1")} <= query_pairs | authoritative_query_pairs,
        {
            "required": sorted({(dead_name, "1"), (live_name, "1")}),
            "packet_observed": sorted(query_pairs),
            "authoritative_server_log": authoritative_query_log,
            "combined_observed": sorted(query_pairs | authoritative_query_pairs),
        },
    )
    check(
        assertions,
        "rfc3263-dead-candidate-syn-observed",
        bool(dead_syns),
        {
            "expected": f"{args.rvoip_address}:{dead_port}",
            "frames": [row.get("frame.number", "") for row in dead_syns],
        },
    )
    check(
        assertions,
        "rfc3263-live-candidate-invite-observed",
        bool(live_invites),
        {
            "expected": f"{args.rvoip_address}:{live_port}",
            "call_ids": sorted({call_id(row) for row in live_invites if call_id(row)}),
        },
    )
    check(
        assertions,
        "rfc3263-dead-candidate-received-zero-invites",
        not dead_invites,
        [row.get("frame.number", "") for row in dead_invites],
    )


def check_routing_packet_contract(
    assertions: list[dict[str, Any]],
    rows: list[dict[str, str]],
    args: argparse.Namespace,
) -> None:
    required = {}
    for name in (
        "original_target_uri",
        "next_hop_route_uri",
        "record_route_uri",
        "local_route_uri",
    ):
        value = getattr(args, name, None)
        if not isinstance(value, str) or not value.strip():
            raise EvidenceError(
                f"--{name.replace('_', '-')} is required for {args.scenario}"
            )
        required[name] = canonical_uri(value)
    invites = downstream_requests(rows, args, "INVITE")
    request_uris = {
        canonical_uri(value)
        for row in invites
        for value in occurrence_list(row.get("sip.r-uri"))
        if canonical_uri(value)
    }
    downstream_routes = [uri_occurrences(row, "sip.Route.uri") for row in invites]
    downstream_record_routes = [
        uri_occurrences(row, "sip.Record-Route.uri") for row in invites
    ]
    responses = [row for row in rows if response_has_status(row, 200, "INVITE")]
    response_record_routes = [
        uri_occurrences(row, "sip.Record-Route.uri")
        for row in responses
        if uri_occurrences(row, "sip.Record-Route.uri")
    ]
    check(
        assertions,
        f"{args.scenario}-exact-downstream-request-uri",
        bool(invites) and request_uris == {required["original_target_uri"]},
        {
            "required": required["original_target_uri"],
            "observed": sorted(request_uris),
        },
    )
    check(
        assertions,
        f"{args.scenario}-exact-downstream-route-set",
        bool(downstream_routes)
        and all(
            routes == [required["next_hop_route_uri"]] for routes in downstream_routes
        ),
        {
            "required": [required["next_hop_route_uri"]],
            "observed": downstream_routes,
        },
    )
    check(
        assertions,
        f"{args.scenario}-local-route-removed",
        bool(downstream_routes)
        and all(
            required["local_route_uri"] not in routes for routes in downstream_routes
        ),
        {
            "removed": required["local_route_uri"],
            "observed": downstream_routes,
        },
    )
    check(
        assertions,
        f"{args.scenario}-record-route-round-trip",
        bool(downstream_record_routes)
        and bool(response_record_routes)
        and all(
            routes.count(required["record_route_uri"]) == 1
            for routes in downstream_record_routes
        )
        and all(
            routes.count(required["record_route_uri"]) == 1
            for routes in response_record_routes
        ),
        {
            "required": required["record_route_uri"],
            "downstream": downstream_record_routes,
            "upstream_responses": response_record_routes,
        },
    )


def check_capacity_packet_contract(
    assertions: list[dict[str, Any]],
    rows: list[dict[str, str]],
    args: argparse.Namespace,
) -> None:
    rvoip_overload_rows = [
        row
        for row in rows
        if response_has_status(row, 503, "INVITE")
        and endpoint(row, "src") == (args.rvoip_address, str(args.rvoip_port))
    ]
    overloaded = set().union(
        *(call_ids_for_response(row, 503, "INVITE") for row in rvoip_overload_rows)
    )
    expected_upstream_status = 500 if args.order == "peer-first" else 503
    upstream_overloaded = call_ids_with_status(rows, expected_upstream_status, "INVITE")
    busy = call_ids_with_status(rows, 486, "INVITE")
    forwarded = set().union(
        *(
            call_ids_for_request(row, "INVITE")
            for row in downstream_requests(rows, args, "INVITE")
        )
    )
    invite_calls = set().union(
        *(call_ids_for_request(row, "INVITE") for row in rows)
    )
    check(
        assertions,
        "capacity-overload-single-rejected-call",
        len(overloaded) == 1 and upstream_overloaded == overloaded,
        {
            "rvoip_boundary_503_call_ids": sorted(overloaded),
            "upstream_call_ids": sorted(upstream_overloaded),
        },
    )
    check(
        assertions,
        "capacity-overload-upstream-status-for-order",
        len(overloaded) == 1 and upstream_overloaded == overloaded,
        {
            "order": args.order,
            "expected_upstream_status": expected_upstream_status,
            "rvoip_boundary_503_call_ids": sorted(overloaded),
            "upstream_call_ids": sorted(upstream_overloaded),
        },
    )
    check(
        assertions,
        "capacity-overload-forwarded-calls-all-finished-486",
        bool(busy) and forwarded == busy,
        {
            "forwarded_call_ids": sorted(forwarded),
            "busy_call_ids": sorted(busy),
        },
    )
    check(
        assertions,
        "capacity-overload-rejected-call-had-zero-downstream-egress",
        len(overloaded) == 1 and overloaded.isdisjoint(forwarded),
        {
            "rejected_call_ids": sorted(overloaded),
            "forwarded_call_ids": sorted(forwarded),
        },
    )
    check(
        assertions,
        "capacity-overload-exact-final-call-partition",
        invite_calls == busy | overloaded
        and busy.isdisjoint(overloaded)
        and len(overloaded) == 1,
        {
            "invite_call_ids": sorted(invite_calls),
            "busy_call_ids": sorted(busy),
            "overloaded_call_ids": sorted(overloaded),
        },
    )


def check_multiple_2xx_ack_packet_contract(
    assertions: list[dict[str, Any]],
    rows: list[dict[str, str]],
    args: argparse.Namespace,
) -> None:
    responses = [row for row in rows if response_has_status(row, 200, "INVITE")]
    response_shapes: dict[str, set[tuple[str, tuple[str, ...]]]] = {}
    for row in responses:
        tags = occurrence_list(row.get("sip.to.tag"))
        contacts = uri_occurrences(row, "sip.contact.uri")
        routes = uri_occurrences(row, "sip.Record-Route.uri")
        if len(tags) == 1 and len(contacts) == 1:
            response_shapes.setdefault(tags[0], set()).add((contacts[0], tuple(routes)))

    upstream_ack_shapes: dict[str, set[tuple[str, tuple[str, ...]]]] = {}
    downstream_ack_shapes: dict[str, set[tuple[str, tuple[str, ...]]]] = {}
    for row in rows:
        if not request_has_method(row, "ACK"):
            continue
        tags = occurrence_list(row.get("sip.to.tag"))
        uris = uri_occurrences(row, "sip.r-uri")
        if len(tags) != 1 or len(uris) != 1:
            continue
        shape = (uris[0], tuple(uri_occurrences(row, "sip.Route.uri")))
        has_rvoip = row_has_via(row, args.rvoip_address, args.rvoip_port)
        has_peer = row_has_via(row, args.peer_address, args.peer_port)
        if not has_rvoip and not has_peer:
            upstream_ack_shapes.setdefault(tags[0], set()).add(shape)
        elif has_rvoip and has_peer:
            downstream_ack_shapes.setdefault(tags[0], set()).add(shape)

    response_contract = len(response_shapes) == 2 and all(
        len(shapes) == 1 and len(next(iter(shapes))[1]) >= 2
        for shapes in response_shapes.values()
    )
    uac_ack_contract = response_contract
    downstream_ack_contract = response_contract
    observations: dict[str, Any] = {}
    for tag, shapes in response_shapes.items():
        if len(shapes) != 1:
            uac_ack_contract = False
            downstream_ack_contract = False
            continue
        contact, response_routes = next(iter(shapes))
        expected_upstream = {(contact, tuple(reversed(response_routes)))}
        expected_downstream = {(contact, ())}
        observed_upstream = upstream_ack_shapes.get(tag, set())
        observed_downstream = downstream_ack_shapes.get(tag, set())
        uac_ack_contract = uac_ack_contract and observed_upstream == expected_upstream
        downstream_ack_contract = (
            downstream_ack_contract and observed_downstream == expected_downstream
        )
        observations[tag] = {
            "contact": contact,
            "response_record_routes": list(response_routes),
            "expected_uac_ack_routes": list(reversed(response_routes)),
            "observed_uac_ack_shapes": sorted(observed_upstream),
            "observed_downstream_ack_shapes": sorted(observed_downstream),
        }
    check(
        assertions,
        "multiple-2xx-two-dialog-contacts-and-record-route-sets",
        response_contract,
        observations,
    )
    check(
        assertions,
        "multiple-2xx-uac-acks-use-contact-and-reversed-route-set",
        uac_ack_contract,
        observations,
    )
    check(
        assertions,
        "multiple-2xx-downstream-acks-reach-contact-with-routes-consumed",
        downstream_ack_contract,
        observations,
    )


def check_invite_dialog_route_packet_contract(
    assertions: list[dict[str, Any]],
    rows: list[dict[str, str]],
    args: argparse.Namespace,
) -> None:
    responses = [row for row in rows if response_has_status(row, 200, "INVITE")]
    response_shapes: dict[str, set[tuple[str, tuple[str, ...]]]] = {}
    for row in responses:
        tags = occurrence_list(row.get("sip.to.tag"))
        contacts = uri_occurrences(row, "sip.contact.uri")
        routes = uri_occurrences(row, "sip.Record-Route.uri")
        if len(tags) == 1 and len(contacts) == 1:
            response_shapes.setdefault(tags[0], set()).add((contacts[0], tuple(routes)))

    upstream: dict[str, dict[str, set[tuple[str, tuple[str, ...]]]]] = {}
    downstream: dict[str, dict[str, set[tuple[str, tuple[str, ...]]]]] = {}
    for row in rows:
        method = next(
            (
                candidate
                for candidate in ("ACK", "BYE")
                if request_has_method(row, candidate)
            ),
            None,
        )
        if method is None:
            continue
        tags = occurrence_list(row.get("sip.to.tag"))
        uris = uri_occurrences(row, "sip.r-uri")
        if len(tags) != 1 or len(uris) != 1:
            continue
        shape = (uris[0], tuple(uri_occurrences(row, "sip.Route.uri")))
        has_rvoip = row_has_via(row, args.rvoip_address, args.rvoip_port)
        has_peer = row_has_via(row, args.peer_address, args.peer_port)
        if not has_rvoip and not has_peer:
            upstream.setdefault(tags[0], {}).setdefault(method, set()).add(shape)
        elif has_rvoip and has_peer:
            downstream.setdefault(tags[0], {}).setdefault(method, set()).add(shape)

    response_contract = len(response_shapes) == 1 and all(
        len(shapes) == 1 and len(next(iter(shapes))[1]) >= 1
        for shapes in response_shapes.values()
    )
    upstream_contract = response_contract
    downstream_contract = response_contract
    observations: dict[str, Any] = {}
    for tag, shapes in response_shapes.items():
        if len(shapes) != 1:
            upstream_contract = False
            downstream_contract = False
            continue
        contact, response_routes = next(iter(shapes))
        expected_upstream = {(contact, tuple(reversed(response_routes)))}
        expected_downstream = {(contact, ())}
        observed_upstream = upstream.get(tag, {})
        observed_downstream = downstream.get(tag, {})
        for method in ("ACK", "BYE"):
            upstream_contract = (
                upstream_contract
                and observed_upstream.get(method, set()) == expected_upstream
            )
            downstream_contract = (
                downstream_contract
                and observed_downstream.get(method, set()) == expected_downstream
            )
        observations[tag] = {
            "contact": contact,
            "response_record_routes": list(response_routes),
            "expected_uac_routes": list(reversed(response_routes)),
            "observed_uac": {
                method: sorted(observed_upstream.get(method, set()))
                for method in ("ACK", "BYE")
            },
            "observed_downstream": {
                method: sorted(observed_downstream.get(method, set()))
                for method in ("ACK", "BYE")
            },
        }
    check(
        assertions,
        "invite-dialog-response-contact-and-record-route-set",
        response_contract,
        observations,
    )
    check(
        assertions,
        "invite-dialog-uac-ack-bye-use-contact-and-reversed-route-set",
        upstream_contract,
        observations,
    )
    check(
        assertions,
        "invite-dialog-downstream-ack-bye-reach-contact-with-routes-consumed",
        downstream_contract,
        observations,
    )


def clustered_times(
    rows: list[dict[str, str]], tolerance_seconds: float = 0.05
) -> list[float]:
    values = sorted(value for row in rows if (value := frame_time(row)) is not None)
    clusters: list[float] = []
    for value in values:
        if not clusters or value - clusters[-1] > tolerance_seconds:
            clusters.append(value)
    return clusters


def check_late_2xx_packet_contract(
    assertions: list[dict[str, Any]],
    rows: list[dict[str, str]],
    args: argparse.Namespace,
) -> None:
    upstream_responses = [
        row
        for row in rows
        if response_has_status(row, 200, "INVITE")
        and not row_has_via(row, args.rvoip_address, args.rvoip_port)
        and not row_has_via(row, args.peer_address, args.peer_port)
    ]
    tags = {
        tag
        for row in upstream_responses
        for tag in occurrence_list(row.get("sip.to.tag"))
    }
    events = clustered_times(upstream_responses)
    upstream_acks = [
        row
        for row in rows
        if request_has_method(row, "ACK")
        and not row_has_via(row, args.rvoip_address, args.rvoip_port)
        and not row_has_via(row, args.peer_address, args.peer_port)
    ]
    ack_events = clustered_times(upstream_acks)
    interval = events[1] - events[0] if len(events) == 2 else None
    minimum = max(0.5, args.late_2xx_delay * 0.75)
    check(
        assertions,
        "late-2xx-two-upstream-forwarding-events",
        len(events) == 2,
        events,
    )
    check(
        assertions,
        "late-2xx-same-dialog-to-tag",
        len(tags) == 1 and "" not in tags,
        sorted(tags),
    )
    paired_events = len(events) == len(ack_events) == 2 and all(
        response_at <= ack_at <= response_at + 0.5
        for response_at, ack_at in zip(events, ack_events)
    )
    check(
        assertions,
        "late-2xx-each-forwarding-event-has-dialog-ack",
        paired_events,
        {
            "response_events": events,
            "ack_events": ack_events,
            "maximum_ack_latency_seconds": 0.5,
        },
    )
    check(
        assertions,
        "late-2xx-packet-delay-within-rfc6026-accepted-window",
        interval is not None and minimum <= interval < args.rfc6026_accepted_window,
        {
            "events": events,
            "observed_interval_seconds": interval,
            "minimum_seconds": minimum,
            "accepted_window_seconds": args.rfc6026_accepted_window,
            "phase": "rfc6026-accepted",
            "post_transaction_termination_claimed": False,
        },
    )


def sip_requirements(scenario: str) -> tuple[set[str], set[int]]:
    requirements = {
        "options-readiness": ({"OPTIONS"}, {200}),
        "invite-success": ({"INVITE", "ACK", "BYE"}, {180, 200}),
        "matched-cancel-before-provisional": ({"INVITE", "CANCEL", "ACK"}, {200, 487}),
        "matched-cancel-after-provisional": (
            {"INVITE", "CANCEL", "ACK"},
            {180, 200, 487},
        ),
        "cancel-retransmission": ({"INVITE", "CANCEL", "ACK"}, {180, 200, 487}),
        "unmatched-cancel": ({"CANCEL"}, {481}),
        "ack-non2xx": ({"INVITE", "ACK"}, {486}),
        "via-response-destination": ({"OPTIONS"}, {200}),
        "message-body-content-length": ({"MESSAGE"}, {200}),
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
        "route-strict": ({"INVITE"}, {200}),
        "route-loose-record-route": ({"INVITE"}, {200}),
        "auth-aggregation": ({"INVITE"}, {401, 407}),
        "capacity-overload": ({"INVITE"}, {486, 503}),
        "sips-routing": ({"OPTIONS"}, {200}),
    }
    try:
        return requirements[scenario]
    except KeyError as error:
        raise EvidenceError(
            f"no packet contract for external scenario {scenario}"
        ) from error


def analyze_sip(
    args: argparse.Namespace, captures: list[dict[str, Any]]
) -> dict[str, Any]:
    rows = collect_fields(args.tshark, captures, "sip", SIP_FIELDS)
    marker = f"x-interop-scenario: {args.scenario}".lower()
    call_ids = set().union(
        *(
            row_call_ids(row)
            for row in rows
            if marker in row.get("sip.msg_hdr", "").lower()
        )
    )
    selected = [row for row in rows if row_call_ids(row) & call_ids]
    methods = {
        method
        for row in selected
        for method in split_occurrences(row.get("sip.Method"))
    }
    statuses = {
        int(status)
        for row in selected
        for status in split_occurrences(row.get("sip.Status-Code"))
        if status.isdigit()
    }
    via_addresses = {
        address
        for row in selected
        for address in split_occurrences(row.get("sip.Via.sent-by.address"))
    }
    via_ports = {
        port
        for row in selected
        for port in split_occurrences(row.get("sip.Via.sent-by.port"))
    }
    required_methods, required_statuses = sip_requirements(args.scenario)
    assertions: list[dict[str, Any]] = []
    check(assertions, "scenario-call-id-observed", bool(call_ids), sorted(call_ids))
    check(
        assertions,
        "required-methods-observed",
        required_methods <= methods,
        {"required": sorted(required_methods), "observed": sorted(methods)},
    )
    check(
        assertions,
        "required-statuses-observed",
        required_statuses <= statuses,
        {"required": sorted(required_statuses), "observed": sorted(statuses)},
    )
    check(
        assertions,
        "rvoip-via-observed",
        args.rvoip_address in via_addresses and str(args.rvoip_port) in via_ports,
        {"addresses": sorted(via_addresses), "ports": sorted(via_ports)},
    )
    check(
        assertions,
        "peer-via-observed",
        args.peer_address in via_addresses and str(args.peer_port) in via_ports,
        {"addresses": sorted(via_addresses), "ports": sorted(via_ports)},
    )
    if args.scenario == "stray-response-drop":
        stray_call_ids = {
            call_id for call_id in call_ids if call_id.startswith("true-stray-")
        }
        stray_rows = [
            row for row in selected if row.get("sip.Call-ID", "") in stray_call_ids
        ]
        inbound_to_rvoip = [
            row
            for row in stray_rows
            if endpoint(row, "dst") == (args.rvoip_address, str(args.rvoip_port))
        ]
        outbound_from_rvoip = [
            row
            for row in stray_rows
            if endpoint(row, "src") == (args.rvoip_address, str(args.rvoip_port))
        ]
        check(
            assertions,
            "one-true-stray-call-observed",
            len(stray_call_ids) == 1,
            sorted(stray_call_ids),
        )
        check(
            assertions,
            "true-stray-arrived-at-rvoip",
            bool(inbound_to_rvoip),
            len(inbound_to_rvoip),
        )
        check(
            assertions,
            "true-stray-had-zero-rvoip-egress",
            not outbound_from_rvoip,
            len(outbound_from_rvoip),
        )
    if args.scenario in {"timer-c-calling", "timer-c-proceeding"}:
        check_timer_c_packet_contract(assertions, selected, args)
    elif args.scenario == "transport-failure":
        tcp_rows = collect_fields(args.tshark, captures, "tcp", TCP_FIELDS)
        check_transport_failure_packet_contract(assertions, selected, tcp_rows, args)
    elif args.scenario == "rfc3263-failover":
        tcp_rows = collect_fields(args.tshark, captures, "tcp", TCP_FIELDS)
        dns_rows = collect_fields(args.tshark, captures, "dns", DNS_FIELDS)
        check_rfc3263_packet_contract(assertions, selected, tcp_rows, dns_rows, args)
    elif args.scenario in {"route-strict", "route-loose-record-route"}:
        check_routing_packet_contract(assertions, selected, args)
    elif args.scenario == "capacity-overload":
        check_capacity_packet_contract(assertions, selected, args)
    elif args.scenario == "invite-success":
        check_invite_dialog_route_packet_contract(assertions, selected, args)
    elif args.scenario == "multiple-2xx":
        check_multiple_2xx_ack_packet_contract(assertions, selected, args)
    elif args.scenario == "late-2xx":
        check_late_2xx_packet_contract(assertions, selected, args)
    return {
        "display_filter": "sip",
        "selected_call_ids": sorted(call_ids),
        "selected_packet_count": len(selected),
        "observed_methods": sorted(methods),
        "observed_statuses": sorted(statuses),
        "via_sent_by_addresses": sorted(via_addresses),
        "via_sent_by_ports": sorted(via_ports),
        "assertions": assertions,
    }


def analyze_tls(
    args: argparse.Namespace, captures: list[dict[str, Any]]
) -> dict[str, Any]:
    rows: list[dict[str, str]] = []
    for capture in captures:
        capture_rows = run_fields(args.tshark, Path(capture["path"]), "tls", TLS_FIELDS)
        for row in capture_rows:
            row["capture"] = Path(capture["path"]).name
        rows.extend(capture_rows)
    snis = {
        name
        for row in rows
        for name in split_occurrences(row.get("tls.handshake.extensions_server_name"))
    }
    handshake_types = {
        value
        for row in rows
        for value in split_occurrences(row.get("tls.handshake.type"))
    }
    alerts = [
        {
            "level": row.get("tls.alert_message.level", ""),
            "description": row.get("tls.alert_message.desc", ""),
        }
        for row in rows
        if row.get("tls.alert_message.level") or row.get("tls.alert_message.desc")
    ]
    certificate_hashes = sorted(
        {
            hashlib.sha256(bytes.fromhex(value.replace(":", ""))).hexdigest()
            for row in rows
            for value in split_occurrences(row.get("tls.handshake.certificate"))
            if re.fullmatch(r"(?:[0-9A-Fa-f]{2}:?)+", value)
        }
    )
    expected_sni = set(args.expected_tls_sni)
    client_hello_observed = "1" in handshake_types
    unexpected_sni = snis - expected_sni
    application_rows = [
        row
        for row in rows
        if "23" in split_occurrences(row.get("tls.record.content_type"))
    ]
    required_application_ports = {
        str(args.rvoip_port),
        str(args.peer_port),
        str(args.tls_uas_boundary_port),
    }
    observed_application_ports = {
        port
        for row in application_rows
        for port in (endpoint(row, "src")[1], endpoint(row, "dst")[1])
        if port
    }
    assertions: list[dict[str, Any]] = []
    check(assertions, "tls-packets-observed", bool(rows), len(rows))
    check(
        assertions,
        "tls-application-data-on-every-encrypted-hop",
        required_application_ports <= observed_application_ports,
        {
            "required_listener_ports": sorted(required_application_ports),
            "observed_ports": sorted(observed_application_ports),
            "application_record_count": len(application_rows),
        },
    )
    check(
        assertions,
        "tls-handshake-sni-valid-when-observed",
        bool(expected_sni)
        and not unexpected_sni
        and (not client_hello_observed or bool(snis)),
        {
            "allowed": sorted(expected_sni),
            "observed": sorted(snis),
            "client_hello_observed": client_hello_observed,
        },
    )
    check(
        assertions,
        "tls-handshake-certificates-observed-when-initiated",
        not client_hello_observed or bool(certificate_hashes),
        {
            "client_hello_observed": client_hello_observed,
            "certificate_sha256": certificate_hashes,
        },
    )
    return {
        "display_filter": "tls",
        "selected_packet_count": len(rows),
        "observed_sni": sorted(snis),
        "observed_handshake_types": sorted(handshake_types),
        "observed_certificate_sha256": certificate_hashes,
        "observed_tls_application_listener_ports": sorted(
            required_application_ports & observed_application_ports
        ),
        "observed_alerts": alerts,
        "assertions": assertions,
    }


def analyze_tls_sip(
    args: argparse.Namespace, captures: list[dict[str, Any]]
) -> dict[str, Any]:
    sip = analyze_sip(args, captures)
    tls = analyze_tls(args, captures)
    return {
        **sip,
        "display_filter": "sip or tls",
        "selected_packet_count": int(sip["selected_packet_count"])
        + int(tls["selected_packet_count"]),
        "selected_sip_packet_count": sip["selected_packet_count"],
        "selected_tls_packet_count": tls["selected_packet_count"],
        "observed_sni": tls["observed_sni"],
        "observed_handshake_types": tls["observed_handshake_types"],
        "observed_certificate_sha256": tls["observed_certificate_sha256"],
        "observed_tls_application_listener_ports": tls[
            "observed_tls_application_listener_ports"
        ],
        "observed_alerts": tls["observed_alerts"],
        "assertions": [*sip["assertions"], *tls["assertions"]],
    }


def analyze_sips_routing(
    args: argparse.Namespace, captures: list[dict[str, Any]]
) -> dict[str, Any]:
    required_ports = (
        args.uac_port,
        args.uas_port,
        args.tls_uac_boundary_port,
        args.tls_uas_boundary_port,
        args.tls_peer_boundary_port,
    )
    if any(port <= 0 for port in required_ports):
        raise EvidenceError("sips-routing requires UAC/UAS and TLS boundary ports")
    tls = analyze_tls(args, captures)
    rows = collect_fields(args.tshark, captures, "sip", SIP_FIELDS)
    marker = "x-interop-scenario: sips-routing"
    call_ids = {
        call_id(row) for row in rows if marker in row.get("sip.msg_hdr", "").lower()
    }
    call_ids.discard("")
    selected = [row for row in rows if call_id(row) in call_ids]
    methods = {
        method
        for row in selected
        for method in split_occurrences(row.get("sip.Method"))
    }
    statuses = {
        int(status)
        for row in selected
        for status in split_occurrences(row.get("sip.Status-Code"))
        if status.isdigit()
    }
    via_addresses = {
        address
        for row in selected
        for address in split_occurrences(row.get("sip.Via.sent-by.address"))
    }
    via_ports = {
        port
        for row in selected
        for port in split_occurrences(row.get("sip.Via.sent-by.port"))
    }
    request_uris = sorted(
        {
            uri
            for row in selected
            if request_has_method(row, "OPTIONS")
            for uri in split_occurrences(row.get("sip.r-uri"))
        }
    )
    sips_requests = [
        row
        for row in selected
        if request_has_method(row, "OPTIONS")
        and args.expected_sips_uri in split_occurrences(row.get("sip.r-uri"))
    ]
    at_uac_boundary = [
        row
        for row in sips_requests
        if endpoint(row, "dst") == (args.rvoip_address, str(args.tls_uac_boundary_port))
    ]
    at_uas = [
        row
        for row in sips_requests
        if endpoint(row, "dst") == (args.rvoip_address, str(args.uas_port))
    ]
    both_vias_at_uas = [
        row
        for row in at_uas
        if row_has_via(row, args.rvoip_address, args.rvoip_port)
        and row_has_via(row, args.peer_address, args.peer_port)
    ]
    external_tls_ports = {str(args.rvoip_port), str(args.peer_port)}
    insecure_external_plaintext = [
        row
        for row in selected
        if endpoint(row, "src")[1] in external_tls_ports
        or endpoint(row, "dst")[1] in external_tls_ports
    ]
    plaintext_endpoints = sorted(
        {
            (
                f"{endpoint(row, 'src')[0]}:{endpoint(row, 'src')[1]}",
                f"{endpoint(row, 'dst')[0]}:{endpoint(row, 'dst')[1]}",
            )
            for row in selected
        }
    )
    required_methods, required_statuses = sip_requirements("sips-routing")
    assertions = list(tls["assertions"])
    check(assertions, "scenario-call-id-observed", bool(call_ids), sorted(call_ids))
    check(
        assertions,
        "required-methods-observed",
        required_methods <= methods,
        {"required": sorted(required_methods), "observed": sorted(methods)},
    )
    check(
        assertions,
        "required-statuses-observed",
        required_statuses <= statuses,
        {"required": sorted(required_statuses), "observed": sorted(statuses)},
    )
    via_observation = {
        "addresses": sorted(via_addresses),
        "ports": sorted(via_ports),
    }
    check(
        assertions,
        "rvoip-via-observed",
        args.rvoip_address in via_addresses and str(args.rvoip_port) in via_ports,
        via_observation,
    )
    check(
        assertions,
        "peer-via-observed",
        args.peer_address in via_addresses and str(args.peer_port) in via_ports,
        via_observation,
    )
    check(
        assertions,
        "sips-request-uri-at-uac-boundary",
        bool(at_uac_boundary),
        {
            "required_uri": args.expected_sips_uri,
            "packets": len(at_uac_boundary),
        },
    )
    check(
        assertions,
        "sips-request-uri-at-uas-boundary",
        bool(at_uas),
        {"required_uri": args.expected_sips_uri, "packets": len(at_uas)},
    )
    check(
        assertions,
        "sips-request-preserved-end-to-end",
        bool(request_uris)
        and request_uris == [args.expected_sips_uri]
        and bool(at_uac_boundary)
        and bool(at_uas),
        request_uris,
    )
    check(
        assertions,
        "sips-both-proxy-vias-observed",
        bool(both_vias_at_uas),
        len(both_vias_at_uas),
    )
    check(
        assertions,
        "no-plaintext-sip-on-external-tls-ports",
        not insecure_external_plaintext,
        len(insecure_external_plaintext),
    )
    check(
        assertions,
        "sips-options-success-observed",
        "OPTIONS" in methods and 200 in statuses,
        {"methods": sorted(methods), "statuses": sorted(statuses)},
    )
    tls_packet_count = int(tls["selected_packet_count"])
    return {
        "display_filter": "sip or tls",
        "selected_call_ids": sorted(call_ids),
        "selected_packet_count": tls_packet_count + len(selected),
        "selected_tls_packet_count": tls_packet_count,
        "selected_sip_packet_count": len(selected),
        "observed_methods": sorted(methods),
        "observed_statuses": sorted(statuses),
        "via_sent_by_addresses": sorted(via_addresses),
        "via_sent_by_ports": sorted(via_ports),
        "observed_sips_request_uris": request_uris,
        "plaintext_sip_endpoints": [
            {"source": source, "destination": destination}
            for source, destination in plaintext_endpoints
        ],
        "insecure_external_sip_packet_count": len(insecure_external_plaintext),
        "observed_sni": tls["observed_sni"],
        "observed_handshake_types": tls["observed_handshake_types"],
        "observed_certificate_sha256": tls["observed_certificate_sha256"],
        "observed_tls_application_listener_ports": tls[
            "observed_tls_application_listener_ports"
        ],
        "observed_alerts": tls["observed_alerts"],
        "assertions": assertions,
    }


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--scenario", required=True)
    result.add_argument("--transport", required=True, choices=("udp", "tcp", "tls"))
    result.add_argument("--peer", required=True)
    result.add_argument("--order", required=True)
    result.add_argument("--rvoip-address", required=True)
    result.add_argument("--rvoip-port", required=True, type=int)
    result.add_argument("--peer-address", required=True)
    result.add_argument("--peer-port", required=True, type=int)
    result.add_argument("--uac-port", type=int, default=0)
    result.add_argument("--uas-port", type=int, default=0)
    result.add_argument("--tls-uac-boundary-port", type=int, default=0)
    result.add_argument("--tls-uas-boundary-port", type=int, default=0)
    result.add_argument("--tls-peer-boundary-port", type=int, default=0)
    result.add_argument("--expected-sips-uri", default="sips:probe@example.test")
    result.add_argument("--timer-c-ms", type=int, default=500)
    result.add_argument("--late-2xx-delay", type=float, default=0.75)
    result.add_argument("--rfc6026-accepted-window", type=float, default=32.0)
    result.add_argument("--dead-target-port", type=int)
    result.add_argument("--normal-target-port", type=int)
    result.add_argument("--dns-port", type=int)
    result.add_argument("--live-target-port", type=int)
    result.add_argument("--rfc3263-zone", default="failover.interop.test")
    result.add_argument("--dns-query-log", type=Path)
    result.add_argument("--original-target-uri")
    result.add_argument("--next-hop-route-uri")
    result.add_argument("--record-route-uri")
    result.add_argument("--local-route-uri")
    result.add_argument("--expected-tls-sni", action="append", default=[])
    result.add_argument("--tshark", default="tshark")
    result.add_argument("--output", required=True, type=Path)
    result.add_argument("pcap", nargs="+", type=Path)
    return result


def main() -> int:
    args = parser().parse_args()
    if shutil.which(args.tshark) is None:
        print(
            f"packet evidence: FAIL: tshark not found: {args.tshark}", file=sys.stderr
        )
        return 2
    try:
        captures = []
        for path in args.pcap:
            if not path.is_file() or path.is_symlink() or path.stat().st_size <= 24:
                raise EvidenceError(f"invalid or empty pcap: {path}")
            captures.append(
                {
                    "path": str(path),
                    "filename": path.name,
                    "sha256": sha256(path),
                    "bytes": path.stat().st_size,
                }
            )
        if args.transport == "tls" and args.scenario == "sips-routing":
            analysis = analyze_sips_routing(args, captures)
        elif args.transport == "tls":
            analysis = analyze_tls_sip(args, captures)
        else:
            analysis = analyze_sip(args, captures)
        passed = all(item["passed"] for item in analysis["assertions"])
        payload = {
            "schema": SCHEMA,
            "scenario": args.scenario,
            "status": "PASS" if passed else "FAIL",
            "peer": args.peer,
            "order": args.order,
            "transport": args.transport,
            "analyzer": tshark_version(args.tshark),
            "captures": [
                {key: value for key, value in item.items() if key != "path"}
                for item in captures
            ],
            **analysis,
        }
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
        return 0 if passed else 1
    except (EvidenceError, OSError, ValueError) as error:
        print(f"packet evidence: FAIL: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
