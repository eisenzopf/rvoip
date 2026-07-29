#!/usr/bin/env python3
"""Prove transport-appropriate upstream CANCEL handling through real proxies.

UDP exercises a same-branch transaction retransmission and requires the cached
200 response.  RFC 3261 section 17.2.2 gives reliable transports a zero-length
Timer J, so TCP/TLS exercise one matched CANCEL and its immediate 200 instead
of manufacturing a retransmission that a conforming reliable UAC would not
send.
"""

from __future__ import annotations

import argparse
import json
import select
import socket
import time
import uuid
from pathlib import Path

from raw_unmatched_cancel import (
    MAX_MESSAGE_BYTES,
    append_trace,
    first_header,
    header_values,
    parse_address,
    sip_message_from_stream,
)


def status_and_method(message: bytes) -> tuple[int, str]:
    start_line = message.split(b"\r\n", 1)[0].decode("ascii", errors="replace")
    if not start_line.startswith("SIP/2.0 "):
        return 0, ""
    fields = start_line.split()
    try:
        status = int(fields[1])
    except (IndexError, ValueError):
        return 0, ""
    cseq = first_header(message, "CSeq").split()
    return status, cseq[-1].upper() if cseq else ""


def cancel_request_count_for_transport(transport: str) -> int:
    """Return the RFC transaction-layer CANCEL count exercised on a transport."""
    return 2 if transport == "udp" else 1


def make_request(
    method: str,
    transport: str,
    advertised_host: str,
    advertised_port: int,
    branch: str,
    call_id: str,
    from_tag: str,
    to_value: str,
) -> bytes:
    wire_name = "TLS" if transport == "tls" else transport.upper()
    lines = [
        f"{method} sip:agent@example.test;transport={wire_name} SIP/2.0",
        (
            f"Via: SIP/2.0/{wire_name} {advertised_host}:{advertised_port};"
            f"branch={branch};rport"
        ),
        f"From: <sip:caller@{advertised_host}>;tag={from_tag}",
        f"To: {to_value}",
        f"Call-ID: {call_id}",
        f"CSeq: 1 {method}",
    ]
    if method == "INVITE":
        lines.append(
            f"Contact: <sip:caller@{advertised_host}:{advertised_port};"
            f"transport={wire_name}>"
        )
    lines.extend(
        (
            "X-Interop-Scenario: cancel-retransmission",
            "X-Interop-Peer-Rport: yes",
            "Max-Forwards: 70",
            "Content-Length: 0",
            "",
            "",
        )
    )
    return "\r\n".join(lines).encode("ascii")


def make_response(request: bytes, status: int, reason: str, tag: str) -> bytes:
    to_value = first_header(request, "To")
    if ";tag=" not in to_value.lower():
        to_value += f";tag={tag}"
    lines = [f"SIP/2.0 {status} {reason}"]
    lines.extend(f"Via: {value}" for value in header_values(request, "Via"))
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


class WirePair:
    def __init__(self, args: argparse.Namespace) -> None:
        self.args = args
        self.udp = args.transport == "udp"
        self.listener: socket.socket | None = None
        self.uac: socket.socket | None = None
        self.uas: socket.socket | None = None
        self.uas_peer: tuple[str, int] | None = None
        self.uac_buffer = b""
        self.uas_buffer = b""

    def __enter__(self) -> WirePair:
        if self.udp:
            self.uas = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            self.uac = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            self.uas.bind(self.args.uas_listen)
            self.uac.bind(self.args.uac_bind)
        else:
            self.listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            self.listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            self.listener.bind(self.args.uas_listen)
            self.listener.listen(8)
            self.uac = socket.create_connection(
                self.args.target,
                timeout=self.args.timeout,
                source_address=self.args.uac_bind,
            )
        for stream in (self.listener, self.uac, self.uas):
            if stream is not None:
                stream.settimeout(self.args.timeout)
        return self

    def __exit__(self, *_error: object) -> None:
        for stream in (self.uas, self.uac, self.listener):
            if stream is not None:
                stream.close()

    def send_uac(self, message: bytes) -> None:
        assert self.uac is not None
        if self.udp:
            self.uac.sendto(message, self.args.target)
        else:
            self.uac.sendall(message)
        append_trace(
            self.args.output_dir / "uac-messages.log",
            self.args.transport,
            "sent",
            message,
        )

    def recv_uas(self, timeout: float | None = None) -> bytes:
        if self.uas is None:
            assert self.listener is not None
            self.listener.settimeout(timeout or self.args.timeout)
            self.uas, self.uas_peer = self.listener.accept()
            self.uas.settimeout(timeout or self.args.timeout)
        else:
            self.uas.settimeout(timeout or self.args.timeout)
        if self.udp:
            message, self.uas_peer = self.uas.recvfrom(MAX_MESSAGE_BYTES)
        else:
            message, self.uas_buffer = sip_message_from_stream(
                self.uas, self.uas_buffer
            )
        append_trace(
            self.args.output_dir / "uas-messages.log",
            self.args.transport,
            "received",
            message,
        )
        return message

    def send_uas(self, message: bytes) -> None:
        assert self.uas is not None
        if self.udp:
            assert self.uas_peer is not None
            self.uas.sendto(message, self.uas_peer)
        else:
            self.uas.sendall(message)
        append_trace(
            self.args.output_dir / "uas-messages.log",
            self.args.transport,
            "sent",
            message,
        )

    def recv_uac(self) -> bytes:
        assert self.uac is not None
        if self.udp:
            message, _source = self.uac.recvfrom(MAX_MESSAGE_BYTES)
        else:
            message, self.uac_buffer = sip_message_from_stream(
                self.uac, self.uac_buffer
            )
        append_trace(
            self.args.output_dir / "uac-messages.log",
            self.args.transport,
            "received",
            message,
        )
        return message

    def downstream_ready(self, timeout: float) -> bool:
        assert self.uas is not None
        ready, _write, _error = select.select([self.uas], [], [], timeout)
        return bool(ready)


def receive_response(pair: WirePair, status: int, method: str) -> bytes:
    deadline = time.monotonic() + pair.args.timeout
    while time.monotonic() < deadline:
        response = pair.recv_uac()
        observed_status, observed_method = status_and_method(response)
        if observed_status == status and observed_method == method:
            return response
    raise RuntimeError(f"timed out waiting for {status} {method}")


def run(args: argparse.Namespace) -> dict[str, object]:
    token = uuid.uuid4().hex
    branch = f"z9hG4bK-cancel-retransmission-{token}"
    call_id = f"cancel-retransmission-{token}@{args.advertised_host}"
    from_tag = token[:16]
    to_value = "<sip:agent@example.test>"
    invite = make_request(
        "INVITE",
        args.transport,
        args.advertised_host,
        args.uac_bind[1],
        branch,
        call_id,
        from_tag,
        to_value,
    )
    cancel = make_request(
        "CANCEL",
        args.transport,
        args.advertised_host,
        args.uac_bind[1],
        branch,
        call_id,
        from_tag,
        to_value,
    )
    with WirePair(args) as pair:
        pair.send_uac(invite)
        downstream_invite = pair.recv_uas()
        pair.send_uas(make_response(downstream_invite, 180, "Ringing", token[:12]))
        receive_response(pair, 180, "INVITE")

        pair.send_uac(cancel)
        downstream_cancel = pair.recv_uas()
        if not downstream_cancel.startswith(b"CANCEL "):
            raise RuntimeError("first downstream request after CANCEL was not CANCEL")
        pair.send_uas(make_response(downstream_cancel, 200, "OK", token[:12]))
        first_200 = receive_response(pair, 200, "CANCEL")

        cancel_request_count = cancel_request_count_for_transport(args.transport)
        if cancel_request_count == 2:
            pair.send_uac(cancel)
            second_200 = receive_response(pair, 200, "CANCEL")
            if first_header(first_200, "Call-ID") != first_header(
                second_200, "Call-ID"
            ):
                raise RuntimeError("duplicate CANCEL 200 changed Call-ID")
            if pair.downstream_ready(0.25):
                duplicate_downstream = pair.recv_uas(timeout=0.1)
                if duplicate_downstream.startswith(b"CANCEL "):
                    raise RuntimeError(
                        "duplicate upstream CANCEL created a second branch CANCEL"
                    )

        pair.send_uas(
            make_response(
                downstream_invite, 487, "Request Terminated", token[:12]
            )
        )
        final = receive_response(pair, 487, "INVITE")
        ack = make_request(
            "ACK",
            args.transport,
            args.advertised_host,
            args.uac_bind[1],
            branch,
            call_id,
            from_tag,
            first_header(final, "To"),
        )
        pair.send_uac(ack)
        downstream_ack = pair.recv_uas()
        if not downstream_ack.startswith(b"ACK "):
            raise RuntimeError("non-2xx ACK did not reach the downstream UAS")

    return {
        "schema": "rvoip-sip-proxy-interop-raw-wire-v1",
        "scenario": "cancel-retransmission",
        "status": "PASS",
        "transport": args.transport,
        "transaction_retransmission_exercised": cancel_request_count == 2,
        "timer_j_replay_expected": args.transport == "udp",
        "upstream_cancel_requests": cancel_request_count,
        "upstream_cancel_200_responses": cancel_request_count,
        "downstream_cancel_requests": 1,
        "downstream_ack_requests": 1,
    }


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
