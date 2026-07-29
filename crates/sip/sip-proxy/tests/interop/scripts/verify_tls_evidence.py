#!/usr/bin/env python3
"""Derive TLS interop proof from public certificates and raw endpoint logs.

The harness does not assert that TLS passed. It records unmodified OpenSSL,
rvoip, and independent-peer output, then this verifier proves the required
properties from those files. The beta reporter can import ``verify_row`` and
recompute the result instead of trusting a harness-authored boolean.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path
from typing import Any


SCHEMA = "rvoip-sip-proxy-interop-tls-evidence-v2"
RESULT_BASENAME = "tls-verifier-result.json"
PACKET_SCHEMA = "rvoip-sip-proxy-interop-packet-evidence-v1"
TLS_PACKET_SCENARIOS = (
    "options-readiness",
    "invite-success",
    "matched-cancel-before-provisional",
    "matched-cancel-after-provisional",
    "cancel-retransmission",
    "unmatched-cancel",
    "ack-non2xx",
    "via-response-destination",
    "message-body-content-length",
    "sips-routing",
)
BASE_PACKET_ASSERTIONS = {
    "scenario-call-id-observed",
    "required-methods-observed",
    "required-statuses-observed",
    "rvoip-via-observed",
    "peer-via-observed",
    "tls-packets-observed",
    "tls-application-data-on-every-encrypted-hop",
    "tls-handshake-sni-valid-when-observed",
    "tls-handshake-certificates-observed-when-initiated",
}
SCENARIO_PACKET_ASSERTIONS = {
    "invite-success": {
        "invite-dialog-response-contact-and-record-route-set",
        "invite-dialog-uac-ack-bye-use-contact-and-reversed-route-set",
        "invite-dialog-downstream-ack-bye-reach-contact-with-routes-consumed",
    },
    "sips-routing": {
        "sips-request-uri-at-uac-boundary",
        "sips-request-uri-at-uas-boundary",
        "sips-request-preserved-end-to-end",
        "sips-both-proxy-vias-observed",
        "no-plaintext-sip-on-external-tls-ports",
        "sips-options-success-observed",
    },
}
IDENTITIES = {
    "rvoip": "rvoip.proxy.test",
    "sipp": "sipp.proxy.test",
    "kamailio": "kamailio.proxy.test",
    "opensips": "opensips.proxy.test",
}
PUBLIC_CERTIFICATES = ("ca", "rvoip", "peer", "sipp")
RAW_INSPECT_BASENAMES = {"container-inspect.json", "image-inspect.json"}
PRIVATE_KEY_PEM = re.compile(
    rb"-----BEGIN (?:RSA |EC |ENCRYPTED |OPENSSH )?PRIVATE KEY-----"
)
ACCEPTED_PEER_RE = re.compile(
    r"^RVOIP_TLS_PEER_ACCEPTED "
    r"direction=inbound transport=tls "
    r"leaf_certificate_sha256=([0-9a-f]{64}) "
    r"presented_chain_len=([1-9][0-9]*)$",
    re.MULTILINE,
)
BOUNDARY_ACCEPTED_RE = re.compile(
    r"^TLS_BOUNDARY_ACCEPTED "
    r"mode=(client|server) "
    r"expected_peer_dns=([A-Za-z0-9.-]+) "
    r"presented_peer_dns=([A-Za-z0-9.-]+) "
    r"server_name=([A-Za-z0-9.-]+) "
    r"leaf_certificate_sha256=([0-9a-f]{64}) "
    r"protocol=(TLSv1\.2)$",
    re.MULTILINE,
)
OPENSIPS_PROVENANCE_SCHEMA = (
    "rvoip-sip-proxy-interop-opensips-tls-image-v1"
)
OPENSIPS_BASE_DIGEST = (
    "sha256:eba1396b438a7f8a9d33c17017aae4670cb43361eb7130359240cf85fc3e6979"
)
OPENSIPS_DOCKERFILE = (
    "crates/sip/sip-proxy/tests/interop/images/opensips-tls/Dockerfile"
)
OPENSIPS_PACKAGES = {
    "opensips": {"version": "3.6.7-1"},
    "opensips-tls-module": {
        "version": "3.6.7-1",
        "deb_sha256": (
            "685f704faf2ce6b9015a0c059a32333bf789cabd8a467c7068aa1cea363de799"
        ),
    },
    "opensips-tlsmgm-module": {
        "version": "3.6.7-1",
        "deb_sha256": (
            "20d83193399a9ee02b8c3f1cc2fbe311231ec22c0b43656add63f77110a68545"
        ),
    },
    "opensips-tls-openssl-module": {
        "version": "3.6.7-1",
        "deb_sha256": (
            "05e0c80de73b352390981352f08183f9acc3dd0187577688c4c174f94fd9bc68"
        ),
    },
}
OPENSIPS_MODULES = {
    "proto_tls.so": {
        "path": "/usr/lib/x86_64-linux-gnu/opensips/modules/proto_tls.so",
        "sha256": (
            "c0a3b92a2d64fc58c8d015338f8016d8fe59e9cc131b91f2774f07281349f7f3"
        ),
    },
    "tls_mgm.so": {
        "path": "/usr/lib/x86_64-linux-gnu/opensips/modules/tls_mgm.so",
        "sha256": (
            "f2726bc731dbdf840bd40dbc7209eded8f16899034177b8dabed481da60662d2"
        ),
    },
    "tls_openssl.so": {
        "path": (
            "/usr/lib/x86_64-linux-gnu/opensips/modules/tls_openssl.so"
        ),
        "sha256": (
            "ec8dbf7164f48b8ae6298f4d014980f567ef4d7cc426ce39d83071b4a4ca956e"
        ),
    },
}


class VerificationError(RuntimeError):
    """The recorded evidence does not prove the TLS claim."""


def _required_file(path: Path, description: str) -> Path:
    if (
        not path.is_file()
        or path.is_symlink()
        or path.stat().st_size <= 0
    ):
        raise VerificationError(f"{description} is missing: {path}")
    return path


def _read_text(path: Path, description: str) -> str:
    return _required_file(path, description).read_text(
        encoding="utf-8", errors="replace"
    )


def _sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _run_openssl(
    openssl: str, arguments: list[str], description: str
) -> subprocess.CompletedProcess[bytes]:
    completed = subprocess.run(
        [openssl, *arguments],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode != 0:
        detail = completed.stderr.decode("utf-8", errors="replace").strip()
        raise VerificationError(f"{description} failed: {detail}")
    return completed


def _reject_sensitive_artifacts(row_dir: Path) -> None:
    for path in row_dir.rglob("*"):
        if path.is_symlink():
            raise VerificationError(f"TLS evidence contains a symlink: {path}")
        if not path.is_file():
            continue
        lowered = path.name.lower()
        if lowered in RAW_INSPECT_BASENAMES:
            raise VerificationError(
                f"TLS evidence contains prohibited raw inspect data: {path}"
            )
        if (
            lowered.endswith((".key", ".key.pem", ".p12", ".pfx"))
            or "private-key" in lowered
        ):
            raise VerificationError(
                f"TLS evidence contains a private-key artifact: {path}"
            )
        if path.stat().st_size <= 2 * 1024 * 1024:
            content = path.read_bytes()
            if PRIVATE_KEY_PEM.search(content):
                raise VerificationError(
                    f"TLS evidence contains private-key PEM data: {path}"
                )


def _certificate_record(
    openssl: str,
    certificate: Path,
    recorded_hash: Path,
    ca_certificate: Path,
    expected_identity: str | None,
) -> dict[str, Any]:
    der = _run_openssl(
        openssl,
        ["x509", "-in", str(certificate), "-outform", "DER"],
        f"DER conversion for {certificate.name}",
    ).stdout
    metadata = _run_openssl(
        openssl,
        [
            "x509",
            "-in",
            str(certificate),
            "-noout",
            "-subject",
            "-issuer",
            "-serial",
            "-dates",
            "-ext",
            "subjectAltName",
        ],
        f"certificate metadata for {certificate.name}",
    ).stdout.decode("utf-8", errors="replace")
    serial_match = re.search(r"^serial=([0-9A-Fa-f]+)$", metadata, re.MULTILINE)
    if serial_match is None:
        raise VerificationError(
            f"certificate metadata omitted serial for {certificate.name}"
        )

    if expected_identity is None:
        verification = _run_openssl(
            openssl,
            ["verify", "-CAfile", str(ca_certificate), str(certificate)],
            "test CA self-verification",
        )
        text = _run_openssl(
            openssl,
            ["x509", "-in", str(certificate), "-noout", "-text"],
            "test CA constraints",
        ).stdout.decode("utf-8", errors="replace")
        if "CA:TRUE" not in text:
            raise VerificationError("TLS test CA lacks a CA:TRUE constraint")
        sans: list[str] = []
    else:
        verification = _run_openssl(
            openssl,
            [
                "verify",
                "-CAfile",
                str(ca_certificate),
                "-verify_hostname",
                expected_identity,
                str(certificate),
            ],
            f"hostname verification for {certificate.name}",
        )
        _run_openssl(
            openssl,
            [
                "verify",
                "-CAfile",
                str(ca_certificate),
                "-purpose",
                "sslclient",
                str(certificate),
            ],
            f"client-certificate purpose for {certificate.name}",
        )
        sans = re.findall(r"DNS:([^,\s]+)", metadata)
        if sans != [expected_identity] or "IP Address:" in metadata:
            raise VerificationError(
                f"{certificate.name} must have exactly one DNS SAN "
                f"{expected_identity!r}; observed={sans!r}"
            )

    if b": OK" not in verification.stdout:
        raise VerificationError(
            f"OpenSSL did not confirm {certificate.name}: "
            f"{verification.stdout!r}"
        )
    derived_der_sha256 = _sha256_bytes(der)
    recorded_hash_text = _read_text(
        recorded_hash, f"recorded DER hash for {certificate.name}"
    )
    recorded_match = re.fullmatch(
        r"\s*([0-9a-fA-F]{64})(?:\s+\S+)?\s*", recorded_hash_text
    )
    if (
        recorded_match is None
        or recorded_match.group(1).lower() != derived_der_sha256
    ):
        raise VerificationError(
            f"recorded DER hash does not match {certificate.name}"
        )
    return {
        "der_sha256": derived_der_sha256,
        "pem_sha256": _sha256_file(certificate),
        "dns_sans": sans,
        "serial": serial_match.group(1).lower(),
    }


def _verify_positive_log(path: Path, expected_name: str) -> dict[str, Any]:
    text = _read_text(path, "positive TLS handshake log")
    required = (
        "CONNECTION ESTABLISHED",
        "Protocol version: TLSv1.2",
        "Verification: OK",
        f"Verified peername: {expected_name}",
    )
    missing = [value for value in required if value not in text]
    if missing:
        raise VerificationError(
            f"positive TLS handshake is incomplete in {path}: missing={missing}"
        )
    if "Peer certificate:" not in text:
        raise VerificationError(
            f"positive TLS handshake omitted peer-certificate metadata: {path}"
        )
    return {"log_sha256": _sha256_file(path), "protocol": "TLSv1.2"}


def _verify_negative_log(
    path: Path, description: str, patterns: tuple[str, ...]
) -> dict[str, Any]:
    text = _read_text(path, description)
    if not any(re.search(pattern, text, re.IGNORECASE) for pattern in patterns):
        raise VerificationError(
            f"{description} lacks the expected fail-closed error: {path}"
        )
    return {"log_sha256": _sha256_file(path), "rejected": description}


def _verify_boundary_log(
    path: Path,
    mode: str,
    expected_peer_dns: str,
    expected_server_name: str,
    expected_fingerprint: str,
) -> dict[str, Any]:
    text = _read_text(path, f"actual SIP TLS {mode} boundary log")
    if "TLS_BOUNDARY_REJECTED" in text or "TLS_BOUNDARY_FATAL" in text:
        raise VerificationError(
            f"actual SIP TLS {mode} boundary reported a rejection"
        )
    accepted = BOUNDARY_ACCEPTED_RE.findall(text)
    if not accepted:
        raise VerificationError(
            f"actual SIP TLS {mode} boundary accepted no connection"
        )
    for observed in accepted:
        (
            observed_mode,
            observed_expected_dns,
            observed_presented_dns,
            observed_server_name,
            observed_fingerprint,
            observed_protocol,
        ) = observed
        if (
            observed_mode != mode
            or observed_expected_dns != expected_peer_dns
            or observed_presented_dns != expected_peer_dns
            or observed_server_name != expected_server_name
            or observed_fingerprint != expected_fingerprint
            or observed_protocol != "TLSv1.2"
        ):
            raise VerificationError(
                f"actual SIP TLS {mode} boundary identity binding failed"
            )
    return {
        "log_sha256": _sha256_file(path),
        "connections": len(accepted),
        "expected_peer_dns": expected_peer_dns,
        "leaf_certificate_sha256": expected_fingerprint,
        "protocol": "TLSv1.2",
    }


def _verify_opensips_provenance(path: Path) -> dict[str, Any]:
    try:
        provenance = json.loads(
            _read_text(path, "OpenSIPS TLS image provenance")
        )
    except json.JSONDecodeError as error:
        raise VerificationError(
            f"OpenSIPS TLS image provenance is invalid JSON: {error}"
        ) from error
    workspace_root = Path(__file__).resolve().parents[6]
    dockerfile_path = workspace_root / OPENSIPS_DOCKERFILE
    expected_dockerfile_sha256 = _sha256_file(
        _required_file(dockerfile_path, "reviewed OpenSIPS TLS Dockerfile")
    )
    image = provenance.get("image")
    dockerfile = provenance.get("dockerfile")
    if (
        provenance.get("schema") != OPENSIPS_PROVENANCE_SCHEMA
        or provenance.get("result") != "PASS"
        or not isinstance(image, dict)
        or image.get("reference")
        != "rvoip/opensips-tls-interop:3.6.7-1"
        or re.fullmatch(
            r"sha256:[0-9a-f]{64}", str(image.get("id", ""))
        )
        is None
        or image.get("platform") != "linux/amd64"
        or image.get("base_digest") != OPENSIPS_BASE_DIGEST
        or not isinstance(dockerfile, dict)
        or dockerfile.get("relative_path") != OPENSIPS_DOCKERFILE
        or dockerfile.get("sha256") != expected_dockerfile_sha256
        or provenance.get("packages") != OPENSIPS_PACKAGES
        or provenance.get("modules") != OPENSIPS_MODULES
    ):
        raise VerificationError(
            "OpenSIPS TLS derived-image provenance does not match review"
        )
    return {
        "artifact_sha256": _sha256_file(path),
        "derived_image_id": image["id"],
        "dockerfile_sha256": expected_dockerfile_sha256,
    }


def _verify_packet_aggregate(
    row_dir: Path,
    peer: str,
    order: str,
    expected_identities: dict[str, str],
    certificates: dict[str, dict[str, Any]],
) -> tuple[dict[str, Any], list[Path]]:
    expected_sni = set(expected_identities.values())
    expected_leaf_hashes = {
        certificates[name]["der_sha256"] for name in ("rvoip", "peer", "sipp")
    }
    observed_sni: set[str] = set()
    observed_certificate_hashes: set[str] = set()
    scenario_records: dict[str, Any] = {}
    source_files: list[Path] = []
    seen_captures: set[str] = set()

    for scenario in TLS_PACKET_SCENARIOS:
        packet_path = _required_file(
            row_dir / "scenarios" / scenario / "packet-evidence.json",
            f"{scenario} TLS packet evidence",
        )
        try:
            packet = json.loads(packet_path.read_text())
        except json.JSONDecodeError as error:
            raise VerificationError(
                f"{scenario} TLS packet evidence is invalid JSON: {error}"
            ) from error
        if (
            packet.get("schema") != PACKET_SCHEMA
            or packet.get("scenario") != scenario
            or packet.get("status") != "PASS"
            or packet.get("peer") != peer
            or packet.get("order") != order
            or packet.get("transport") != "tls"
        ):
            raise VerificationError(
                f"{scenario} TLS packet evidence identity/status mismatch"
            )
        assertions = {
            item.get("name"): item
            for item in packet.get("assertions", [])
            if isinstance(item, dict) and isinstance(item.get("name"), str)
        }
        required_assertions = BASE_PACKET_ASSERTIONS | SCENARIO_PACKET_ASSERTIONS.get(
            scenario, set()
        )
        missing_or_failed = sorted(
            name
            for name in required_assertions
            if assertions.get(name, {}).get("passed") is not True
        )
        if missing_or_failed:
            raise VerificationError(
                f"{scenario} TLS packet assertions are missing or failed: "
                f"{missing_or_failed}"
            )

        scenario_sni = packet.get("observed_sni")
        if not isinstance(scenario_sni, list) or not all(
            isinstance(value, str) and value for value in scenario_sni
        ):
            raise VerificationError(f"{scenario} TLS SNI evidence is malformed")
        unexpected_sni = set(scenario_sni) - expected_sni
        if unexpected_sni:
            raise VerificationError(
                f"{scenario} TLS packet evidence contains unexpected SNI: "
                f"{sorted(unexpected_sni)}"
            )
        observed_sni.update(scenario_sni)

        scenario_certificates = packet.get("observed_certificate_sha256")
        if not isinstance(scenario_certificates, list) or not all(
            isinstance(value, str)
            and re.fullmatch(r"[0-9a-f]{64}", value) is not None
            for value in scenario_certificates
        ):
            raise VerificationError(
                f"{scenario} TLS certificate packet evidence is malformed"
            )
        observed_certificate_hashes.update(scenario_certificates)

        captures = packet.get("captures")
        if not isinstance(captures, list) or not captures:
            raise VerificationError(f"{scenario} TLS packet captures are missing")
        capture_records = []
        for capture in captures:
            if not isinstance(capture, dict):
                raise VerificationError(
                    f"{scenario} TLS packet capture metadata is malformed"
                )
            filename = capture.get("filename")
            recorded_hash = capture.get("sha256")
            recorded_bytes = capture.get("bytes")
            if (
                not isinstance(filename, str)
                or Path(filename).name != filename
                or not filename.startswith(f"{scenario}--")
                or not filename.endswith(".pcap")
                or filename in seen_captures
                or not isinstance(recorded_hash, str)
                or re.fullmatch(r"[0-9a-f]{64}", recorded_hash) is None
                or not isinstance(recorded_bytes, int)
                or isinstance(recorded_bytes, bool)
                or recorded_bytes <= 24
            ):
                raise VerificationError(
                    f"{scenario} TLS packet capture metadata is invalid"
                )
            capture_path = _required_file(
                row_dir / filename, f"{scenario} TLS packet capture"
            )
            if (
                capture_path.stat().st_size != recorded_bytes
                or _sha256_file(capture_path) != recorded_hash
            ):
                raise VerificationError(
                    f"{scenario} TLS packet capture hash/size mismatch"
                )
            seen_captures.add(filename)
            source_files.append(capture_path)
            capture_records.append(
                {
                    "filename": filename,
                    "sha256": recorded_hash,
                    "bytes": recorded_bytes,
                }
            )

        source_files.append(packet_path)
        scenario_records[scenario] = {
            "packet_evidence_sha256": _sha256_file(packet_path),
            "observed_sni": sorted(set(scenario_sni)),
            "observed_certificate_sha256": sorted(set(scenario_certificates)),
            "captures": capture_records,
        }

    if observed_sni != expected_sni:
        raise VerificationError(
            "row-level TLS packet SNI aggregate is incomplete: "
            f"expected={sorted(expected_sni)} observed={sorted(observed_sni)}"
        )
    if not expected_leaf_hashes <= observed_certificate_hashes:
        raise VerificationError(
            "row-level TLS packet certificate aggregate is incomplete: "
            f"missing={sorted(expected_leaf_hashes - observed_certificate_hashes)}"
        )
    return (
        {
            "scenarios": scenario_records,
            "expected_sni": sorted(expected_sni),
            "observed_sni": sorted(observed_sni),
            "expected_leaf_certificate_sha256": sorted(expected_leaf_hashes),
            "observed_certificate_sha256": sorted(observed_certificate_hashes),
            "capture_count": len(seen_captures),
        },
        source_files,
    )


def verify_row(
    row_dir: Path,
    peer: str,
    order: str,
    openssl: str = "openssl",
) -> dict[str, Any]:
    """Recompute the TLS result for one matrix row."""

    row_dir = row_dir.resolve()
    if peer not in {"kamailio", "opensips"}:
        raise VerificationError(f"unsupported TLS peer: {peer!r}")
    if order not in {"rvoip-first", "peer-first"}:
        raise VerificationError(f"unsupported TLS order: {order!r}")
    if not row_dir.is_dir() or row_dir.is_symlink():
        raise VerificationError(f"TLS row directory is missing: {row_dir}")

    _reject_sensitive_artifacts(row_dir)
    public_dir = row_dir / "tls-public"
    ca_certificate = _required_file(
        public_dir / "ca.pem", "public TLS CA certificate"
    )
    certificate_paths = {
        name: _required_file(
            public_dir / f"{name}.pem", f"public {name} certificate"
        )
        for name in PUBLIC_CERTIFICATES
    }
    certificate_hash_paths = {
        name: _required_file(
            public_dir / f"{name}.der.sha256",
            f"recorded DER hash for {name} certificate",
        )
        for name in PUBLIC_CERTIFICATES
    }
    expected_identities = {
        "rvoip": IDENTITIES["rvoip"],
        "peer": IDENTITIES[peer],
        "sipp": IDENTITIES["sipp"],
    }
    certificates = {
        "ca": _certificate_record(
            openssl,
            certificate_paths["ca"],
            certificate_hash_paths["ca"],
            ca_certificate,
            None,
        ),
        "rvoip": _certificate_record(
            openssl,
            certificate_paths["rvoip"],
            certificate_hash_paths["rvoip"],
            ca_certificate,
            expected_identities["rvoip"],
        ),
        "peer": _certificate_record(
            openssl,
            certificate_paths["peer"],
            certificate_hash_paths["peer"],
            ca_certificate,
            expected_identities["peer"],
        ),
        "sipp": _certificate_record(
            openssl,
            certificate_paths["sipp"],
            certificate_hash_paths["sipp"],
            ca_certificate,
            expected_identities["sipp"],
        ),
    }
    certificate_hashes = {
        record["der_sha256"] for record in certificates.values()
    }
    if len(certificate_hashes) != len(certificates):
        raise VerificationError("TLS evidence reused a certificate identity")
    packet_aggregate, packet_source_files = _verify_packet_aggregate(
        row_dir,
        peer,
        order,
        expected_identities,
        certificates,
    )

    verified_legs = {
        "rvoip-to-peer": _verify_positive_log(
            row_dir / "tls-rvoip-to-peer-positive.log",
            expected_identities["peer"],
        ),
        "peer-to-rvoip": _verify_positive_log(
            row_dir / "tls-peer-to-rvoip-positive.log",
            expected_identities["rvoip"],
        ),
    }
    negative_controls = {
        "rvoip-to-peer-wrong-name": _verify_negative_log(
            row_dir / "tls-rvoip-to-peer-wrong-name.log",
            "rvoip-to-peer wrong-name rejection",
            (r"hostname mismatch", r"does not match"),
        ),
        "rvoip-to-peer-wrong-ca": _verify_negative_log(
            row_dir / "tls-rvoip-to-peer-wrong-ca.log",
            "rvoip-to-peer wrong-CA rejection",
            (
                r"self-signed certificate",
                r"unable to get local issuer",
                r"certificate verify failed",
                r"unknown ca",
            ),
        ),
        "peer-to-rvoip-wrong-name": _verify_negative_log(
            row_dir / "tls-peer-to-rvoip-wrong-name.log",
            "peer-to-rvoip wrong-name rejection",
            (r"hostname mismatch", r"does not match"),
        ),
        "peer-to-rvoip-wrong-ca": _verify_negative_log(
            row_dir / "tls-peer-to-rvoip-wrong-ca.log",
            "peer-to-rvoip wrong-CA rejection",
            (
                r"self-signed certificate",
                r"unable to get local issuer",
                r"certificate verify failed",
                r"unknown ca",
            ),
        ),
        "peer-rejects-untrusted-client": _verify_negative_log(
            row_dir / "tls-peer-rejects-untrusted-client.log",
            "peer untrusted-client rejection",
            (
                r"unknown ca",
                r"certificate unknown",
                r"bad certificate",
                r"certificate verify failed",
                r"tlsv1 alert",
            ),
        ),
        "rvoip-rejects-untrusted-client": _verify_negative_log(
            row_dir / "tls-rvoip-rejects-untrusted-client.log",
            "rvoip untrusted-client rejection",
            (
                r"unknown ca",
                r"certificate unknown",
                r"bad certificate",
                r"certificate verify failed",
                r"tlsv1 alert",
            ),
        ),
    }

    rvoip_log_path = _required_file(row_dir / "rvoip.log", "rvoip TLS log")
    rvoip_log = rvoip_log_path.read_text(
        encoding="utf-8", errors="replace"
    )
    if not re.search(
        r"^RVOIP_PROXY_READY .*transport=tls .*tls_dns_authority=true$",
        rvoip_log,
        re.MULTILINE,
    ):
        raise VerificationError(
            "rvoip TLS log does not prove explicit DNS transport authority"
        )
    if "RVOIP_TLS_PEER_METADATA_MISSING" in rvoip_log:
        raise VerificationError(
            "rvoip accepted TLS SIP traffic without verified peer metadata"
        )
    accepted_peers = ACCEPTED_PEER_RE.findall(rvoip_log)
    expected_rvoip_client = "sipp" if order == "rvoip-first" else "peer"
    expected_rvoip_fingerprint = certificates[expected_rvoip_client][
        "der_sha256"
    ]
    if not accepted_peers:
        raise VerificationError(
            "rvoip log has no rustls-verified client identity on actual SIP traffic"
        )
    unexpected_fingerprints = {
        fingerprint
        for fingerprint, _chain_len in accepted_peers
        if fingerprint != expected_rvoip_fingerprint
    }
    if unexpected_fingerprints:
        raise VerificationError(
            "rvoip accepted an unexpected TLS client certificate on this row: "
            f"{sorted(unexpected_fingerprints)}"
        )

    first_proxy = "rvoip" if order == "rvoip-first" else "peer"
    last_proxy = "peer" if order == "rvoip-first" else "rvoip"
    boundary_paths = {
        "client": _required_file(
            row_dir / "tls-boundary-client.log",
            "actual SIP TLS client boundary log",
        ),
        "server": _required_file(
            row_dir / "tls-boundary-server.log",
            "actual SIP TLS server boundary log",
        ),
    }
    boundary_results = {
        "client": _verify_boundary_log(
            boundary_paths["client"],
            "client",
            expected_identities[first_proxy],
            expected_identities[first_proxy],
            certificates[first_proxy]["der_sha256"],
        ),
        "server": _verify_boundary_log(
            boundary_paths["server"],
            "server",
            expected_identities[last_proxy],
            expected_identities["sipp"],
            certificates[last_proxy]["der_sha256"],
        ),
    }

    peer_log_path = _required_file(row_dir / "peer.log", "peer TLS log")
    peer_log = peer_log_path.read_text(encoding="utf-8", errors="replace")
    peer_inbound_client = "rvoip" if order == "rvoip-first" else "sipp"
    expected_inbound_serial = certificates[peer_inbound_client]["serial"]
    observed_inbound_serials = {
        value.lower()
        for value in re.findall(
            r"INTEROP_TLS_VERIFIED direction=inbound "
            r"peer_serial=([0-9A-Fa-f]+)",
            peer_log,
        )
    }
    if observed_inbound_serials != {expected_inbound_serial}:
        raise VerificationError(
            "independent peer inbound mTLS serial binding failed"
        )

    if "INTEROP_TLS_IDENTITY_REJECT" in peer_log:
        raise VerificationError(
            f"{peer} inbound TLS identity validation rejected actual traffic"
        )
    next_hop = "sipp" if order == "rvoip-first" else "rvoip"
    outbound_boundary_path = _required_file(
        row_dir / f"tls-{peer}-outbound-boundary.log",
        f"{peer} outbound hostname-verifying boundary log",
    )
    peer_extra_paths = [outbound_boundary_path]
    peer_outbound_enforcement = {
        "kind": "gate-local-hostname-verifying-boundary",
        "boundary": _verify_boundary_log(
            outbound_boundary_path,
            "client",
            expected_identities[next_hop],
            expected_identities[next_hop],
            certificates[next_hop]["der_sha256"],
        ),
    }

    opensips_provenance_path: Path | None = None
    opensips_provenance: dict[str, Any] | None = None
    if peer == "opensips":
        opensips_provenance_path = _required_file(
            row_dir / "opensips-tls-image-provenance.json",
            "OpenSIPS TLS image provenance",
        )
        opensips_provenance = _verify_opensips_provenance(
            opensips_provenance_path
        )

    source_files = [
        *certificate_paths.values(),
        *certificate_hash_paths.values(),
        rvoip_log_path,
        peer_log_path,
        *boundary_paths.values(),
        *peer_extra_paths,
        *packet_source_files,
        *(
            row_dir / name
            for name in (
                "tls-rvoip-to-peer-positive.log",
                "tls-peer-to-rvoip-positive.log",
                "tls-rvoip-to-peer-wrong-name.log",
                "tls-rvoip-to-peer-wrong-ca.log",
                "tls-peer-to-rvoip-wrong-name.log",
                "tls-peer-to-rvoip-wrong-ca.log",
                "tls-peer-rejects-untrusted-client.log",
                "tls-rvoip-rejects-untrusted-client.log",
            )
        ),
        *(
            [opensips_provenance_path]
            if opensips_provenance_path is not None
            else []
        ),
    ]
    return {
        "schema": SCHEMA,
        "result": "PASS",
        "transport": "tls",
        "peer": peer,
        "order": order,
        "expected_identities": expected_identities,
        "certificates": certificates,
        "verified_legs": verified_legs,
        "negative_controls": negative_controls,
        "packet_aggregate": packet_aggregate,
        "opensips_image_provenance": opensips_provenance,
        "actual_sip": {
            "rvoip_expected_client": expected_rvoip_client,
            "rvoip_accepted_leaf_sha256": expected_rvoip_fingerprint,
            "rvoip_observed_messages": len(accepted_peers),
            "peer_inbound_client": peer_inbound_client,
            "peer_inbound_certificate_serial": expected_inbound_serial,
            "peer_outbound_enforcement": peer_outbound_enforcement,
            "hostname_verifying_boundaries": boundary_results,
        },
        "source_files_sha256": {
            str(path.relative_to(row_dir)): _sha256_file(path)
            for path in source_files
        },
    }


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--row-dir", required=True, type=Path)
    parser.add_argument(
        "--peer", required=True, choices=("kamailio", "opensips")
    )
    parser.add_argument(
        "--order", required=True, choices=("rvoip-first", "peer-first")
    )
    parser.add_argument("--openssl", default=os.environ.get("OPENSSL", "openssl"))
    parser.add_argument("--write-result", type=Path)
    return parser.parse_args()


def main() -> int:
    args = _parse_args()
    try:
        result = verify_row(args.row_dir, args.peer, args.order, args.openssl)
    except (OSError, VerificationError) as error:
        print(f"TLS evidence verification failed: {error}", file=sys.stderr)
        return 1

    rendered = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if args.write_result:
        output = args.write_result
        if output.name != RESULT_BASENAME:
            print(
                f"TLS verifier output must be named {RESULT_BASENAME}",
                file=sys.stderr,
            )
            return 2
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(rendered, encoding="utf-8")
    else:
        sys.stdout.write(rendered)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
