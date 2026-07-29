#!/usr/bin/env python3
"""Bounded TCP wire primitives for real-peer advanced proxy scenarios."""

from __future__ import annotations

import argparse
import json
import re
import select
import socket
import time
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from raw_unmatched_cancel import (
    append_trace,
    first_header,
    header_values,
    parse_address as _parse_address,
    sip_message_from_stream,
)


SCHEMA = "rvoip-sip-proxy-interop-advanced-raw-wire-v1"
MAX_ADVANCED_MESSAGE_BYTES = 64 * 1024
MAX_TRACE_MESSAGES = 512
MAX_TRACE_BYTES = 8 * 1024 * 1024


def parse_address(value: str) -> tuple[str, int]:
    """Expose the shared strict host:port parser to advanced drivers."""

    return _parse_address(value)


def parse_bind_address(value: str) -> tuple[str, int]:
    """Parse host:port while allowing port zero for an ephemeral TCP bind."""

    host, separator, port_text = value.rpartition(":")
    if not separator or not host:
        raise argparse.ArgumentTypeError(
            f"expected host:port, observed {value!r}"
        )
    try:
        port = int(port_text)
    except ValueError as error:
        raise argparse.ArgumentTypeError("port must be an integer") from error
    if not 0 <= port <= 65535:
        raise argparse.ArgumentTypeError("port must be in 0..65535")
    return host, port


def require_safe_value(value: str, label: str) -> str:
    if not value or "\r" in value or "\n" in value:
        raise ValueError(f"{label} must be a non-empty single-line value")
    return value


def request_uri(message: bytes) -> str:
    start = message.split(b"\r\n", 1)[0].decode(
        "ascii", errors="replace"
    )
    fields = start.split()
    if len(fields) != 3 or fields[2] != "SIP/2.0":
        raise RuntimeError(f"invalid SIP request line: {start!r}")
    return fields[1]


def status_and_method(message: bytes) -> tuple[int, str]:
    start = message.split(b"\r\n", 1)[0].decode(
        "ascii", errors="replace"
    )
    if not start.startswith("SIP/2.0 "):
        return 0, ""
    try:
        status = int(start.split()[1])
    except (IndexError, ValueError):
        return 0, ""
    cseq = first_header(message, "CSeq").split()
    return status, cseq[-1].upper() if cseq else ""


def via_sent_bys(message: bytes) -> list[str]:
    result: list[str] = []
    for value in header_values(message, "Via"):
        for entry in value.split(","):
            match = re.match(
                r"\s*SIP/2\.0/[A-Za-z0-9]+\s+([^;,\s]+)",
                entry,
            )
            if match is None:
                raise RuntimeError(f"malformed Via value: {entry!r}")
            result.append(match.group(1))
    return result


def via_branch(message: bytes) -> str:
    values = header_values(message, "Via")
    if not values:
        raise RuntimeError("SIP message has no Via")
    match = re.search(r"(?:^|;)branch=([^;,\s]+)", values[0])
    if match is None:
        raise RuntimeError("top Via has no branch")
    return match.group(1)


def require_external_vias(
    message: bytes,
    expected_rvoip: str,
    expected_peer: str,
    order: str,
) -> dict[str, Any]:
    sent_bys = via_sent_bys(message)
    for authority in (expected_rvoip, expected_peer):
        if sent_bys.count(authority) != 1:
            raise RuntimeError(
                f"expected exactly one Via sent-by {authority!r}; "
                f"observed={sent_bys!r}"
            )
    expected_order = (
        [expected_peer, expected_rvoip]
        if order == "rvoip-first"
        else [expected_rvoip, expected_peer]
    )
    observed_order = [
        value for value in sent_bys if value in set(expected_order)
    ]
    if observed_order != expected_order:
        raise RuntimeError(
            "proxy Via order does not match the configured topology: "
            f"expected={expected_order!r} observed={observed_order!r}"
        )
    return {
        "expected_rvoip_sent_by": expected_rvoip,
        "expected_peer_sent_by": expected_peer,
        "observed_proxy_via_order": observed_order,
        "all_via_sent_bys": sent_bys,
    }


def make_request(
    method: str,
    scenario: str,
    advertised_host: str,
    advertised_port: int,
    *,
    uri: str = "sip:agent@example.test;transport=tcp",
    call_id: str | None = None,
    branch: str | None = None,
    from_tag: str | None = None,
    to_value: str = "<sip:agent@example.test>",
    cseq: int = 1,
    phase: str | None = None,
    extra_headers: tuple[str, ...] = (),
) -> bytes:
    for label, value in (
        ("method", method),
        ("scenario", scenario),
        ("advertised host", advertised_host),
        ("request URI", uri),
        ("To", to_value),
    ):
        require_safe_value(value, label)
    for value in extra_headers:
        require_safe_value(value, "extra header")
        if ":" not in value:
            raise ValueError(f"extra header lacks colon: {value!r}")

    token = uuid.uuid4().hex
    call_id = call_id or f"{scenario}-{token}@{advertised_host}"
    branch = branch or f"z9hG4bK-{scenario}-{token}"
    from_tag = from_tag or token[:16]
    lines = [
        f"{method} {uri} SIP/2.0",
        (
            f"Via: SIP/2.0/TCP {advertised_host}:{advertised_port};"
            f"branch={branch};rport"
        ),
        f"From: <sip:caller@{advertised_host}>;tag={from_tag}",
        f"To: {to_value}",
        f"Call-ID: {call_id}",
        f"CSeq: {cseq} {method}",
    ]
    if method == "INVITE":
        lines.append(
            f"Contact: <sip:caller@{advertised_host}:{advertised_port};"
            "transport=tcp>"
        )
    lines.append(f"X-Interop-Scenario: {scenario}")
    if phase is not None:
        require_safe_value(phase, "phase")
        lines.append(f"X-Interop-Phase: {phase}")
    lines.extend(extra_headers)
    lines.extend(("Max-Forwards: 70", "Content-Length: 0", "", ""))
    return "\r\n".join(lines).encode("ascii")


def make_response(
    request: bytes,
    status: int,
    reason: str,
    *,
    extra_headers: tuple[str, ...] = (),
    copy_record_route: bool = True,
) -> bytes:
    require_safe_value(reason, "reason")
    for value in extra_headers:
        require_safe_value(value, "response header")
        if ":" not in value:
            raise ValueError(f"response header lacks colon: {value!r}")
    to_value = first_header(request, "To")
    if ";tag=" not in to_value.lower():
        to_value += ";tag=advanced-uas"
    lines = [f"SIP/2.0 {status} {reason}"]
    lines.extend(f"Via: {value}" for value in header_values(request, "Via"))
    if copy_record_route:
        lines.extend(
            f"Record-Route: {value}"
            for value in header_values(request, "Record-Route")
        )
    lines.extend(
        (
            f"From: {first_header(request, 'From')}",
            f"To: {to_value}",
            f"Call-ID: {first_header(request, 'Call-ID')}",
            f"CSeq: {first_header(request, 'CSeq')}",
        )
    )
    lines.extend(extra_headers)
    lines.extend(("Content-Length: 0", "", ""))
    return "\r\n".join(lines).encode("iso-8859-1")


@dataclass(frozen=True)
class Received:
    message: bytes
    connection: socket.socket


class TraceBudget:
    def __init__(self, output_dir: Path) -> None:
        self.output_dir = output_dir
        self.messages = 0

    def record(
        self,
        path: Path,
        direction: str,
        message: bytes,
        *,
        aggregate: bool = False,
    ) -> None:
        if len(message) > MAX_ADVANCED_MESSAGE_BYTES:
            raise RuntimeError(
                f"advanced SIP message exceeds {MAX_ADVANCED_MESSAGE_BYTES} bytes"
            )
        self.messages += 1
        if self.messages > MAX_TRACE_MESSAGES:
            raise RuntimeError(
                f"advanced scenario exceeded {MAX_TRACE_MESSAGES} messages"
            )
        append_trace(path, "tcp", direction, message)
        if aggregate and path.name != "uas-messages.log":
            append_trace(
                self.output_dir / "uas-messages.log",
                "tcp",
                direction,
                message,
            )

    def validate_files(self) -> dict[str, int]:
        sizes: dict[str, int] = {}
        for path in sorted(self.output_dir.glob("*-messages.log")):
            size = path.stat().st_size
            if size > MAX_TRACE_BYTES:
                raise RuntimeError(
                    f"trace exceeds {MAX_TRACE_BYTES} bytes: {path.name}"
                )
            sizes[path.name] = size
        return sizes


class TcpUac:
    def __init__(
        self,
        target: tuple[str, int],
        bind: tuple[str, int],
        timeout: float,
        budget: TraceBudget,
    ) -> None:
        self.timeout = timeout
        self.budget = budget
        self.requested_bind = bind
        self.buffer = b""
        self.socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.socket.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.socket.settimeout(timeout)
        try:
            self.socket.bind(bind)
            self.socket.connect(target)
        except BaseException:
            self.socket.close()
            raise

    def close(self) -> None:
        self.socket.close()

    @property
    def local_address(self) -> tuple[str, int]:
        host, port = self.socket.getsockname()
        return str(host), int(port)

    def binding_observation(self) -> dict[str, Any]:
        return {
            "requested": f"{self.requested_bind[0]}:{self.requested_bind[1]}",
            "effective": f"{self.local_address[0]}:{self.local_address[1]}",
            "ephemeral": self.requested_bind[1] == 0,
        }

    def __enter__(self) -> TcpUac:
        return self

    def __exit__(self, *_error: object) -> None:
        self.close()

    def send(self, message: bytes) -> None:
        self.socket.sendall(message)
        self.budget.record(
            self.budget.output_dir / "uac-messages.log",
            "sent",
            message,
        )

    def receive(self) -> bytes:
        message, self.buffer = sip_message_from_stream(
            self.socket, self.buffer
        )
        self.budget.record(
            self.budget.output_dir / "uac-messages.log",
            "received",
            message,
        )
        return message

    def wait_response(
        self,
        status: int,
        method: str,
        *,
        call_id: str | None = None,
    ) -> bytes:
        deadline = time.monotonic() + self.timeout
        while time.monotonic() < deadline:
            message = self.receive()
            observed_status, observed_method = status_and_method(message)
            if (
                observed_status == status
                and observed_method == method.upper()
                and (
                    call_id is None
                    or first_header(message, "Call-ID") == call_id
                )
            ):
                return message
        raise RuntimeError(f"timed out waiting for {status} {method}")

    def wait_final_response(
        self,
        method: str,
        *,
        call_id: str | None = None,
    ) -> bytes:
        deadline = time.monotonic() + self.timeout
        while time.monotonic() < deadline:
            message = self.receive()
            observed_status, observed_method = status_and_method(message)
            if (
                observed_status >= 200
                and observed_method == method.upper()
                and (
                    call_id is None
                    or first_header(message, "Call-ID") == call_id
                )
            ):
                return message
        raise RuntimeError(f"timed out waiting for final {method} response")


class TcpEndpoint:
    def __init__(
        self,
        name: str,
        address: tuple[str, int],
        timeout: float,
        budget: TraceBudget,
    ) -> None:
        self.name = name
        self.timeout = timeout
        self.budget = budget
        self.listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.listener.bind(address)
        self.listener.listen(16)
        self.listener.setblocking(False)
        self.connections: list[socket.socket] = []
        self.buffers: dict[socket.socket, bytes] = {}

    @property
    def log_path(self) -> Path:
        if self.name == "primary":
            return self.budget.output_dir / "uas-messages.log"
        return self.budget.output_dir / f"{self.name}-messages.log"

    def close(self) -> None:
        for connection in self.connections:
            connection.close()
        self.connections.clear()
        self.buffers.clear()
        self.listener.close()

    def __enter__(self) -> TcpEndpoint:
        return self

    def __exit__(self, *_error: object) -> None:
        self.close()

    def receive(self, timeout: float | None = None) -> Received:
        deadline = time.monotonic() + (
            self.timeout if timeout is None else timeout
        )
        while time.monotonic() < deadline:
            remaining = max(0.0, deadline - time.monotonic())
            readable, _writable, _errors = select.select(
                [self.listener, *self.connections],
                [],
                [],
                remaining,
            )
            if not readable:
                break
            for stream in readable:
                if stream is self.listener:
                    connection, _source = self.listener.accept()
                    connection.setblocking(False)
                    self.connections.append(connection)
                    self.buffers[connection] = b""
                    continue
                stream.settimeout(max(0.001, remaining))
                try:
                    message, buffered = sip_message_from_stream(
                        stream, self.buffers[stream]
                    )
                except (ConnectionError, RuntimeError):
                    stream.close()
                    self.connections.remove(stream)
                    self.buffers.pop(stream, None)
                    continue
                finally:
                    if stream.fileno() >= 0:
                        stream.setblocking(False)
                self.buffers[stream] = buffered
                self.budget.record(
                    self.log_path,
                    "received",
                    message,
                    aggregate=self.name != "primary",
                )
                return Received(message, stream)
        raise TimeoutError(f"{self.name} received no SIP message before timeout")

    def receive_optional(self, timeout: float) -> Received | None:
        try:
            return self.receive(timeout)
        except TimeoutError:
            return None

    def send(self, received: Received, message: bytes) -> None:
        received.connection.sendall(message)
        self.budget.record(
            self.log_path,
            "sent",
            message,
            aggregate=self.name != "primary",
        )


def assert_request_scenario(message: bytes, scenario: str) -> None:
    if f"X-Interop-Scenario: {scenario}\r\n".encode("ascii") not in message:
        raise RuntimeError(
            f"downstream request omitted scenario marker {scenario!r}"
        )


def finalize_payload(
    output_dir: Path,
    scenario: str,
    observations: dict[str, Any],
    budget: TraceBudget,
) -> dict[str, Any]:
    return {
        "schema": SCHEMA,
        "scenario": scenario,
        "status": "PASS",
        "transport": "tcp",
        "external_peer_exercised": True,
        "trace_messages": budget.messages,
        "trace_bytes": budget.validate_files(),
        "observations": observations,
    }


def write_payload(output_dir: Path, payload: dict[str, Any]) -> None:
    (output_dir / "raw-wire.json").write_text(
        json.dumps(payload, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def write_error(output_dir: Path, error: BaseException) -> None:
    (output_dir / "raw-wire-error.txt").write_text(
        f"{type(error).__name__}: {error}\n",
        encoding="utf-8",
    )
