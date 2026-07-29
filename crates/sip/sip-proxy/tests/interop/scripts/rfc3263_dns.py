#!/usr/bin/env python3
"""Minimal authoritative DNS fixture for real RFC 3263 interoperability.

The fixture is deliberately small and deterministic: an SRV query for
``_sip._tcp.<zone>`` returns a dead candidate at priority 10 and a live
candidate at priority 20. A queries for both SRV targets return the configured
IPv4 address. NAPTR and AAAA return authoritative NOERROR/no-data so rvoip's
production Hickory resolver follows the RFC 3263 TCP SRV fallback path.
"""

from __future__ import annotations

import argparse
import ipaddress
import json
import signal
import socket
import struct
from pathlib import Path
from typing import Any


SCHEMA = "rvoip-sip-proxy-rfc3263-dns-v1"
TYPE_A = 1
TYPE_SRV = 33
TYPE_NAPTR = 35
CLASS_IN = 1


class DnsError(RuntimeError):
    """Raised for malformed or unsupported DNS wire input."""


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


def normalize_name(value: str) -> str:
    value = value.rstrip(".").lower()
    if not value or len(value) > 253:
        raise ValueError("DNS name must be 1..253 octets")
    labels = value.split(".")
    if any(
        not label
        or len(label) > 63
        or not all(character.isalnum() or character == "-" for character in label)
        for label in labels
    ):
        raise ValueError(f"invalid DNS name: {value!r}")
    return value


def normalize_wire_name(value: str) -> str:
    """Normalize a DNS wire name, permitting leading-underscore service labels."""

    value = value.rstrip(".").lower()
    if not value or len(value) > 253:
        raise ValueError("DNS name must be 1..253 octets")
    labels = value.split(".")
    for label in labels:
        if not label or len(label) > 63:
            raise ValueError(f"invalid DNS name: {value!r}")
        host_label = label[1:] if label.startswith("_") else label
        if not host_label or not all(
            character.isalnum() or character == "-" for character in host_label
        ):
            raise ValueError(f"invalid DNS name: {value!r}")
    return value


def encode_name(value: str) -> bytes:
    return b"".join(
        bytes((len(label),)) + label.encode("ascii")
        for label in normalize_wire_name(value).split(".")
    ) + b"\x00"


def decode_name(packet: bytes, offset: int) -> tuple[str, int]:
    labels: list[str] = []
    next_offset: int | None = None
    visited: set[int] = set()
    while True:
        if offset >= len(packet):
            raise DnsError("truncated DNS name")
        length = packet[offset]
        if length & 0xC0 == 0xC0:
            if offset + 1 >= len(packet):
                raise DnsError("truncated DNS compression pointer")
            pointer = ((length & 0x3F) << 8) | packet[offset + 1]
            if pointer in visited:
                raise DnsError("DNS compression pointer loop")
            visited.add(pointer)
            next_offset = next_offset or offset + 2
            offset = pointer
            continue
        if length & 0xC0:
            raise DnsError("unsupported DNS label encoding")
        offset += 1
        if length == 0:
            return ".".join(labels).lower(), next_offset or offset
        if offset + length > len(packet):
            raise DnsError("truncated DNS label")
        labels.append(packet[offset : offset + length].decode("ascii"))
        offset += length


def resource_record(name: bytes, record_type: int, rdata: bytes) -> bytes:
    return (
        name
        + struct.pack("!HHIH", record_type, CLASS_IN, 60, len(rdata))
        + rdata
    )


def build_response(
    query: bytes,
    *,
    zone: str,
    address: ipaddress.IPv4Address,
    dead_port: int,
    live_port: int,
) -> tuple[bytes, dict[str, Any]]:
    if len(query) < 12:
        raise DnsError("truncated DNS header")
    identifier, flags, questions, answers, authorities, additionals = struct.unpack(
        "!HHHHHH", query[:12]
    )
    del answers, authorities, additionals
    if questions != 1:
        raise DnsError("fixture requires exactly one DNS question")
    qname, offset = decode_name(query, 12)
    if offset + 4 > len(query):
        raise DnsError("truncated DNS question")
    qtype, qclass = struct.unpack("!HH", query[offset : offset + 4])
    if qclass != CLASS_IN:
        raise DnsError("fixture supports only IN class")
    question = query[12 : offset + 4]

    normalized_zone = normalize_name(zone)
    service = f"_sip._tcp.{normalized_zone}"
    dead_name = f"dead.{normalized_zone}"
    live_name = f"live.{normalized_zone}"
    records: list[bytes] = []
    record_descriptions: list[dict[str, Any]] = []
    owner = b"\xc0\x0c"
    if qtype == TYPE_SRV and qname == service:
        for priority, port, target in (
            (10, dead_port, dead_name),
            (20, live_port, live_name),
        ):
            records.append(
                resource_record(
                    owner,
                    TYPE_SRV,
                    struct.pack("!HHH", priority, 0, port)
                    + encode_name(target),
                )
            )
            record_descriptions.append(
                {
                    "type": "SRV",
                    "priority": priority,
                    "weight": 0,
                    "port": port,
                    "target": target,
                }
            )
    elif qtype == TYPE_A and qname in {dead_name, live_name}:
        records.append(resource_record(owner, TYPE_A, address.packed))
        record_descriptions.append(
            {"type": "A", "name": qname, "address": str(address)}
        )

    # QR=1, AA=1, preserve RD, NOERROR. The fixture is authoritative and is
    # intentionally not recursive.
    response_flags = 0x8400 | (flags & 0x0100)
    response = (
        struct.pack(
            "!HHHHHH",
            identifier,
            response_flags,
            1,
            len(records),
            0,
            0,
        )
        + question
        + b"".join(records)
    )
    return response, {
        "schema": SCHEMA,
        "query_name": qname,
        "query_type": qtype,
        "answers": record_descriptions,
    }


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--listen", type=parse_address, required=True)
    result.add_argument("--zone", required=True)
    result.add_argument("--address", type=ipaddress.IPv4Address, required=True)
    result.add_argument("--dead-port", type=int, required=True)
    result.add_argument("--live-port", type=int, required=True)
    result.add_argument("--log", type=Path, required=True)
    return result


def main() -> int:
    args = parser().parse_args()
    zone = normalize_name(args.zone)
    if any(not 0 < port <= 65535 for port in (args.dead_port, args.live_port)):
        raise SystemExit("candidate ports must be in 1..65535")
    if args.dead_port == args.live_port:
        raise SystemExit("dead and live candidate ports must differ")
    args.log.parent.mkdir(parents=True, exist_ok=True)

    running = True

    def stop(_signal: int, _frame: object) -> None:
        nonlocal running
        running = False

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)

    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as server:
        server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        server.bind(args.listen)
        server.settimeout(0.25)
        print(
            "RFC3263_DNS_READY "
            f"listen={args.listen[0]}:{args.listen[1]} zone={zone}",
            flush=True,
        )
        with args.log.open("a", encoding="utf-8") as log:
            while running:
                try:
                    query, source = server.recvfrom(4096)
                except TimeoutError:
                    continue
                try:
                    response, evidence = build_response(
                        query,
                        zone=zone,
                        address=args.address,
                        dead_port=args.dead_port,
                        live_port=args.live_port,
                    )
                    server.sendto(response, source)
                    evidence["source"] = f"{source[0]}:{source[1]}"
                    evidence["status"] = "answered"
                except (DnsError, UnicodeError, ValueError) as error:
                    evidence = {
                        "schema": SCHEMA,
                        "source": f"{source[0]}:{source[1]}",
                        "status": "rejected",
                        "error": type(error).__name__,
                    }
                log.write(json.dumps(evidence, sort_keys=True) + "\n")
                log.flush()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
