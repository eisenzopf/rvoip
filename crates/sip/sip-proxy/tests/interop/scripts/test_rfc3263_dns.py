#!/usr/bin/env python3
"""Wire-level tests for the deterministic RFC 3263 DNS authority."""

from __future__ import annotations

import ipaddress
import struct
import unittest

import rfc3263_dns


def query(name: str, record_type: int) -> bytes:
    return (
        struct.pack("!HHHHHH", 0x1234, 0x0100, 1, 0, 0, 0)
        + rfc3263_dns.encode_name(name)
        + struct.pack("!HH", record_type, rfc3263_dns.CLASS_IN)
    )


class Rfc3263DnsTests(unittest.TestCase):
    def build(self, name: str, record_type: int):
        return rfc3263_dns.build_response(
            query(name, record_type),
            zone="failover.interop.test",
            address=ipaddress.IPv4Address("192.0.2.10"),
            dead_port=25999,
            live_port=25081,
        )

    def test_srv_answers_are_dead_then_live_by_priority(self) -> None:
        response, evidence = self.build(
            "_sip._tcp.failover.interop.test",
            rfc3263_dns.TYPE_SRV,
        )
        identifier, flags, questions, answers, _ns, _ar = struct.unpack(
            "!HHHHHH", response[:12]
        )
        self.assertEqual(identifier, 0x1234)
        self.assertEqual(flags & 0x8400, 0x8400)
        self.assertEqual((questions, answers), (1, 2))
        self.assertEqual(
            evidence["answers"],
            [
                {
                    "type": "SRV",
                    "priority": 10,
                    "weight": 0,
                    "port": 25999,
                    "target": "dead.failover.interop.test",
                },
                {
                    "type": "SRV",
                    "priority": 20,
                    "weight": 0,
                    "port": 25081,
                    "target": "live.failover.interop.test",
                },
            ],
        )

    def test_wire_service_labels_do_not_weaken_host_validation(self) -> None:
        encoded = rfc3263_dns.encode_name(
            "_sip._tcp.failover.interop.test"
        )
        decoded, offset = rfc3263_dns.decode_name(encoded, 0)
        self.assertEqual(decoded, "_sip._tcp.failover.interop.test")
        self.assertEqual(offset, len(encoded))
        with self.assertRaises(ValueError):
            rfc3263_dns.normalize_name("_sip.failover.interop.test")

    def test_srv_targets_have_a_records(self) -> None:
        for name in (
            "dead.failover.interop.test",
            "live.failover.interop.test",
        ):
            _response, evidence = self.build(name, rfc3263_dns.TYPE_A)
            self.assertEqual(
                evidence["answers"],
                [{"type": "A", "name": name, "address": "192.0.2.10"}],
            )

    def test_naptr_and_aaaa_are_authoritative_no_data(self) -> None:
        for record_type in (rfc3263_dns.TYPE_NAPTR, 28):
            response, evidence = self.build(
                "failover.interop.test", record_type
            )
            self.assertEqual(struct.unpack("!H", response[6:8])[0], 0)
            self.assertEqual(evidence["answers"], [])

    def test_rejects_malformed_question(self) -> None:
        with self.assertRaises(rfc3263_dns.DnsError):
            rfc3263_dns.build_response(
                b"\x00" * 8,
                zone="failover.interop.test",
                address=ipaddress.IPv4Address("192.0.2.10"),
                dead_port=25999,
                live_port=25081,
            )


if __name__ == "__main__":
    unittest.main()
