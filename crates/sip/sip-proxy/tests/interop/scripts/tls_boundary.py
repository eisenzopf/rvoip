#!/usr/bin/env python3
"""Hostname-verifying TLS boundary for actual SIPp interoperability traffic.

SIPp 3.7.7 validates certificate chains but does not expose RFC 6125 hostname
verification. This boundary keeps SIPp on loopback TCP and makes every external
hop verified mTLS:

* client mode: SIPp UAC TCP -> boundary -> verified mTLS to the first proxy;
* server mode: last proxy -> verified mTLS boundary -> SIPp UAS TCP.

Only bounded identity metadata is logged. Certificate bodies and keys are
never emitted.
"""

from __future__ import annotations

import argparse
import hashlib
import selectors
import signal
import socket
import ssl
import sys
import threading
from dataclasses import dataclass


def parse_address(value: str) -> tuple[str, int]:
    host, separator, port_text = value.rpartition(":")
    if not separator or not host:
        raise argparse.ArgumentTypeError(
            f"address must be host:port, observed {value!r}"
        )
    try:
        port = int(port_text)
    except ValueError as error:
        raise argparse.ArgumentTypeError("port must be an integer") from error
    if not 0 < port <= 65535:
        raise argparse.ArgumentTypeError("port must be in 1..65535")
    return host, port


def exact_dns_sans(certificate: dict) -> list[str]:
    return [
        value
        for kind, value in certificate.get("subjectAltName", ())
        if kind == "DNS"
    ]


def verify_exact_peer_identity(
    tls_socket: ssl.SSLSocket, expected_dns: str
) -> tuple[str, str]:
    certificate = tls_socket.getpeercert()
    dns_sans = exact_dns_sans(certificate)
    non_dns_sans = [
        (kind, value)
        for kind, value in certificate.get("subjectAltName", ())
        if kind != "DNS"
    ]
    if dns_sans != [expected_dns] or non_dns_sans:
        raise ssl.SSLCertVerificationError(
            f"peer must present exactly DNS SAN {expected_dns!r}"
        )
    der = tls_socket.getpeercert(binary_form=True)
    if not der:
        raise ssl.SSLCertVerificationError(
            "peer did not expose a verified leaf certificate"
        )
    return dns_sans[0], hashlib.sha256(der).hexdigest()


def relay(left: socket.socket, right: socket.socket) -> None:
    selector = selectors.DefaultSelector()
    selector.register(left, selectors.EVENT_READ, right)
    selector.register(right, selectors.EVENT_READ, left)
    try:
        while True:
            for key, _events in selector.select(timeout=1):
                source = key.fileobj
                destination = key.data
                data = source.recv(64 * 1024)
                if not data:
                    return
                destination.sendall(data)
    finally:
        selector.close()


@dataclass(frozen=True)
class BoundaryConfig:
    mode: str
    listen: tuple[str, int]
    destination: tuple[str, int]
    ca: str
    certificate: str
    private_key: str
    expected_peer_dns: str
    server_identity: str


class TlsBoundary:
    def __init__(self, config: BoundaryConfig) -> None:
        self.config = config
        self.stop = threading.Event()
        self.listener: socket.socket | None = None
        self.threads: list[threading.Thread] = []
        self.log_lock = threading.Lock()
        self.context = self._build_context()

    def _build_context(self) -> ssl.SSLContext:
        if self.config.mode == "client":
            context = ssl.create_default_context(
                ssl.Purpose.SERVER_AUTH, cafile=self.config.ca
            )
            context.check_hostname = True
            context.verify_mode = ssl.CERT_REQUIRED
        else:
            context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            context.verify_mode = ssl.CERT_REQUIRED
            context.load_verify_locations(cafile=self.config.ca)

            def require_exact_sni(
                tls_socket: ssl.SSLSocket,
                server_name: str | None,
                _context: ssl.SSLContext,
            ) -> None:
                tls_socket.interop_server_name = server_name  # type: ignore[attr-defined]
                if server_name != self.config.server_identity:
                    raise ssl.SSLError("unrecognized_name")

            context.sni_callback = require_exact_sni
        context.minimum_version = ssl.TLSVersion.TLSv1_2
        context.maximum_version = ssl.TLSVersion.TLSv1_2
        context.load_cert_chain(
            certfile=self.config.certificate,
            keyfile=self.config.private_key,
        )
        return context

    def log(self, message: str) -> None:
        with self.log_lock:
            print(message, flush=True)

    def _connect_client_tls(self) -> ssl.SSLSocket:
        raw = socket.create_connection(self.config.destination, timeout=10)
        try:
            tls_socket = self.context.wrap_socket(
                raw,
                server_hostname=self.config.expected_peer_dns,
            )
        except BaseException:
            raw.close()
            raise
        tls_socket.settimeout(None)
        return tls_socket

    def _accept_server_tls(self, raw: socket.socket) -> ssl.SSLSocket:
        tls_socket = self.context.wrap_socket(raw, server_side=True)
        observed_sni = getattr(tls_socket, "interop_server_name", None)
        if observed_sni != self.config.server_identity:
            tls_socket.close()
            raise ssl.SSLError("required SNI was not observed")
        tls_socket.settimeout(None)
        return tls_socket

    def handle(self, inbound: socket.socket) -> None:
        peer: socket.socket | None = None
        tls_socket: ssl.SSLSocket | None = None
        try:
            inbound.settimeout(10)
            if self.config.mode == "client":
                tls_socket = self._connect_client_tls()
                peer = tls_socket
                plaintext = inbound
                plaintext.settimeout(None)
                server_name = self.config.expected_peer_dns
            else:
                tls_socket = self._accept_server_tls(inbound)
                peer = socket.create_connection(self.config.destination, timeout=10)
                peer.settimeout(None)
                plaintext = peer
                server_name = self.config.server_identity

            dns_san, fingerprint = verify_exact_peer_identity(
                tls_socket, self.config.expected_peer_dns
            )
            protocol = tls_socket.version()
            if protocol != "TLSv1.2":
                raise ssl.SSLError(f"unexpected TLS protocol {protocol!r}")
            self.log(
                "TLS_BOUNDARY_ACCEPTED "
                f"mode={self.config.mode} "
                f"expected_peer_dns={self.config.expected_peer_dns} "
                f"presented_peer_dns={dns_san} "
                f"server_name={server_name} "
                f"leaf_certificate_sha256={fingerprint} "
                f"protocol={protocol}"
            )
            relay(plaintext, tls_socket if self.config.mode == "server" else peer)
        except ssl.SSLCertVerificationError:
            self.log(
                "TLS_BOUNDARY_REJECTED "
                f"mode={self.config.mode} reason=certificate_identity"
            )
        except ssl.SSLError:
            self.log(
                f"TLS_BOUNDARY_REJECTED mode={self.config.mode} reason=tls"
            )
        except OSError:
            if not self.stop.is_set():
                self.log(
                    f"TLS_BOUNDARY_REJECTED mode={self.config.mode} reason=io"
                )
        finally:
            for stream in (peer, tls_socket, inbound):
                if stream is not None:
                    try:
                        stream.close()
                    except OSError:
                        pass

    def serve(self) -> None:
        family = socket.AF_INET6 if ":" in self.config.listen[0] else socket.AF_INET
        listener = socket.socket(family, socket.SOCK_STREAM)
        self.listener = listener
        listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        listener.bind(self.config.listen)
        listener.listen(128)
        listener.settimeout(0.5)
        self.log(
            "TLS_BOUNDARY_READY "
            f"mode={self.config.mode} "
            f"expected_peer_dns={self.config.expected_peer_dns} "
            f"server_identity={self.config.server_identity}"
        )
        while not self.stop.is_set():
            try:
                inbound, _address = listener.accept()
            except TimeoutError:
                continue
            except OSError:
                if self.stop.is_set():
                    break
                raise
            thread = threading.Thread(
                target=self.handle, args=(inbound,), daemon=True
            )
            thread.start()
            self.threads.append(thread)
        for thread in self.threads:
            thread.join(timeout=2)

    def shutdown(self) -> None:
        self.stop.set()
        if self.listener is not None:
            self.listener.close()


def parse_args() -> BoundaryConfig:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", required=True, choices=("client", "server"))
    parser.add_argument("--listen", required=True, type=parse_address)
    parser.add_argument("--destination", required=True, type=parse_address)
    parser.add_argument("--ca", required=True)
    parser.add_argument("--certificate", required=True)
    parser.add_argument("--private-key", required=True)
    parser.add_argument("--expected-peer-dns", required=True)
    parser.add_argument(
        "--server-identity", default="sipp.proxy.test"
    )
    args = parser.parse_args()
    return BoundaryConfig(
        mode=args.mode,
        listen=args.listen,
        destination=args.destination,
        ca=args.ca,
        certificate=args.certificate,
        private_key=args.private_key,
        expected_peer_dns=args.expected_peer_dns,
        server_identity=args.server_identity,
    )


def main() -> int:
    boundary = TlsBoundary(parse_args())

    def stop(_signal: int, _frame: object) -> None:
        boundary.shutdown()

    signal.signal(signal.SIGINT, stop)
    signal.signal(signal.SIGTERM, stop)
    try:
        boundary.serve()
    except OSError as error:
        print(
            f"TLS_BOUNDARY_FATAL reason=io class={type(error).__name__}",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
