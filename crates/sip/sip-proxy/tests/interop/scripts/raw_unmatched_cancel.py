#!/usr/bin/env python3
"""Exercise an unmatched CANCEL through both external proxies.

SIPp treats an initial CANCEL as out-of-call traffic and cannot use a normal
UAS scenario to return the downstream 481. This bounded driver supplies both
wire endpoints while the request and response still traverse the real rvoip
proxy and the selected Kamailio/OpenSIPS peer.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import socket
import time
import uuid
from pathlib import Path


MAX_MESSAGE_BYTES = 1024 * 1024


def parse_address(value: str) -> tuple[str, int]:
    host, separator, port_text = value.rpartition(":")
    if not separator or not host:
        raise argparse.ArgumentTypeError(f"expected host:port, observed {value!r}")
    try:
        port = int(port_text)
    except ValueError as error:
        raise argparse.ArgumentTypeError("port must be an integer") from error
    if not 0 < port <= 65535:
        raise argparse.ArgumentTypeError("port must be in 1..65535")
    return host, port


def sip_message_from_stream(
    stream: socket.socket, buffered: bytes
) -> tuple[bytes, bytes]:
    while b"\r\n\r\n" not in buffered:
        block = stream.recv(64 * 1024)
        if not block:
            raise RuntimeError("stream closed before a complete SIP header")
        buffered += block
        if len(buffered) > MAX_MESSAGE_BYTES:
            raise RuntimeError("SIP header exceeds bounded driver limit")
    header, remainder = buffered.split(b"\r\n\r\n", 1)
    content_length = 0
    for line in header.split(b"\r\n")[1:]:
        name, separator, value = line.partition(b":")
        if separator and name.strip().lower() == b"content-length":
            content_length = int(value.strip())
            break
    if content_length < 0 or len(header) + content_length > MAX_MESSAGE_BYTES:
        raise RuntimeError("invalid or oversized SIP Content-Length")
    while len(remainder) < content_length:
        block = stream.recv(min(64 * 1024, content_length - len(remainder)))
        if not block:
            raise RuntimeError("stream closed before the complete SIP body")
        remainder += block
    body, remainder = remainder[:content_length], remainder[content_length:]
    return header + b"\r\n\r\n" + body, remainder


def header_values(message: bytes, wanted: str) -> list[str]:
    values: list[str] = []
    for raw_line in message.decode("iso-8859-1").split("\r\n")[1:]:
        name, separator, value = raw_line.partition(":")
        if separator and name.strip().lower() == wanted.lower():
            values.append(value.strip())
    return values


def first_header(message: bytes, wanted: str) -> str:
    values = header_values(message, wanted)
    if not values:
        raise RuntimeError(f"received SIP message lacks {wanted}")
    return values[0]


def build_request(
    advertised_host: str, advertised_port: int, transport: str
) -> tuple[bytes, str]:
    token = uuid.uuid4().hex
    call_id = f"unmatched-{token}@{advertised_host}"
    wire_name = "TLS" if transport == "tls" else transport.upper()
    request = (
        f"CANCEL sip:agent@example.test;transport={wire_name} SIP/2.0\r\n"
        f"Via: SIP/2.0/{wire_name} {advertised_host}:{advertised_port};"
        f"branch=z9hG4bK-unmatched-{token};rport\r\n"
        f"From: <sip:caller@{advertised_host}>;tag={token[:16]}\r\n"
        "To: <sip:agent@example.test>\r\n"
        f"Call-ID: {call_id}\r\n"
        "CSeq: 1 CANCEL\r\n"
        "X-Interop-Scenario: unmatched-cancel\r\n"
        "X-Interop-Unmatched-Cancel: yes\r\n"
        "X-Interop-Peer-Rport: yes\r\n"
        "Max-Forwards: 70\r\n"
        "Content-Length: 0\r\n"
        "\r\n"
    ).encode("ascii")
    return request, call_id


def build_response(request: bytes) -> bytes:
    vias = header_values(request, "Via")
    if not vias:
        raise RuntimeError("unmatched CANCEL reached UAS without a Via")
    to_value = first_header(request, "To")
    if ";tag=" not in to_value.lower():
        to_value += ";tag=raw-unmatched-uas"
    lines = ["SIP/2.0 481 Call/Transaction Does Not Exist"]
    lines.extend(f"Via: {value}" for value in vias)
    lines.extend(
        (
            f"From: {first_header(request, 'From')}",
            f"To: {to_value}",
            f"Call-ID: {first_header(request, 'Call-ID')}",
            f"CSeq: {first_header(request, 'CSeq')}",
            "X-Interop-Origin: uas",
            "Content-Length: 0",
            "",
            "",
        )
    )
    return "\r\n".join(lines).encode("iso-8859-1")


def append_trace(path: Path, transport: str, direction: str, message: bytes) -> None:
    timestamp = time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime())
    with path.open("ab") as stream:
        stream.write(
            (
                f"----------------------------------------------- {timestamp}\n"
                f"{transport.upper()} message {direction} [{len(message)}] bytes:\n\n"
            ).encode("ascii")
        )
        stream.write(message.replace(b"\r\n", b"\n"))
        stream.write(b"\n\n")


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run_udp(args: argparse.Namespace, request: bytes) -> tuple[bytes, bytes]:
    uas = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    uac = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    for stream in (uas, uac):
        stream.settimeout(args.timeout)
    try:
        uas.bind(args.uas_listen)
        uac.bind(args.uac_bind)
        uac.sendto(request, args.target)
        downstream, source = uas.recvfrom(MAX_MESSAGE_BYTES)
        response = build_response(downstream)
        uas.sendto(response, source)
        while True:
            upstream, _source = uac.recvfrom(MAX_MESSAGE_BYTES)
            if upstream.startswith(b"SIP/2.0 481 "):
                return downstream, upstream
    finally:
        uas.close()
        uac.close()


def run_tcp(args: argparse.Namespace, request: bytes) -> tuple[bytes, bytes]:
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    listener.settimeout(args.timeout)
    listener.bind(args.uas_listen)
    listener.listen(8)
    uac = socket.create_connection(
        args.target, timeout=args.timeout, source_address=args.uac_bind
    )
    uac.settimeout(args.timeout)
    downstream_connection: socket.socket | None = None
    try:
        uac.sendall(request)
        downstream_connection, _source = listener.accept()
        downstream_connection.settimeout(args.timeout)
        downstream, _ = sip_message_from_stream(downstream_connection, b"")
        response = build_response(downstream)
        downstream_connection.sendall(response)
        buffered = b""
        while True:
            upstream, buffered = sip_message_from_stream(uac, buffered)
            if upstream.startswith(b"SIP/2.0 481 "):
                return downstream, upstream
    finally:
        if downstream_connection is not None:
            downstream_connection.close()
        uac.close()
        listener.close()


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--transport", choices=("udp", "tcp", "tls"), required=True)
    result.add_argument("--uas-listen", type=parse_address, required=True)
    result.add_argument("--uac-bind", type=parse_address, required=True)
    result.add_argument("--target", type=parse_address, required=True)
    result.add_argument("--advertised-host", required=True)
    result.add_argument("--output-dir", type=Path, required=True)
    result.add_argument("--timeout", type=float, default=10)
    return result


def main() -> int:
    args = parser().parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    request, call_id = build_request(
        args.advertised_host, args.uac_bind[1], args.transport
    )
    log_transport = args.transport
    try:
        if args.transport == "udp":
            downstream, upstream = run_udp(args, request)
        else:
            downstream, upstream = run_tcp(args, request)
        if (
            not downstream.startswith(b"CANCEL ")
            or b"X-Interop-Scenario: unmatched-cancel\r\n" not in downstream
            or first_header(upstream, "Call-ID") != call_id
            or first_header(upstream, "CSeq").upper() != "1 CANCEL"
        ):
            raise RuntimeError("unmatched CANCEL wire identity was not preserved")
        append_trace(
            args.output_dir / "uac-messages.log", log_transport, "sent", request
        )
        append_trace(
            args.output_dir / "uas-messages.log",
            log_transport,
            "received",
            downstream,
        )
        append_trace(
            args.output_dir / "uas-messages.log",
            log_transport,
            "sent",
            build_response(downstream),
        )
        append_trace(
            args.output_dir / "uac-messages.log",
            log_transport,
            "received",
            upstream,
        )
        payload = {
            "schema": "rvoip-sip-proxy-interop-raw-wire-v1",
            "scenario": "unmatched-cancel",
            "status": "PASS",
            "transport": args.transport,
            "call_id_sha256": hashlib.sha256(call_id.encode()).hexdigest(),
            "request_bytes": len(request),
            "downstream_request_bytes": len(downstream),
            "upstream_response_bytes": len(upstream),
        }
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
