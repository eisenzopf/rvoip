#!/usr/bin/env python3
"""Tests for the SIPp hostname-verifying TLS boundary."""

from __future__ import annotations

import hashlib
import importlib.util
import socket
import sys
import threading
import time
import unittest
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parent
MODULE_SPEC = importlib.util.spec_from_file_location(
    "tls_boundary", SCRIPT_DIR / "tls_boundary.py"
)
assert MODULE_SPEC and MODULE_SPEC.loader
boundary_module = importlib.util.module_from_spec(MODULE_SPEC)
sys.modules[MODULE_SPEC.name] = boundary_module
MODULE_SPEC.loader.exec_module(boundary_module)


class FakeTlsSocket:
    def __init__(self, stream: socket.socket, identity: str) -> None:
        self.stream = stream
        self.identity = identity
        self.der = b"verified-test-leaf"

    def getpeercert(self, binary_form: bool = False):
        if binary_form:
            return self.der
        return {"subjectAltName": (("DNS", self.identity),)}

    def version(self) -> str:
        return "TLSv1.2"

    def settimeout(self, value) -> None:
        self.stream.settimeout(value)

    def recv(self, size: int) -> bytes:
        return self.stream.recv(size)

    def sendall(self, data: bytes) -> None:
        self.stream.sendall(data)

    def fileno(self) -> int:
        return self.stream.fileno()

    def close(self) -> None:
        self.stream.close()


class TlsBoundaryTests(unittest.TestCase):
    def test_kamailio_boundary_socket_never_advertises_wildcard_via(self) -> None:
        template = (
            SCRIPT_DIR.parent / "config/kamailio-tls.cfg.in"
        ).read_text(encoding="utf-8")
        self.assertIn(
            "listen=tcp:0.0.0.0:__TCP_EGRESS_PORT__ "
            "advertise __PUBLIC_HOST__:__PUBLIC_PORT__",
            template,
        )

    def test_client_relay_survives_more_than_ten_seconds_idle(self) -> None:
        application, inbound = socket.socketpair()
        outbound, remote = socket.socketpair()
        fake_tls = FakeTlsSocket(outbound, "rvoip.proxy.test")
        config = boundary_module.BoundaryConfig(
            mode="client",
            listen=("127.0.0.1", 1),
            destination=("127.0.0.1", 2),
            ca="unused",
            certificate="unused",
            private_key="unused",
            expected_peer_dns="rvoip.proxy.test",
            server_identity="sipp.proxy.test",
        )
        boundary = object.__new__(boundary_module.TlsBoundary)
        boundary.config = config
        boundary.stop = threading.Event()
        logs: list[str] = []
        boundary.log = logs.append
        boundary._connect_client_tls = lambda: fake_tls

        worker = threading.Thread(target=boundary.handle, args=(inbound,))
        worker.start()
        try:
            application.sendall(b"before-idle")
            self.assertEqual(remote.recv(64), b"before-idle")
            time.sleep(10.25)
            application.sendall(b"after-idle")
            self.assertEqual(remote.recv(64), b"after-idle")
        finally:
            application.close()
            remote.close()
            worker.join(timeout=2)
        self.assertFalse(worker.is_alive())
        self.assertTrue(
            any(line.startswith("TLS_BOUNDARY_ACCEPTED") for line in logs)
        )
        self.assertFalse(
            any(line.startswith("TLS_BOUNDARY_REJECTED") for line in logs)
        )

    def test_exact_dns_san_and_fingerprint_are_required(self) -> None:
        local, remote = socket.socketpair()
        try:
            tls_socket = FakeTlsSocket(local, "peer.proxy.test")
            identity, fingerprint = (
                boundary_module.verify_exact_peer_identity(
                    tls_socket, "peer.proxy.test"
                )
            )
            self.assertEqual(identity, "peer.proxy.test")
            self.assertEqual(
                fingerprint, hashlib.sha256(tls_socket.der).hexdigest()
            )
            with self.assertRaises(Exception):
                boundary_module.verify_exact_peer_identity(
                    tls_socket, "wrong.proxy.test"
                )
        finally:
            local.close()
            remote.close()


if __name__ == "__main__":
    unittest.main()
