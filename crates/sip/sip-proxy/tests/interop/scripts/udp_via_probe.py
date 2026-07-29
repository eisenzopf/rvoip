#!/usr/bin/env python3
"""Prove RFC 3261 UDP response routing to Via sent-by without ``rport``.

The request is transmitted from one UDP port while its top Via advertises a
different listening port. A passing proxy chain returns the response to the
Via sent-by port and sends nothing to the packet-source port.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import selectors
import socket
import time
import uuid
from pathlib import Path


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--local-address", required=True)
    result.add_argument("--sent-by-port", required=True, type=int)
    result.add_argument("--target-host", required=True)
    result.add_argument("--target-port", required=True, type=int)
    result.add_argument("--timeout-seconds", type=float, default=10.0)
    result.add_argument("--output", required=True, type=Path)
    return result


def main() -> int:
    args = parser().parse_args()
    branch = f"z9hG4bK-via-{uuid.uuid4().hex}"
    call_id = f"via-response-{uuid.uuid4().hex}@example.test"
    request = (
        "OPTIONS sip:probe@example.test SIP/2.0\r\n"
        f"Via: SIP/2.0/UDP {args.local_address}:{args.sent_by_port};branch={branch}\r\n"
        f"From: <sip:probe@{args.local_address}>;tag={uuid.uuid4().hex}\r\n"
        "To: <sip:probe@example.test>\r\n"
        f"Call-ID: {call_id}\r\n"
        "CSeq: 1 OPTIONS\r\n"
        "X-Interop-Scenario: via-response-destination\r\n"
        "Max-Forwards: 70\r\n"
        "Content-Length: 0\r\n"
        "\r\n"
    ).encode("ascii")

    sent_by = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    source = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sent_by.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    sent_by.bind((args.local_address, args.sent_by_port))
    source.bind((args.local_address, 0))
    source_port = source.getsockname()[1]
    sent_by.setblocking(False)
    source.setblocking(False)

    selector = selectors.DefaultSelector()
    selector.register(sent_by, selectors.EVENT_READ, "sent_by")
    selector.register(source, selectors.EVENT_READ, "source")
    source.sendto(request, (args.target_host, args.target_port))

    deadline = time.monotonic() + args.timeout_seconds
    observations: list[dict[str, object]] = []
    received_on_sent_by = False
    received_on_source = False
    response_sha256 = None
    while time.monotonic() < deadline:
        events = selector.select(max(0.0, deadline - time.monotonic()))
        if not events:
            break
        for key, _ in events:
            payload, peer = key.fileobj.recvfrom(65535)
            text = payload.decode("iso-8859-1", errors="replace")
            is_matching = (
                text.startswith("SIP/2.0 200 ")
                and f"Call-ID: {call_id}" in text
                and "CSeq: 1 OPTIONS" in text
            )
            observations.append(
                {
                    "socket": key.data,
                    "peer": f"{peer[0]}:{peer[1]}",
                    "bytes": len(payload),
                    "matching_response": is_matching,
                }
            )
            if is_matching:
                response_sha256 = hashlib.sha256(payload).hexdigest()
                if key.data == "sent_by":
                    received_on_sent_by = True
                else:
                    received_on_source = True
        if received_on_sent_by:
            # Keep a short negative-observation window for an erroneous
            # duplicate delivered to the packet-source socket.
            deadline = min(deadline, time.monotonic() + 0.25)

    sent_by.close()
    source.close()
    passed = received_on_sent_by and not received_on_source
    payload = {
        "schema": "rvoip-sip-proxy-udp-via-probe-v1",
        "status": "PASS" if passed else "FAIL",
        "request": {
            "sha256": hashlib.sha256(request).hexdigest(),
            "source_port": source_port,
            "via_sent_by_port": args.sent_by_port,
            "contains_rport": False,
        },
        "response": {
            "sha256": response_sha256,
            "received_on_via_sent_by": received_on_sent_by,
            "received_on_packet_source": received_on_source,
        },
        "observations": observations,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
