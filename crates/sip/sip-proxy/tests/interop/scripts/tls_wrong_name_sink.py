#!/usr/bin/env python3
"""Diagnostic valid-chain/wrong-name TLS sink that detects SIP byte leakage.

This helper is deliberately not part of the release proof. It exposed that
Kamailio's outbound connection event can run too late to enforce hostname
identity before application data reaches the server, which is why release
traffic uses the gate-owned hostname-verifying boundary instead.
"""

from __future__ import annotations

import argparse
import socket
import ssl
import sys

from tls_boundary import parse_address, verify_exact_peer_identity


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--listen", required=True, type=parse_address)
    parser.add_argument("--ca", required=True)
    parser.add_argument("--certificate", required=True)
    parser.add_argument("--private-key", required=True)
    parser.add_argument("--expected-client-dns", required=True)
    parser.add_argument("--expected-sni", required=True)
    parser.add_argument("--accept-timeout", type=float, default=10)
    parser.add_argument("--data-timeout", type=float, default=2)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_2
    context.maximum_version = ssl.TLSVersion.TLSv1_2
    context.verify_mode = ssl.CERT_REQUIRED
    context.load_verify_locations(cafile=args.ca)
    context.load_cert_chain(args.certificate, args.private_key)

    def require_sni(
        tls_socket: ssl.SSLSocket,
        server_name: str | None,
        _context: ssl.SSLContext,
    ) -> None:
        tls_socket.interop_server_name = server_name  # type: ignore[attr-defined]

    context.sni_callback = require_sni
    family = socket.AF_INET6 if ":" in args.listen[0] else socket.AF_INET
    listener = socket.socket(family, socket.SOCK_STREAM)
    listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    listener.bind(args.listen)
    listener.listen(1)
    listener.settimeout(args.accept_timeout)
    print(
        "TLS_WRONG_NAME_SINK_READY "
        f"expected_sni={args.expected_sni} "
        f"expected_client_dns={args.expected_client_dns}",
        flush=True,
    )
    try:
        raw, _address = listener.accept()
        with raw:
            with context.wrap_socket(raw, server_side=True) as tls_socket:
                if (
                    getattr(tls_socket, "interop_server_name", None)
                    != args.expected_sni
                ):
                    observed_sni = getattr(
                        tls_socket, "interop_server_name", None
                    )
                    print(
                        "TLS_WRONG_NAME_SINK_FATAL "
                        "class=SniMismatch "
                        f"observed_sni={observed_sni or 'none'}",
                        file=sys.stderr,
                        flush=True,
                    )
                    return 1
                client_dns, client_fingerprint = verify_exact_peer_identity(
                    tls_socket, args.expected_client_dns
                )
                protocol = tls_socket.version()
                tls_socket.settimeout(args.data_timeout)
                try:
                    application_data = tls_socket.recv(64 * 1024)
                except TimeoutError:
                    application_data = b""
                if application_data:
                    print(
                        "TLS_WRONG_NAME_SINK_RESULT "
                        "result=APPLICATION_DATA_LEAK "
                        f"application_bytes={len(application_data)}",
                        flush=True,
                    )
                    return 1
                print(
                    "TLS_WRONG_NAME_SINK_RESULT "
                    "result=NO_APPLICATION_DATA "
                    f"expected_sni={args.expected_sni} "
                    f"client_dns={client_dns} "
                    f"client_leaf_certificate_sha256={client_fingerprint} "
                    f"protocol={protocol} "
                    "application_bytes=0",
                    flush=True,
                )
                return 0
    except (OSError, ssl.SSLError) as error:
        print(
            "TLS_WRONG_NAME_SINK_FATAL "
            f"class={type(error).__name__}",
            file=sys.stderr,
            flush=True,
        )
        return 1
    finally:
        listener.close()


if __name__ == "__main__":
    raise SystemExit(main())
