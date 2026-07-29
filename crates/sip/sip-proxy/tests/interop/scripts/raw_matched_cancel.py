#!/usr/bin/env python3
"""Exercise matched SIP CANCEL ordering through two real stateful proxies."""

from __future__ import annotations

import argparse
import json
import select
import time
import uuid
from pathlib import Path

from raw_cancel_retransmission import (
    WirePair,
    make_response,
    status_and_method,
)
from raw_unmatched_cancel import first_header, parse_address


def make_request(
    method: str,
    scenario: str,
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
            f"X-Interop-Scenario: {scenario}",
            "X-Interop-Peer-Rport: yes",
            "Max-Forwards: 70",
            "Content-Length: 0",
            "",
            "",
        )
    )
    return "\r\n".join(lines).encode("ascii")


def receive_uac_response(
    pair: WirePair, status: int, method: str, deadline: float
) -> bytes:
    while time.monotonic() < deadline:
        response = pair.recv_uac()
        observed_status, observed_method = status_and_method(response)
        if observed_status == status and observed_method == method:
            return response
    raise RuntimeError(f"timed out waiting for {status} {method}")


def drain_pre_provisional_window(
    pair: WirePair, until: float
) -> tuple[bool, int]:
    cancel_200 = False
    invite_retransmissions = 0
    assert pair.uac is not None
    assert pair.uas is not None
    while time.monotonic() < until:
        timeout = max(0.0, until - time.monotonic())
        ready, _write, _error = select.select(
            [pair.uac, pair.uas], [], [], timeout
        )
        if not ready:
            break
        if pair.uas in ready:
            request = pair.recv_uas()
            if request.startswith(b"CANCEL "):
                raise RuntimeError(
                    "downstream CANCEL arrived before the first UAS provisional"
                )
            if request.startswith(b"INVITE "):
                invite_retransmissions += 1
            else:
                raise RuntimeError(
                    "unexpected downstream request before provisional response"
                )
        if pair.uac in ready:
            response = pair.recv_uac()
            status, method = status_and_method(response)
            if status == 200 and method == "CANCEL":
                cancel_200 = True
            elif status == 100 and method == "INVITE":
                continue
            else:
                raise RuntimeError(
                    f"unexpected upstream response before UAS provisional: "
                    f"{status} {method}"
                )
    return cancel_200, invite_retransmissions


def receive_cancel_and_upstream_200(
    pair: WirePair,
    downstream_invite: bytes,
    cancel_200: bool,
    deadline: float,
) -> tuple[bytes, bool, int]:
    downstream_cancel: bytes | None = None
    upstream_180 = 0
    assert pair.uac is not None
    assert pair.uas is not None
    while time.monotonic() < deadline and (
        downstream_cancel is None or not cancel_200
    ):
        ready, _write, _error = select.select(
            [pair.uac, pair.uas], [], [], max(0.0, deadline - time.monotonic())
        )
        if not ready:
            break
        if pair.uas in ready:
            request = pair.recv_uas()
            if request.startswith(b"CANCEL "):
                if downstream_cancel is not None:
                    raise RuntimeError("matched CANCEL was duplicated downstream")
                downstream_cancel = request
                pair.send_uas(
                    make_response(request, 200, "OK", "matched-cancel")
                )
            elif request.startswith(b"INVITE "):
                continue
            else:
                raise RuntimeError("unexpected downstream request during cancellation")
        if pair.uac in ready:
            response = pair.recv_uac()
            status, method = status_and_method(response)
            if status == 200 and method == "CANCEL":
                cancel_200 = True
            elif status == 180 and method == "INVITE":
                upstream_180 += 1
            elif status == 100 and method == "INVITE":
                continue
            else:
                raise RuntimeError(
                    f"unexpected upstream cancellation response: {status} {method}"
                )
    if downstream_cancel is None:
        raise RuntimeError("matched CANCEL never reached the downstream UAS")
    if not cancel_200:
        raise RuntimeError("upstream UAC never received 200 for matched CANCEL")
    return downstream_cancel, cancel_200, upstream_180


def run(args: argparse.Namespace) -> dict[str, object]:
    token = uuid.uuid4().hex
    scenario = f"matched-cancel-{args.ordering}-provisional"
    branch = f"z9hG4bK-{scenario}-{token}"
    call_id = f"{scenario}-{token}@{args.advertised_host}"
    from_tag = token[:16]
    to_value = "<sip:agent@example.test>"
    invite = make_request(
        "INVITE",
        scenario,
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
        scenario,
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
        deadline = time.monotonic() + args.timeout
        cancel_200 = False
        invite_retransmissions = 0
        upstream_180 = 0

        if args.ordering == "before":
            receive_uac_response(pair, 100, "INVITE", deadline)
            pair.send_uac(cancel)
            cancel_200, invite_retransmissions = drain_pre_provisional_window(
                pair, time.monotonic() + args.pre_provisional_seconds
            )
            pair.send_uas(
                make_response(
                    downstream_invite, 180, "Ringing", token[:12]
                )
            )
        else:
            pair.send_uas(
                make_response(
                    downstream_invite, 180, "Ringing", token[:12]
                )
            )
            receive_uac_response(pair, 180, "INVITE", deadline)
            upstream_180 = 1
            pair.send_uac(cancel)

        _downstream_cancel, cancel_200, later_180 = (
            receive_cancel_and_upstream_200(
                pair, downstream_invite, cancel_200, deadline
            )
        )
        upstream_180 += later_180
        pair.send_uas(
            make_response(
                downstream_invite, 487, "Request Terminated", token[:12]
            )
        )
        final = receive_uac_response(pair, 487, "INVITE", deadline)
        ack = make_request(
            "ACK",
            scenario,
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
        "scenario": scenario,
        "status": "PASS",
        "transport": args.transport,
        "ordering": args.ordering,
        "upstream_cancel_200_responses": 1,
        "upstream_provisional_responses": upstream_180,
        "downstream_cancel_requests": 1,
        "downstream_cancel_before_provisional": False,
        "downstream_ack_requests": 1,
        "pre_provisional_invite_retransmissions": invite_retransmissions,
    }


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--ordering", choices=("before", "after"), required=True)
    result.add_argument("--transport", choices=("udp", "tcp", "tls"), required=True)
    result.add_argument("--uas-listen", type=parse_address, required=True)
    result.add_argument("--uac-bind", type=parse_address, required=True)
    result.add_argument("--target", type=parse_address, required=True)
    result.add_argument("--advertised-host", required=True)
    result.add_argument("--output-dir", type=Path, required=True)
    result.add_argument("--timeout", type=float, default=10)
    result.add_argument("--pre-provisional-seconds", type=float, default=0.25)
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
