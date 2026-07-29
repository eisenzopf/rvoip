#!/usr/bin/env python3
"""Deterministic external-peer UDP drivers for advanced proxy scenarios.

The selected external proxy is the upstream hop:

    raw UAC -> Kamailio/OpenSIPS -> rvoip -> raw UAS endpoints

Fork selection is requested only through the allowlisted
``X-Interop-Scenario`` header. The rvoip interop executable owns the mapping
from that header to its primary target and two ordered ``--aux-target``
endpoints.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import select
import socket
import time
import uuid
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable

from raw_cancel_retransmission import status_and_method
from raw_unmatched_cancel import (
    MAX_MESSAGE_BYTES,
    append_trace,
    first_header,
    header_values,
    parse_address,
)


SCHEMA = "rvoip-sip-proxy-interop-raw-wire-v1"
FORK_SCENARIOS = {
    "sequential-fork",
    "parallel-fork",
    "multiple-2xx",
    "late-2xx",
    "sixxx-cancel",
}
SCENARIOS = (*sorted(FORK_SCENARIOS), "stray-response-drop")
REQUIRED_BRANCHES = ("primary", "aux1", "aux2")


def parse_named_address(value: str) -> tuple[str, tuple[str, int]]:
    name, separator, authority = value.partition("=")
    if not separator or not re.fullmatch(r"[a-z][a-z0-9_-]{0,31}", name):
        raise argparse.ArgumentTypeError(
            "UAS listener must use name=host:port with a simple lowercase name"
        )
    return name, parse_address(authority)


def request_method(message: bytes) -> str:
    start_line = message.split(b"\r\n", 1)[0].decode("ascii", errors="replace")
    return start_line.split(" ", 1)[0].upper()


def request_uri(message: bytes) -> str:
    start_line = message.split(b"\r\n", 1)[0].decode("ascii", errors="replace")
    fields = start_line.split()
    if len(fields) < 3 or fields[-1].upper() != "SIP/2.0":
        raise RuntimeError(f"malformed SIP request line: {start_line!r}")
    return fields[1]


def split_header_entries(message: bytes, header: str) -> list[str]:
    """Flatten a SIP list header without splitting commas inside URI syntax."""

    entries: list[str] = []
    for value in header_values(message, header):
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


def uri_header_values(message: bytes, header: str) -> list[str]:
    return [entry_uri(value) for value in split_header_entries(message, header)]


def sha256_text(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def via_sent_by_values(message: bytes) -> list[str]:
    values: list[str] = []
    for header in header_values(message, "Via"):
        for value in header.split(","):
            match = re.match(
                r"\s*SIP/2\.0/[A-Z0-9]+\s+([^;,\s]+)",
                value,
                re.IGNORECASE,
            )
            if match:
                values.append(match.group(1))
    return values


def make_request(
    method: str,
    scenario: str,
    request_uri: str,
    advertised_host: str,
    advertised_port: int,
    branch: str,
    call_id: str,
    from_tag: str,
    to_value: str,
    cseq: int = 1,
    extra_headers: tuple[str, ...] = (),
) -> bytes:
    lines = [
        f"{method} {request_uri} SIP/2.0",
        (f"Via: SIP/2.0/UDP {advertised_host}:{advertised_port};branch={branch};rport"),
        f"From: <sip:caller@{advertised_host}>;tag={from_tag}",
        f"To: {to_value}",
        f"Call-ID: {call_id}",
        f"CSeq: {cseq} {method}",
    ]
    if method == "INVITE":
        lines.append(
            f"Contact: <sip:caller@{advertised_host}:{advertised_port};transport=udp>"
        )
    lines.extend(extra_headers)
    lines.extend(
        (
            f"X-Interop-Scenario: {scenario}",
            "X-Interop-Peer-Rport: yes",
            "Max-Forwards: 70",
            "Content-Length: 0",
            "",
            "",
        )
    )
    return "\r\n".join(lines).encode("ascii")


def make_response(
    request: bytes,
    status: int,
    reason: str,
    to_tag: str,
    contact: tuple[str, int] | None = None,
) -> bytes:
    to_value = first_header(request, "To")
    if ";tag=" not in to_value.lower():
        to_value += f";tag={to_tag}"
    lines = [f"SIP/2.0 {status} {reason}"]
    lines.extend(f"Via: {value}" for value in header_values(request, "Via"))
    lines.extend(
        f"Record-Route: {value}" for value in header_values(request, "Record-Route")
    )
    lines.extend(
        (
            f"From: {first_header(request, 'From')}",
            f"To: {to_value}",
            f"Call-ID: {first_header(request, 'Call-ID')}",
            f"CSeq: {first_header(request, 'CSeq')}",
        )
    )
    if contact is not None:
        lines.append(f"Contact: <sip:{to_tag}@{contact[0]}:{contact[1]};transport=udp>")
    lines.extend(("Content-Length: 0", "", ""))
    return "\r\n".join(lines).encode("iso-8859-1")


def make_stray_response(
    scenario: str,
    rvoip: tuple[str, int],
    peer: tuple[str, int],
    uac: tuple[str, int],
    token: str,
) -> tuple[str, bytes]:
    call_id = f"true-stray-{token}@example.test"
    wire = "\r\n".join(
        (
            "SIP/2.0 200 OK",
            (
                f"Via: SIP/2.0/UDP {rvoip[0]}:{rvoip[1]};"
                f"branch=z9hG4bK-stray-rvoip-{token}"
            ),
            (f"Via: SIP/2.0/UDP {peer[0]}:{peer[1]};branch=z9hG4bK-stray-peer-{token}"),
            (f"Via: SIP/2.0/UDP {uac[0]}:{uac[1]};branch=z9hG4bK-stray-uac-{token}"),
            f"From: <sip:agent@example.test>;tag=stray-{token[:12]}",
            "To: <sip:caller@example.test>;tag=stray-destination",
            f"Call-ID: {call_id}",
            "CSeq: 1 INVITE",
            f"X-Interop-Scenario: {scenario}",
            "Content-Length: 0",
            "",
            "",
        )
    ).encode("ascii")
    return call_id, wire


@dataclass
class BranchEndpoint:
    name: str
    address: tuple[str, int]
    sock: socket.socket
    peer: tuple[str, int] | None = None
    pending: list[bytes] = field(default_factory=list)
    received_counts: dict[str, int] = field(default_factory=dict)


class UdpHarness:
    def __init__(self, args: argparse.Namespace) -> None:
        self.args = args
        self.uac = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.uac.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.uac.bind(args.uac_bind)
        self.uac.setblocking(False)
        self.uac_pending: list[bytes] = []
        self.branches: dict[str, BranchEndpoint] = {}
        for name, address in args.uas_listen:
            branch_socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            branch_socket.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            branch_socket.bind(address)
            branch_socket.setblocking(False)
            self.branches[name] = BranchEndpoint(name, address, branch_socket)

    def __enter__(self) -> UdpHarness:
        return self

    def __exit__(self, *_error: object) -> None:
        for branch in self.branches.values():
            branch.sock.close()
        self.uac.close()

    def _trace_uac(self, direction: str, message: bytes) -> None:
        append_trace(
            self.args.output_dir / "uac-messages.log",
            "udp",
            direction,
            message,
        )

    def _trace_uas(self, name: str, direction: str, message: bytes) -> None:
        append_trace(
            self.args.output_dir / "uas-messages.log",
            "udp",
            direction,
            message,
        )
        append_trace(
            self.args.output_dir / f"uas-{name}-messages.log",
            "udp",
            direction,
            message,
        )

    def send_uac(self, message: bytes) -> None:
        self.uac.sendto(message, self.args.proxy_target)
        self._trace_uac("sent", message)

    def send_branch(self, name: str, message: bytes) -> None:
        branch = self.branches[name]
        if branch.peer is None:
            raise RuntimeError(f"{name} has not received a request")
        branch.sock.sendto(message, branch.peer)
        self._trace_uas(name, "sent", message)

    def send_branch_direct(
        self, name: str, message: bytes, destination: tuple[str, int]
    ) -> None:
        branch = self.branches[name]
        branch.sock.sendto(message, destination)
        self._trace_uas(name, "sent", message)

    def pump(self, timeout: float) -> bool:
        sockets = [self.uac, *(item.sock for item in self.branches.values())]
        ready, _write, _error = select.select(sockets, [], [], max(0.0, timeout))
        if not ready:
            return False
        by_socket = {item.sock: item for item in self.branches.values()}
        for ready_socket in ready:
            message, source = ready_socket.recvfrom(MAX_MESSAGE_BYTES)
            if ready_socket is self.uac:
                self.uac_pending.append(message)
                self._trace_uac("received", message)
                continue
            branch = by_socket[ready_socket]
            branch.peer = source
            branch.pending.append(message)
            method = request_method(message)
            branch.received_counts[method] = branch.received_counts.get(method, 0) + 1
            self._trace_uas(branch.name, "received", message)
        return True

    def _wait_for(
        self,
        take: Callable[[], bytes | None],
        description: str,
        timeout: float | None = None,
    ) -> bytes:
        deadline = time.monotonic() + (timeout or self.args.timeout)
        while time.monotonic() < deadline:
            result = take()
            if result is not None:
                return result
            self.pump(deadline - time.monotonic())
        raise RuntimeError(f"timed out waiting for {description}")

    def wait_uac_response(
        self, status: int, method: str, timeout: float | None = None
    ) -> bytes:
        def take() -> bytes | None:
            for index, message in enumerate(self.uac_pending):
                if status_and_method(message) == (status, method):
                    return self.uac_pending.pop(index)
            return None

        return self._wait_for(take, f"upstream {status} {method}", timeout)

    def wait_branch_request(
        self, name: str, method: str, timeout: float | None = None
    ) -> bytes:
        def take() -> bytes | None:
            for index, message in enumerate(self.branches[name].pending):
                if request_method(message) == method:
                    return self.branches[name].pending.pop(index)
            return None

        return self._wait_for(take, f"{method} at {name}", timeout)

    def collect_branch_requests(
        self, method: str, names: tuple[str, ...]
    ) -> dict[str, bytes]:
        return {name: self.wait_branch_request(name, method) for name in names}

    def assert_no_branch_request(
        self, method: str, names: tuple[str, ...], duration: float
    ) -> None:
        deadline = time.monotonic() + duration
        while time.monotonic() < deadline:
            self.pump(deadline - time.monotonic())
        for name in names:
            if any(
                request_method(message) == method
                for message in self.branches[name].pending
            ):
                raise RuntimeError(f"unexpected {method} reached {name}")

    def assert_no_uac_call_id(self, call_id: str, duration: float) -> None:
        deadline = time.monotonic() + duration
        while time.monotonic() < deadline:
            self.pump(deadline - time.monotonic())
        if any(
            first_header(message, "Call-ID") == call_id for message in self.uac_pending
        ):
            raise RuntimeError("true stray response was forwarded to the upstream UAC")

    def assert_external_path(self, request: bytes) -> None:
        vias = via_sent_by_values(request)
        for expected in (
            self.args.expected_rvoip_sent_by,
            self.args.expected_peer_sent_by,
        ):
            if expected not in vias:
                raise RuntimeError(
                    f"downstream request omitted external path Via {expected}; "
                    f"observed={vias}"
                )


@dataclass(frozen=True)
class CallIdentity:
    scenario: str
    token: str
    call_id: str
    from_tag: str
    invite_branch: str


def call_identity(scenario: str) -> CallIdentity:
    token = uuid.uuid4().hex
    return CallIdentity(
        scenario=scenario,
        token=token,
        call_id=f"{scenario}-{token}@example.test",
        from_tag=token[:16],
        invite_branch=f"z9hG4bK-{scenario}-{token}",
    )


def initial_invite(args: argparse.Namespace, call: CallIdentity) -> bytes:
    return make_request(
        "INVITE",
        call.scenario,
        "sip:agent@example.test;transport=udp",
        args.advertised_host,
        args.uac_bind[1],
        call.invite_branch,
        call.call_id,
        call.from_tag,
        "<sip:agent@example.test>",
    )


def branch_contact_uri(branch: BranchEndpoint) -> str:
    return f"sip:{branch.name}@{branch.address[0]}:{branch.address[1]};transport=udp"


def send_dialog_ack(
    harness: UdpHarness,
    call: CallIdentity,
    response: bytes,
    branch_name: str,
    generation: int,
) -> tuple[bytes, dict[str, object]]:
    contacts = uri_header_values(response, "Contact")
    if len(contacts) != 1:
        raise RuntimeError(
            f"2xx for {branch_name} must carry exactly one Contact; observed={contacts}"
        )
    response_route_entries = split_header_entries(response, "Record-Route")
    response_routes = [entry_uri(value) for value in response_route_entries]
    ack_route_entries = list(reversed(response_route_entries))
    ack = make_request(
        "ACK",
        call.scenario,
        contacts[0],
        harness.args.advertised_host,
        harness.args.uac_bind[1],
        f"z9hG4bK-{call.scenario}-ack-{generation}-{call.token}",
        call.call_id,
        call.from_tag,
        first_header(response, "To"),
        extra_headers=tuple(f"Route: {entry}" for entry in ack_route_entries),
    )
    harness.send_uac(ack)
    return ack, {
        "branch": branch_name,
        "response_contact_uri": contacts[0],
        "response_record_route_set": response_routes,
        "uac_ack_request_uri": request_uri(ack),
        "uac_ack_route_set": uri_header_values(ack, "Route"),
    }


def verify_dialog_ack_at_branch(
    harness: UdpHarness,
    ack: bytes,
    response: bytes,
    branch_name: str,
    prepared: dict[str, object],
    *,
    minimum_record_routes: int = 0,
) -> dict[str, object]:
    harness.assert_external_path(ack)
    contact = uri_header_values(response, "Contact")
    response_routes = uri_header_values(response, "Record-Route")
    downstream_routes = uri_header_values(ack, "Route")
    observed_uri = request_uri(ack)
    expected_uri = contact[0] if len(contact) == 1 else ""
    if len(response_routes) < minimum_record_routes:
        raise RuntimeError(
            f"{branch_name} 2xx carried {len(response_routes)} Record-Route "
            f"values; required at least {minimum_record_routes}"
        )
    if prepared.get("uac_ack_request_uri") != expected_uri:
        raise RuntimeError("prepared ACK did not use the response Contact")
    if prepared.get("uac_ack_route_set") != list(reversed(response_routes)):
        raise RuntimeError("prepared ACK did not reverse the response route set")
    if observed_uri != expected_uri:
        raise RuntimeError(
            f"{branch_name} received ACK for {observed_uri}, expected {expected_uri}"
        )
    if downstream_routes:
        raise RuntimeError(
            f"{branch_name} received ACK with unconsumed Route values: "
            f"{downstream_routes}"
        )
    if first_header(ack, "To") != first_header(response, "To"):
        raise RuntimeError(f"{branch_name} ACK changed the dialog To tag")
    return {
        **prepared,
        "downstream_ack_request_uri": observed_uri,
        "downstream_ack_route_set": downstream_routes,
        "route_values_consumed_by_proxies": True,
    }


def send_failure_ack(harness: UdpHarness, call: CallIdentity, response: bytes) -> None:
    ack = make_request(
        "ACK",
        call.scenario,
        "sip:agent@example.test;transport=udp",
        harness.args.advertised_host,
        harness.args.uac_bind[1],
        call.invite_branch,
        call.call_id,
        call.from_tag,
        first_header(response, "To"),
    )
    harness.send_uac(ack)


def verify_invites(harness: UdpHarness, invites: dict[str, bytes]) -> None:
    for request in invites.values():
        harness.assert_external_path(request)


def run_sequential(harness: UdpHarness, args: argparse.Namespace) -> dict[str, object]:
    call = call_identity(args.scenario)
    harness.send_uac(initial_invite(args, call))
    primary = harness.wait_branch_request("primary", "INVITE")
    harness.assert_external_path(primary)
    harness.assert_no_branch_request("INVITE", ("aux1", "aux2"), 0.1)

    harness.send_branch(
        "primary", make_response(primary, 486, "Busy Here", "sequential-primary")
    )
    harness.wait_branch_request("primary", "ACK")
    aux1 = harness.wait_branch_request("aux1", "INVITE")
    harness.assert_external_path(aux1)
    harness.assert_no_branch_request("INVITE", ("aux2",), 0.1)

    harness.send_branch("aux1", make_response(aux1, 180, "Ringing", "sequential-aux1"))
    harness.wait_uac_response(180, "INVITE")
    harness.send_branch(
        "aux1",
        make_response(
            aux1,
            200,
            "OK",
            "sequential-aux1",
            harness.branches["aux1"].address,
        ),
    )
    success = harness.wait_uac_response(200, "INVITE")
    send_dialog_ack(harness, call, success, "aux1", 1)
    harness.wait_branch_request("aux1", "ACK")
    harness.assert_no_branch_request("INVITE", ("aux2",), 0.1)
    return {
        "call_id_sha256": sha256_text(call.call_id),
        "branch_order": ["primary", "aux1"],
        "branch_finals": {"primary": 486, "aux1": 200},
        "unused_branches": ["aux2"],
        "upstream_final": 200,
    }


def run_parallel(harness: UdpHarness, args: argparse.Namespace) -> dict[str, object]:
    call = call_identity(args.scenario)
    harness.send_uac(initial_invite(args, call))
    invites = harness.collect_branch_requests("INVITE", REQUIRED_BRANCHES)
    verify_invites(harness, invites)

    harness.send_branch(
        "primary",
        make_response(invites["primary"], 486, "Busy Here", "parallel-primary"),
    )
    harness.send_branch(
        "aux1",
        make_response(invites["aux1"], 480, "Temporarily Unavailable", "parallel-aux1"),
    )
    harness.wait_branch_request("primary", "ACK")
    harness.wait_branch_request("aux1", "ACK")
    harness.send_branch(
        "aux2", make_response(invites["aux2"], 180, "Ringing", "parallel-aux2")
    )
    harness.wait_uac_response(180, "INVITE")
    harness.send_branch(
        "aux2",
        make_response(
            invites["aux2"],
            200,
            "OK",
            "parallel-aux2",
            harness.branches["aux2"].address,
        ),
    )
    success = harness.wait_uac_response(200, "INVITE")
    send_dialog_ack(harness, call, success, "aux2", 1)
    harness.wait_branch_request("aux2", "ACK")
    return {
        "call_id_sha256": sha256_text(call.call_id),
        "parallel_invite_branches": list(REQUIRED_BRANCHES),
        "branch_finals": {"primary": 486, "aux1": 480, "aux2": 200},
        "upstream_final": 200,
    }


def run_multiple_2xx(
    harness: UdpHarness, args: argparse.Namespace
) -> dict[str, object]:
    call = call_identity(args.scenario)
    harness.send_uac(initial_invite(args, call))
    invites = harness.collect_branch_requests("INVITE", REQUIRED_BRANCHES)
    verify_invites(harness, invites)

    harness.send_branch(
        "aux2", make_response(invites["aux2"], 486, "Busy Here", "multi-aux2")
    )
    harness.wait_branch_request("aux2", "ACK")
    first_wire = make_response(
        invites["primary"],
        200,
        "OK",
        "multi-primary",
        harness.branches["primary"].address,
    )
    second_wire = make_response(
        invites["aux1"],
        200,
        "OK",
        "multi-aux1",
        harness.branches["aux1"].address,
    )
    harness.send_branch("primary", first_wire)
    first = harness.wait_uac_response(200, "INVITE")
    _first_ack, first_prepared = send_dialog_ack(harness, call, first, "primary", 1)
    first_at_uas = harness.wait_branch_request("primary", "ACK")
    first_evidence = verify_dialog_ack_at_branch(
        harness,
        first_at_uas,
        first,
        "primary",
        first_prepared,
        minimum_record_routes=2,
    )
    harness.send_branch("aux1", second_wire)
    second = harness.wait_uac_response(200, "INVITE")
    if first_header(first, "To") == first_header(second, "To"):
        raise RuntimeError("distinct forked 2xx responses lost their To tags")
    _second_ack, second_prepared = send_dialog_ack(harness, call, second, "aux1", 2)
    second_at_uas = harness.wait_branch_request("aux1", "ACK")
    second_evidence = verify_dialog_ack_at_branch(
        harness,
        second_at_uas,
        second,
        "aux1",
        second_prepared,
        minimum_record_routes=2,
    )
    return {
        "call_id_sha256": sha256_text(call.call_id),
        "distinct_2xx_to_tags": 2,
        "upstream_invite_2xx": 2,
        "acked_2xx_branches": ["primary", "aux1"],
        "dialog_ack_routes": [first_evidence, second_evidence],
        "failure_branches": {"aux2": 486},
    }


def run_late_2xx(harness: UdpHarness, args: argparse.Namespace) -> dict[str, object]:
    call = call_identity(args.scenario)
    harness.send_uac(initial_invite(args, call))
    invites = harness.collect_branch_requests("INVITE", REQUIRED_BRANCHES)
    verify_invites(harness, invites)
    for name, status, reason in (
        ("aux1", 480, "Temporarily Unavailable"),
        ("aux2", 486, "Busy Here"),
    ):
        harness.send_branch(
            name, make_response(invites[name], status, reason, f"late-{name}")
        )
        harness.wait_branch_request(name, "ACK")

    success_wire = make_response(
        invites["primary"],
        200,
        "OK",
        "late-primary",
        harness.branches["primary"].address,
    )
    harness.send_branch("primary", success_wire)
    first_sent_at = time.monotonic()
    first = harness.wait_uac_response(200, "INVITE")
    first_forwarded_at = time.monotonic()
    _first_ack, first_prepared = send_dialog_ack(harness, call, first, "primary", 1)
    first_at_uas = harness.wait_branch_request("primary", "ACK")
    first_ack_evidence = verify_dialog_ack_at_branch(
        harness,
        first_at_uas,
        first,
        "primary",
        first_prepared,
        minimum_record_routes=2,
    )
    time.sleep(args.late_2xx_delay)
    second_sent_at = time.monotonic()
    harness.send_branch("primary", success_wire)
    second = harness.wait_uac_response(200, "INVITE")
    second_forwarded_at = time.monotonic()
    forwarding_interval = second_forwarded_at - first_forwarded_at
    if forwarding_interval < args.late_2xx_delay * 0.8:
        raise RuntimeError(
            "late 2xx forwarding interval was shorter than the requested delay: "
            f"requested={args.late_2xx_delay:.3f}s "
            f"observed={forwarding_interval:.3f}s"
        )
    if forwarding_interval >= args.rfc6026_accepted_window:
        raise RuntimeError(
            "late 2xx was sent outside the configured RFC 6026 Accepted window"
        )
    if first_header(first, "To") != first_header(second, "To"):
        raise RuntimeError("late retransmitted 2xx changed its dialog To tag")
    _second_ack, second_prepared = send_dialog_ack(harness, call, second, "primary", 2)
    second_at_uas = harness.wait_branch_request("primary", "ACK")
    second_ack_evidence = verify_dialog_ack_at_branch(
        harness,
        second_at_uas,
        second,
        "primary",
        second_prepared,
        minimum_record_routes=2,
    )
    return {
        "call_id_sha256": sha256_text(call.call_id),
        "late_2xx_delay_seconds": args.late_2xx_delay,
        "late_2xx_timing": {
            "phase": "rfc6026-accepted",
            "requested_delay_seconds": args.late_2xx_delay,
            "observed_forwarding_interval_seconds": forwarding_interval,
            "first_forward_latency_seconds": first_forwarded_at - first_sent_at,
            "second_forward_latency_seconds": second_forwarded_at - second_sent_at,
            "accepted_window_seconds": args.rfc6026_accepted_window,
            "within_accepted_window": True,
            "post_transaction_termination_claimed": False,
        },
        "same_dialog_upstream_2xx": 2,
        "acked_2xx_branches": ["primary", "primary"],
        "late_dialog_ack_routes": [
            first_ack_evidence,
            second_ack_evidence,
        ],
        "failure_branches": {"aux1": 480, "aux2": 486},
    }


def run_sixxx(harness: UdpHarness, args: argparse.Namespace) -> dict[str, object]:
    call = call_identity(args.scenario)
    harness.send_uac(initial_invite(args, call))
    invites = harness.collect_branch_requests("INVITE", REQUIRED_BRANCHES)
    verify_invites(harness, invites)
    for name in REQUIRED_BRANCHES:
        harness.send_branch(
            name,
            make_response(invites[name], 180, "Ringing", f"sixxx-{name}"),
        )
    harness.wait_uac_response(180, "INVITE")
    harness.send_branch(
        "primary",
        make_response(invites["primary"], 603, "Decline", "sixxx-primary"),
    )
    harness.wait_branch_request("primary", "ACK")

    cancelled: list[str] = []
    for name in ("aux1", "aux2"):
        cancel = harness.wait_branch_request(name, "CANCEL")
        cancelled.append(name)
        harness.send_branch(name, make_response(cancel, 200, "OK", f"sixxx-{name}"))
        harness.send_branch(
            name,
            make_response(invites[name], 487, "Request Terminated", f"sixxx-{name}"),
        )
        harness.wait_branch_request(name, "ACK")
    # The proxy deliberately latches the global failure until the cancelled
    # siblings settle, leaving room for a racing 2xx to win.
    upstream_final = harness.wait_uac_response(603, "INVITE")
    send_failure_ack(harness, call, upstream_final)
    harness.assert_no_branch_request("CANCEL", REQUIRED_BRANCHES, 0.1)
    cancel_counts = {
        name: harness.branches[name].received_counts.get("CANCEL", 0)
        for name in REQUIRED_BRANCHES
    }
    if cancel_counts != {"primary": 0, "aux1": 1, "aux2": 1}:
        raise RuntimeError(
            "6xx cancellation must reach each eligible proceeding branch exactly once; "
            f"observed={cancel_counts}"
        )
    return {
        "call_id_sha256": sha256_text(call.call_id),
        "global_failure": 603,
        "cancelled_proceeding_branches": cancelled,
        "cancel_requests_per_branch": cancel_counts,
        "upstream_final": 603,
    }


def run_stray(harness: UdpHarness, args: argparse.Namespace) -> dict[str, object]:
    call = call_identity(args.scenario)
    options = make_request(
        "OPTIONS",
        call.scenario,
        "sip:agent@example.test;transport=udp",
        args.advertised_host,
        args.uac_bind[1],
        call.invite_branch,
        call.call_id,
        call.from_tag,
        "<sip:agent@example.test>",
    )
    harness.send_uac(options)
    downstream = harness.wait_branch_request("primary", "OPTIONS")
    harness.assert_external_path(downstream)
    harness.send_branch(
        "primary", make_response(downstream, 200, "OK", "stray-readiness")
    )
    harness.wait_uac_response(200, "OPTIONS")

    stray_call_id, stray = make_stray_response(
        args.scenario,
        args.rvoip_target,
        args.proxy_target,
        args.uac_bind,
        call.token,
    )
    harness.send_branch_direct("primary", stray, args.rvoip_target)
    harness.assert_no_uac_call_id(stray_call_id, args.stray_observation_seconds)
    return {
        "call_id_sha256": sha256_text(call.call_id),
        "readiness_options_status": 200,
        "stray_call_id": stray_call_id,
        "stray_call_id_sha256": sha256_text(stray_call_id),
        "stray_observation_seconds": args.stray_observation_seconds,
        "stray_upstream_responses": 0,
    }


def validate_args(args: argparse.Namespace) -> None:
    branches = dict(args.uas_listen)
    if len(branches) != len(args.uas_listen):
        raise ValueError("UAS listener names must be unique")
    required = set(
        REQUIRED_BRANCHES if args.scenario in FORK_SCENARIOS else ("primary",)
    )
    if not required <= set(branches):
        raise ValueError(f"{args.scenario} requires UAS listeners {sorted(required)}")
    if args.late_2xx_delay <= 0 or args.late_2xx_delay >= args.timeout:
        raise ValueError(
            "late 2xx delay must be positive and below the scenario timeout"
        )
    if (
        args.rfc6026_accepted_window <= args.late_2xx_delay
        or args.rfc6026_accepted_window > 64.0
    ):
        raise ValueError(
            "RFC 6026 Accepted window must exceed the late 2xx delay "
            "and be at most 64 seconds"
        )
    if (
        args.stray_observation_seconds <= 0
        or args.stray_observation_seconds >= args.timeout
    ):
        raise ValueError(
            "stray observation window must be positive and below the scenario timeout"
        )


def run(args: argparse.Namespace) -> dict[str, object]:
    validate_args(args)
    runners: dict[
        str, Callable[[UdpHarness, argparse.Namespace], dict[str, object]]
    ] = {
        "sequential-fork": run_sequential,
        "parallel-fork": run_parallel,
        "multiple-2xx": run_multiple_2xx,
        "late-2xx": run_late_2xx,
        "sixxx-cancel": run_sixxx,
        "stray-response-drop": run_stray,
    }
    with UdpHarness(args) as harness:
        details = runners[args.scenario](harness, args)
        counts = {
            name: dict(sorted(branch.received_counts.items()))
            for name, branch in sorted(harness.branches.items())
        }
    return {
        "schema": SCHEMA,
        "scenario": args.scenario,
        "status": "PASS",
        "transport": "udp",
        "external_peer_path_observed": True,
        "branch_request_counts": counts,
        **details,
    }


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--scenario", choices=SCENARIOS, required=True)
    result.add_argument("--uac-bind", type=parse_address, required=True)
    result.add_argument("--proxy-target", type=parse_address, required=True)
    result.add_argument("--rvoip-target", type=parse_address, required=True)
    result.add_argument(
        "--uas-listen",
        action="append",
        type=parse_named_address,
        required=True,
        metavar="NAME=HOST:PORT",
    )
    result.add_argument("--advertised-host", required=True)
    result.add_argument("--expected-rvoip-sent-by", required=True)
    result.add_argument("--expected-peer-sent-by", required=True)
    result.add_argument("--output-dir", type=Path, required=True)
    result.add_argument("--timeout", type=float, default=10.0)
    result.add_argument("--late-2xx-delay", type=float, default=0.75)
    result.add_argument(
        "--rfc6026-accepted-window",
        type=float,
        default=32.0,
        help=(
            "known INVITE Timer M/L retention window used only to bound the "
            "delayed-2xx observation; this scenario does not claim forwarding "
            "after transaction termination"
        ),
    )
    result.add_argument("--stray-observation-seconds", type=float, default=0.5)
    return result


def main() -> int:
    args = parser().parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    try:
        payload = run(args)
        (args.output_dir / "raw-wire.json").write_text(
            json.dumps(payload, indent=2, sort_keys=True) + "\n"
        )
        return 0
    except (OSError, RuntimeError, ValueError) as error:
        (args.output_dir / "raw-wire-error.txt").write_text(
            f"{type(error).__name__}: {error}\n"
        )
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
