#!/usr/bin/env python3
"""Tests for the evidence-complete beta release report generator."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from itertools import product
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parent
CRATE_DIR = SCRIPT_DIR.parent
POLICY_PATH = CRATE_DIR / "config/beta-release-policy.yaml"
MODULE_SPEC = importlib.util.spec_from_file_location(
    "beta_release_report", SCRIPT_DIR / "beta_release_report.py"
)
assert MODULE_SPEC and MODULE_SPEC.loader
reporting = importlib.util.module_from_spec(MODULE_SPEC)
MODULE_SPEC.loader.exec_module(reporting)


def available_current_report() -> Path | None:
    configured = os.environ.get("RVOIP_BETA_REPORT_ROOT")
    candidates = [
        Path(configured) if configured else None,
        Path(
            "/Users/jonathan/Developer/rvoip-beta-evidence/"
            "20260724T231330Z/reports/20260724T231400Z"
        ),
    ]
    return next(
        (path for path in candidates if path and (path / "attestation.json").is_file()),
        None,
    )


def file_record(path: Path) -> dict[str, int | str]:
    return {
        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        "bytes": path.stat().st_size,
    }


def write_json(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def run_openssl(arguments: list[str]) -> bytes:
    openssl = shutil.which("openssl")
    if openssl is None:
        raise RuntimeError("OpenSSL is required for proxy interop reporter tests")
    completed = subprocess.run(
        [openssl, *arguments],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    return completed.stdout


def generate_tls_fixture_pki(root: Path) -> Path:
    pki = root / "tls-fixture-private"
    pki.mkdir()
    run_openssl(
        [
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-sha256",
            "-days",
            "2",
            "-subj",
            "/CN=rvoip proxy interop fixture CA/O=rvoip",
            "-addext",
            "basicConstraints=critical,CA:TRUE,pathlen:0",
            "-addext",
            "keyUsage=critical,keyCertSign,cRLSign",
            "-keyout",
            str(pki / "ca.key.pem"),
            "-out",
            str(pki / "ca.pem"),
        ]
    )
    identities = {
        "rvoip": "rvoip.proxy.test",
        "kamailio": "kamailio.proxy.test",
        "opensips": "opensips.proxy.test",
        "sipp": "sipp.proxy.test",
    }
    for serial, (name, identity) in enumerate(identities.items(), 1001):
        run_openssl(
            [
                "req",
                "-new",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-sha256",
                "-subj",
                f"/CN={identity}/O=rvoip proxy interop fixture",
                "-addext",
                f"subjectAltName=DNS:{identity}",
                "-keyout",
                str(pki / f"{name}.key.pem"),
                "-out",
                str(pki / f"{name}.csr.pem"),
            ]
        )
        extension = pki / f"{name}.ext"
        extension.write_text(
            "\n".join(
                [
                    "basicConstraints=critical,CA:FALSE",
                    "keyUsage=critical,digitalSignature,keyEncipherment",
                    "extendedKeyUsage=serverAuth,clientAuth",
                    f"subjectAltName=DNS:{identity}",
                    "",
                ]
            )
        )
        run_openssl(
            [
                "x509",
                "-req",
                "-sha256",
                "-days",
                "2",
                "-CA",
                str(pki / "ca.pem"),
                "-CAkey",
                str(pki / "ca.key.pem"),
                "-set_serial",
                str(serial),
                "-extfile",
                str(extension),
                "-in",
                str(pki / f"{name}.csr.pem"),
                "-out",
                str(pki / f"{name}.pem"),
            ]
        )
    return pki


def write_tls_row_evidence(row_dir: Path, pki: Path, peer: str, order: str) -> None:
    verifier = reporting._proxy_interop_tls_verifier()
    if peer == "opensips":
        dockerfile = (
            Path(reporting.__file__).resolve().parents[4] / verifier.OPENSIPS_DOCKERFILE
        )
        write_json(
            row_dir / "opensips-tls-image-provenance.json",
            {
                "schema": verifier.OPENSIPS_PROVENANCE_SCHEMA,
                "result": "PASS",
                "image": {
                    "reference": "rvoip/opensips-tls-interop:3.6.7-1",
                    "id": "sha256:" + "9" * 64,
                    "platform": "linux/amd64",
                    "base_digest": verifier.OPENSIPS_BASE_DIGEST,
                },
                "dockerfile": {
                    "relative_path": verifier.OPENSIPS_DOCKERFILE,
                    "sha256": hashlib.sha256(dockerfile.read_bytes()).hexdigest(),
                },
                "packages": verifier.OPENSIPS_PACKAGES,
                "modules": verifier.OPENSIPS_MODULES,
            },
        )
    public = row_dir / "tls-public"
    public.mkdir()
    for source_name, destination_name in (
        ("ca", "ca"),
        ("rvoip", "rvoip"),
        (peer, "peer"),
        ("sipp", "sipp"),
    ):
        certificate = public / f"{destination_name}.pem"
        shutil.copyfile(pki / f"{source_name}.pem", certificate)
        der = run_openssl(["x509", "-in", str(certificate), "-outform", "DER"])
        (public / f"{destination_name}.der.sha256").write_text(
            hashlib.sha256(der).hexdigest() + "\n"
        )

    identities = {
        "rvoip": "rvoip.proxy.test",
        "peer": f"{peer}.proxy.test",
    }
    positive_logs = {
        "tls-rvoip-to-peer-positive.log": identities["peer"],
        "tls-peer-to-rvoip-positive.log": identities["rvoip"],
    }
    for name, identity in positive_logs.items():
        (row_dir / name).write_text(
            "\n".join(
                [
                    "CONNECTION ESTABLISHED",
                    "Protocol version: TLSv1.2",
                    "Peer certificate:",
                    "Verification: OK",
                    f"Verified peername: {identity}",
                    "",
                ]
            )
        )
    for name in (
        "tls-rvoip-to-peer-wrong-name.log",
        "tls-peer-to-rvoip-wrong-name.log",
    ):
        (row_dir / name).write_text("hostname mismatch\n")
    for name in (
        "tls-rvoip-to-peer-wrong-ca.log",
        "tls-peer-to-rvoip-wrong-ca.log",
        "tls-peer-rejects-untrusted-client.log",
        "tls-rvoip-rejects-untrusted-client.log",
    ):
        (row_dir / name).write_text("certificate verify failed\n")

    expected_client = "sipp" if order == "rvoip-first" else peer
    accepted_der = run_openssl(
        ["x509", "-in", str(pki / f"{expected_client}.pem"), "-outform", "DER"]
    )
    accepted_hash = hashlib.sha256(accepted_der).hexdigest()
    (row_dir / "rvoip.log").write_text(
        "RVOIP_PROXY_READY address=127.0.0.1 transport=tls "
        "tls_dns_authority=true\n"
        "RVOIP_TLS_PEER_ACCEPTED direction=inbound transport=tls "
        f"leaf_certificate_sha256={accepted_hash} presented_chain_len=1\n"
    )
    peer_inbound_client = "rvoip" if order == "rvoip-first" else "sipp"
    serial_output = run_openssl(
        [
            "x509",
            "-in",
            str(pki / f"{peer_inbound_client}.pem"),
            "-noout",
            "-serial",
        ]
    ).decode()
    peer_serial = serial_output.strip().split("=", 1)[1].lower()
    peer_lines = [f"INTEROP_TLS_VERIFIED direction=inbound peer_serial={peer_serial}"]
    (row_dir / "peer.log").write_text("\n".join(peer_lines) + "\n")
    first_proxy = "rvoip" if order == "rvoip-first" else "peer"
    last_proxy = "peer" if order == "rvoip-first" else "rvoip"
    certificate_sources = {"rvoip": "rvoip", "peer": peer}
    certificate_hashes = {}
    for name, source_name in certificate_sources.items():
        der = run_openssl(
            ["x509", "-in", str(pki / f"{source_name}.pem"), "-outform", "DER"]
        )
        certificate_hashes[name] = hashlib.sha256(der).hexdigest()
    (row_dir / "tls-boundary-client.log").write_text(
        "TLS_BOUNDARY_ACCEPTED mode=client "
        f"expected_peer_dns={identities[first_proxy]} "
        f"presented_peer_dns={identities[first_proxy]} "
        f"server_name={identities[first_proxy]} "
        f"leaf_certificate_sha256={certificate_hashes[first_proxy]} "
        "protocol=TLSv1.2\n"
    )
    (row_dir / "tls-boundary-server.log").write_text(
        "TLS_BOUNDARY_ACCEPTED mode=server "
        f"expected_peer_dns={identities[last_proxy]} "
        f"presented_peer_dns={identities[last_proxy]} "
        "server_name=sipp.proxy.test "
        f"leaf_certificate_sha256={certificate_hashes[last_proxy]} "
        "protocol=TLSv1.2\n"
    )
    next_hop = "sipp" if order == "rvoip-first" else "rvoip"
    next_hop_identity = "sipp.proxy.test" if next_hop == "sipp" else identities["rvoip"]
    next_hop_der = run_openssl(
        [
            "x509",
            "-in",
            str(pki / f"{next_hop}.pem"),
            "-outform",
            "DER",
        ]
    )
    (row_dir / f"tls-{peer}-outbound-boundary.log").write_text(
        "TLS_BOUNDARY_READY mode=client "
        f"expected_peer_dns={next_hop_identity} "
        f"server_identity={next_hop_identity}\n"
        "TLS_BOUNDARY_ACCEPTED mode=client "
        f"expected_peer_dns={next_hop_identity} "
        f"presented_peer_dns={next_hop_identity} "
        f"server_name={next_hop_identity} "
        "leaf_certificate_sha256="
        f"{hashlib.sha256(next_hop_der).hexdigest()} "
        "protocol=TLSv1.2\n"
    )
    write_json(row_dir / "tls-verifier-result.json", {})


def write_external_scenario(
    scenario_dir: Path,
    scenario: str,
    peer: str,
    order: str,
    transport: str,
) -> None:
    packet_capture = scenario_dir.parents[1] / f"{scenario}--loopback.pcap"
    packet_capture.write_bytes(
        (
            "fixture scenario packet capture with records "
            f"{peer}/{order}/{transport}/{scenario}\n"
        ).encode()
    )
    packet_evidence = {
        "schema": "rvoip-sip-proxy-interop-packet-evidence-v1",
        "scenario": scenario,
        "status": "PASS",
        "peer": peer,
        "order": order,
        "transport": transport,
        "analyzer": {
            "tshark": "TShark fixture 4.4",
            "libpcap": "with libpcap fixture",
        },
        "captures": [
            {
                "filename": packet_capture.name,
                **file_record(packet_capture),
            }
        ],
        "display_filter": "tls" if transport == "tls" else "sip",
        "selected_packet_count": 1,
    }
    if transport == "tls":
        row_dir = scenario_dir.parents[1]
        observed_sni = sorted(
            {
                "rvoip.proxy.test",
                f"{peer}.proxy.test",
                "sipp.proxy.test",
            }
        )
        observed_handshakes = ["1", "2"]
        observed_certificates = sorted(
            (row_dir / f"tls-public/{name}.der.sha256").read_text().strip()
            for name in ("rvoip", "peer", "sipp")
        )
        required_methods, required_statuses = (
            reporting.PROXY_INTEROP_PACKET_REQUIREMENTS[scenario]
        )
        observed_methods = sorted(required_methods)
        observed_statuses = sorted(required_statuses)
        selected_call_ids = [f"{scenario}@fixture"]
        via_observation = {
            "addresses": ["127.0.0.1", "192.0.2.10"],
            "ports": ["25060", "25070"],
        }
        application_ports = ["25060", "25070", "25180"]
        assertions = [
            {
                "name": "scenario-call-id-observed",
                "passed": True,
                "observed": selected_call_ids,
            },
            {
                "name": "required-methods-observed",
                "passed": True,
                "observed": {
                    "required": observed_methods,
                    "observed": observed_methods,
                },
            },
            {
                "name": "required-statuses-observed",
                "passed": True,
                "observed": {
                    "required": observed_statuses,
                    "observed": observed_statuses,
                },
            },
            {
                "name": "rvoip-via-observed",
                "passed": True,
                "observed": via_observation,
            },
            {
                "name": "peer-via-observed",
                "passed": True,
                "observed": via_observation,
            },
            {
                "name": "tls-packets-observed",
                "passed": True,
                "observed": 3,
            },
            {
                "name": "tls-application-data-on-every-encrypted-hop",
                "passed": True,
                "observed": {
                    "required_listener_ports": application_ports,
                    "observed_ports": application_ports,
                    "application_record_count": 3,
                },
            },
            {
                "name": "tls-handshake-sni-valid-when-observed",
                "passed": True,
                "observed": {
                    "allowed": observed_sni,
                    "observed": observed_sni,
                    "client_hello_observed": True,
                },
            },
            {
                "name": "tls-handshake-certificates-observed-when-initiated",
                "passed": True,
                "observed": {
                    "client_hello_observed": True,
                    "certificate_sha256": observed_certificates,
                },
            },
        ]
        if scenario == "invite-success":
            assertions.extend(
                {
                    "name": name,
                    "passed": True,
                    "observed": "fixture dialog route contract",
                }
                for name in sorted(
                    reporting.PROXY_INTEROP_INVITE_DIALOG_PACKET_ASSERTIONS
                )
            )
        packet_evidence.update(
            {
                "display_filter": "sip or tls",
                "selected_packet_count": 4,
                "selected_sip_packet_count": 1,
                "selected_tls_packet_count": 3,
                "selected_call_ids": selected_call_ids,
                "observed_methods": observed_methods,
                "observed_statuses": observed_statuses,
                "via_sent_by_addresses": via_observation["addresses"],
                "via_sent_by_ports": via_observation["ports"],
                "observed_sni": observed_sni,
                "observed_handshake_types": observed_handshakes,
                "observed_certificate_sha256": observed_certificates,
                "observed_tls_application_listener_ports": application_ports,
                "observed_alerts": [],
                "assertions": assertions,
            }
        )
    else:
        required_methods, required_statuses = (
            reporting.PROXY_INTEROP_PACKET_REQUIREMENTS[scenario]
        )
        observed_methods = sorted(required_methods)
        observed_statuses = sorted(required_statuses)
        selected_call_ids = [
            (
                f"true-stray-{scenario}@fixture"
                if scenario == "stray-response-drop"
                else f"{scenario}@fixture"
            )
        ]
        via_observation = {
            "addresses": ["127.0.0.1"],
            "ports": ["25060"],
        }
        assertions = [
            {
                "name": "scenario-call-id-observed",
                "passed": True,
                "observed": selected_call_ids,
            },
            {
                "name": "required-methods-observed",
                "passed": True,
                "observed": {
                    "required": observed_methods,
                    "observed": observed_methods,
                },
            },
            {
                "name": "required-statuses-observed",
                "passed": True,
                "observed": {
                    "required": observed_statuses,
                    "observed": observed_statuses,
                },
            },
            {
                "name": "rvoip-via-observed",
                "passed": True,
                "observed": via_observation,
            },
            {
                "name": "peer-via-observed",
                "passed": True,
                "observed": via_observation,
            },
        ]
        if scenario == "stray-response-drop":
            assertions.extend(
                [
                    {
                        "name": "one-true-stray-call-observed",
                        "passed": True,
                        "observed": selected_call_ids,
                    },
                    {
                        "name": "true-stray-arrived-at-rvoip",
                        "passed": True,
                        "observed": 1,
                    },
                    {
                        "name": "true-stray-had-zero-rvoip-egress",
                        "passed": True,
                        "observed": 0,
                    },
                ]
            )
        packet_evidence.update(
            {
                "selected_call_ids": selected_call_ids,
                "observed_methods": observed_methods,
                "observed_statuses": observed_statuses,
                "via_sent_by_addresses": via_observation["addresses"],
                "via_sent_by_ports": via_observation["ports"],
                "assertions": assertions,
            }
        )
    packet_result = scenario_dir / "packet-evidence.json"
    write_json(packet_result, packet_evidence)
    inputs = {packet_result.name: file_record(packet_result)}
    evidence_kind = "external-sipp-and-packet-observation"
    if scenario in (
        reporting.PROXY_INTEROP_UDP_ADVANCED_SCENARIOS
        | reporting.PROXY_INTEROP_TCP_ADVANCED_SCENARIOS
    ):
        evidence_kind = reporting.PROXY_INTEROP_RAW_EVIDENCE_KIND
        raw_wire = scenario_dir / "raw-wire.json"
        external_key = (
            "external_peer_path_observed"
            if transport == "udp"
            else "external_peer_exercised"
        )
        write_json(
            raw_wire,
            {
                "schema": reporting.PROXY_INTEROP_RAW_SCHEMAS[transport],
                "scenario": scenario,
                "status": "PASS",
                "transport": transport,
                external_key: True,
            },
        )
        inputs[raw_wire.name] = file_record(raw_wire)
    write_json(
        scenario_dir / "result.json",
        {
            "schema": reporting.PROXY_INTEROP_SCENARIO_SCHEMA,
            "scenario": scenario,
            "status": "PASS",
            "evidence_kind": evidence_kind,
            "external_peer_exercised": True,
            "peer": peer,
            "order": order,
            "transport": transport,
            "assertions": [
                {
                    "name": "external-peer-observed",
                    "passed": True,
                    "observed": peer,
                }
            ],
            "inputs": inputs,
        },
    )


def write_retention_scenario(
    scenario_dir: Path, peer: str, order: str, transport: str
) -> None:
    retention_log = scenario_dir / "rvoip.log"
    phases = {}
    for phase in reporting.PROXY_INTEROP_RETENTION_PHASES:
        counters = {"transactions": 1} if phase == "activity" else {"transactions": 0}
        phases[phase] = {
            "counters": counters,
            "nonzero": {name: value for name, value in counters.items() if value != 0},
        }
    retention_log.write_text(
        "".join(
            "RVOIP_PROXY_RETENTION "
            f"phase={phase} "
            + " ".join(f"{name}={value}" for name, value in record["counters"].items())
            + "\n"
            for phase, record in phases.items()
        )
    )
    write_json(
        scenario_dir / "result.json",
        {
            "schema": reporting.PROXY_INTEROP_SCENARIO_SCHEMA,
            "scenario": "retention-cleanup",
            "status": "PASS",
            "evidence_kind": "retention-phase-snapshots",
            "external_peer_exercised": True,
            "peer": peer,
            "order": order,
            "transport": transport,
            "assertions": [
                {
                    "name": "retention-converged",
                    "passed": True,
                    "observed": "all terminal phases zero",
                }
            ],
            "inputs": {"rvoip.log": file_record(retention_log)},
            "phases": phases,
        },
    )


def write_tls_scenario(
    scenario_dir: Path, row_dir: Path, peer: str, order: str
) -> None:
    required = [
        "tls-verifier-result.json",
        "rvoip.log",
        "peer.log",
        "tls-rvoip-to-peer-positive.log",
        "tls-peer-to-rvoip-positive.log",
        "tls-rvoip-to-peer-wrong-name.log",
        "tls-rvoip-to-peer-wrong-ca.log",
        "tls-peer-to-rvoip-wrong-name.log",
        "tls-peer-to-rvoip-wrong-ca.log",
        "tls-peer-rejects-untrusted-client.log",
        "tls-rvoip-rejects-untrusted-client.log",
        "tls-boundary-client.log",
        "tls-boundary-server.log",
    ]
    required.append(f"tls-{peer}-outbound-boundary.log")
    inputs = {}
    for name in required:
        destination = scenario_dir / name
        shutil.copyfile(row_dir / name, destination)
        inputs[name] = file_record(destination)
    runtime_copy = scenario_dir / "peer-runtime.json"
    shutil.copyfile(row_dir / runtime_copy.name, runtime_copy)
    inputs[runtime_copy.name] = file_record(runtime_copy)
    if peer == "opensips":
        provenance_name = "opensips-tls-image-provenance.json"
        provenance_copy = scenario_dir / provenance_name
        shutil.copyfile(row_dir / provenance_name, provenance_copy)
        inputs[provenance_name] = file_record(provenance_copy)
    verifier_source = (
        Path(reporting.__file__).resolve().parents[2]
        / "sip-proxy/tests/interop/scripts/verify_tls_evidence.py"
    )
    verifier_copy = scenario_dir / verifier_source.name
    shutil.copyfile(verifier_source, verifier_copy)
    inputs[verifier_copy.name] = file_record(verifier_copy)
    packet_capture = row_dir / "sips-routing--loopback.pcap"
    packet_capture.write_bytes(f"fixture SIPS capture {peer}/{order}/tls\n".encode())
    vias = {
        "addresses": ["127.0.0.1", "192.0.2.10"],
        "ports": ["25060", "25070"],
    }
    observed_sni = sorted(
        {
            "rvoip.proxy.test",
            f"{peer}.proxy.test",
            "sipp.proxy.test",
        }
    )
    observed_certificates = sorted(
        (row_dir / f"tls-public/{name}.der.sha256").read_text().strip()
        for name in ("rvoip", "peer", "sipp")
    )
    application_ports = ["25060", "25070", "25180"]
    packet_assertions = [
        {
            "name": "tls-packets-observed",
            "passed": True,
            "observed": 4,
        },
        {
            "name": "tls-application-data-on-every-encrypted-hop",
            "passed": True,
            "observed": {
                "required_listener_ports": application_ports,
                "observed_ports": application_ports,
                "application_record_count": 3,
            },
        },
        {
            "name": "tls-handshake-sni-valid-when-observed",
            "passed": True,
            "observed": {
                "allowed": observed_sni,
                "observed": observed_sni,
                "client_hello_observed": True,
            },
        },
        {
            "name": "tls-handshake-certificates-observed-when-initiated",
            "passed": True,
            "observed": {
                "client_hello_observed": True,
                "certificate_sha256": observed_certificates,
            },
        },
        {
            "name": "scenario-call-id-observed",
            "passed": True,
            "observed": ["sips-routing@fixture"],
        },
        {
            "name": "required-methods-observed",
            "passed": True,
            "observed": {
                "required": ["OPTIONS"],
                "observed": ["OPTIONS"],
            },
        },
        {
            "name": "required-statuses-observed",
            "passed": True,
            "observed": {"required": [200], "observed": [200]},
        },
        {
            "name": "rvoip-via-observed",
            "passed": True,
            "observed": vias,
        },
        {
            "name": "peer-via-observed",
            "passed": True,
            "observed": vias,
        },
        {
            "name": "sips-request-uri-at-uac-boundary",
            "passed": True,
            "observed": {
                "required_uri": "sips:probe@example.test",
                "packets": 1,
            },
        },
        {
            "name": "sips-request-uri-at-uas-boundary",
            "passed": True,
            "observed": {
                "required_uri": "sips:probe@example.test",
                "packets": 1,
            },
        },
        {
            "name": "sips-request-preserved-end-to-end",
            "passed": True,
            "observed": ["sips:probe@example.test"],
        },
        {
            "name": "sips-both-proxy-vias-observed",
            "passed": True,
            "observed": 1,
        },
        {
            "name": "no-plaintext-sip-on-external-tls-ports",
            "passed": True,
            "observed": 0,
        },
        {
            "name": "sips-options-success-observed",
            "passed": True,
            "observed": {"methods": ["OPTIONS"], "statuses": [200]},
        },
    ]
    packet_result = scenario_dir / "packet-evidence.json"
    write_json(
        packet_result,
        {
            "schema": "rvoip-sip-proxy-interop-packet-evidence-v1",
            "scenario": "sips-routing",
            "status": "PASS",
            "peer": peer,
            "order": order,
            "transport": "tls",
            "analyzer": {
                "tshark": "TShark fixture 4.4",
                "libpcap": "with libpcap fixture",
            },
            "captures": [
                {
                    "filename": packet_capture.name,
                    **file_record(packet_capture),
                }
            ],
            "display_filter": "sip or tls",
            "selected_call_ids": ["sips-routing@fixture"],
            "selected_packet_count": 8,
            "selected_tls_packet_count": 4,
            "selected_sip_packet_count": 4,
            "observed_methods": ["OPTIONS"],
            "observed_statuses": [200],
            "via_sent_by_addresses": vias["addresses"],
            "via_sent_by_ports": vias["ports"],
            "observed_sips_request_uris": ["sips:probe@example.test"],
            "plaintext_sip_endpoints": [
                {
                    "source": "127.0.0.1:25090",
                    "destination": "127.0.0.1:25190",
                },
                {
                    "source": "127.0.0.1:45123",
                    "destination": "127.0.0.1:25080",
                },
            ],
            "insecure_external_sip_packet_count": 0,
            "observed_sni": observed_sni,
            "observed_handshake_types": ["1", "2"],
            "observed_certificate_sha256": observed_certificates,
            "observed_tls_application_listener_ports": application_ports,
            "observed_alerts": [],
            "assertions": packet_assertions,
        },
    )
    inputs[packet_result.name] = file_record(packet_result)
    for name, content in {
        "uac-messages.log": "fixture actual SIPS UAC wire trace\n",
        "uas-messages.log": "fixture actual SIPS UAS wire trace\n",
        "uac-stats.csv": "SuccessfulCall(C);1\n",
        "uas-stats.csv": "SuccessfulCall(C);1\n",
    }.items():
        path = scenario_dir / name
        path.write_text(content)
        inputs[name] = file_record(path)
    write_json(
        scenario_dir / "result.json",
        {
            "schema": reporting.PROXY_INTEROP_SCENARIO_SCHEMA,
            "scenario": "sips-routing",
            "status": "PASS",
            "evidence_kind": "verified-external-tls",
            "external_peer_exercised": True,
            "peer": peer,
            "order": order,
            "transport": "tls",
            "assertions": [
                {
                    "name": name,
                    "passed": True,
                    "observed": "fixture bound live SIPS evidence",
                }
                for name in sorted(reporting.PROXY_INTEROP_SIPS_RESULT_ASSERTIONS)
            ],
            "inputs": inputs,
        },
    )


def write_supplemental_scenario(
    row_dir: Path,
    peer: str,
    order: str,
    transport: str,
    sha: str,
    fingerprint: str,
) -> None:
    scenario = "sequential-fork"
    supplemental = row_dir / "supplemental" / scenario
    supplemental.mkdir(parents=True)
    log = supplemental / "exact-test.log"
    log.write_text(
        "running 1 test\n"
        "test fixture_exact_test ... ok\n"
        "test result: ok. 1 passed; 0 failed; 0 ignored\n"
    )
    write_json(
        supplemental / "result.json",
        {
            "schema": reporting.PROXY_INTEROP_SCENARIO_SCHEMA,
            "scenario": scenario,
            "status": "PASS",
            "evidence_kind": reporting.PROXY_INTEROP_SUPPLEMENTAL_EVIDENCE_KIND,
            "external_peer_exercised": False,
            "limitation": "Supplemental deterministic race evidence only.",
            "peer_row": {
                "peer": peer,
                "order": order,
                "transport": transport,
            },
            "source": {
                "sha": sha,
                "fingerprint_sha256": fingerprint,
            },
            "commands": [
                {
                    "argv": [
                        "cargo",
                        "test",
                        "--package",
                        "rvoip-sip-proxy",
                        "--test",
                        "proxy_fixture",
                        "fixture_exact_test",
                        "--",
                        "--exact",
                        "--test-threads=1",
                    ],
                    "exit_code": 0,
                    "one_exact_test_passed": True,
                    "log": log.name,
                    "log_sha256": hashlib.sha256(log.read_bytes()).hexdigest(),
                }
            ],
        },
    )


def write_runtime_state_fixture(proxy: Path) -> None:
    snapshot = {
        "schema": reporting.PROXY_INTEROP_RUNTIME_STATE_SCHEMA,
        "kind": "snapshot",
        "captured_at_utc": "2026-07-26T00:00:00Z",
        "ports": [25060, 25070],
        "collectors": {"docker": "docker-list-json", "listeners": "lsof"},
        "docker": {
            "containers": [
                {
                    "id": "container-1",
                    "name": "unrelated",
                    "image": "example:test",
                    "state": "running",
                }
            ],
            "networks": [
                {
                    "id": "network-1",
                    "name": "bridge",
                    "driver": "bridge",
                    "scope": "local",
                }
            ],
            "volumes": [{"name": "volume-1", "driver": "local", "scope": "local"}],
        },
        "listeners": [
            {
                "transport": "tcp",
                "port": 25060,
                "pid": "101",
                "process": "unrelated",
                "endpoint": "*:25060",
            }
        ],
    }
    write_json(proxy / "runtime-state-start.json", snapshot)
    end = dict(snapshot)
    end["captured_at_utc"] = "2026-07-26T01:00:00Z"
    write_json(proxy / "runtime-state-end.json", end)
    empty = {"added": [], "removed": [], "changed": []}
    write_json(
        proxy / "runtime-state-check.json",
        {
            "schema": reporting.PROXY_INTEROP_RUNTIME_STATE_SCHEMA,
            "kind": "comparison",
            "compared_at_utc": "2026-07-26T01:00:01Z",
            "ports": snapshot["ports"],
            "clean": True,
            "preexisting_state_preserved": True,
            "no_added_leftovers": True,
            "added_leftovers": {},
            "removed_preexisting": {},
            "changed_preexisting": {},
            "differences": {
                name: dict(empty)
                for name in ("containers", "networks", "volumes", "listeners")
            },
        },
    )


def write_peer_runtime_fixture(row_dir: Path, peer: str, transport: str) -> None:
    is_derived_opensips = peer == "opensips" and transport == "tls"
    write_json(
        row_dir / "peer-runtime.json",
        {
            "schema": reporting.PROXY_INTEROP_PEER_RUNTIME_SCHEMA,
            "product": peer,
            "container": {
                "name": f"rvoip-{peer}-fixture",
                "id": f"{peer}-{transport}-container",
                "created": "2026-07-26T00:00:00Z",
                "platform": "linux",
            },
            "image": {
                "id": "sha256:" + ("9" if is_derived_opensips else "8") * 64,
                "reference": (
                    reporting.PROXY_INTEROP_OPENSIPS_TLS_REFERENCE
                    if is_derived_opensips
                    else reporting.PROXY_INTEROP_IMAGES[peer]
                ),
            },
            "configuration": {
                "network_mode": "bridge",
                "published_ports": {},
                "exposed_ports": [],
                "restart_policy": {"name": "no", "maximum_retry_count": 0},
            },
            "state": {
                "status": "running",
                "running": True,
                "paused": False,
                "restarting": False,
                "oom_killed": False,
                "dead": False,
                "exit_code": 0,
                "started_at": "2026-07-26T00:00:00Z",
                "finished_at": "",
            },
            "network": {
                "published_ports": {},
                "networks": {
                    "fixture": {
                        "network_id": "network-1",
                        "ip_address": "172.18.0.2",
                    }
                },
            },
        },
    )


def write_proxy_interop_fixture(root: Path) -> Path:
    proxy = root / "proxy-interop"
    proxy.mkdir(parents=True)
    sha = "a" * 40
    fingerprint = "b" * 64
    pki = generate_tls_fixture_pki(root)
    rows = []
    matrix = ["peer\torder\ttransport\tstatus\tduration_seconds\tartifact\tscenarios"]
    for peer, order, transport in product(
        sorted(reporting.PROXY_INTEROP_PEERS),
        sorted(reporting.PROXY_INTEROP_ORDERS),
        sorted(reporting.PROXY_INTEROP_TRANSPORTS),
    ):
        relative = f"{peer}/{order}/{transport}"
        row_dir = proxy / relative
        row_dir.mkdir(parents=True)
        peer_version = {
            "kamailio": "version: kamailio 6.1.3 (x86_64/linux)",
            "opensips": "version: opensips 3.6.7 (x86_64/linux)",
        }[peer]
        (row_dir / "peer-version.txt").write_text(peer_version + "\n")
        (row_dir / "retention-check.txt").write_text("counter_count=39\nnonzero={}\n")
        write_peer_runtime_fixture(row_dir, peer, transport)
        if transport == "tls":
            write_tls_row_evidence(row_dir, pki, peer, order)
        scenarios = set(
            reporting._expected_proxy_interop_scenarios((peer, order, transport))
        )
        for scenario in sorted(scenarios):
            scenario_dir = row_dir / "scenarios" / scenario
            scenario_dir.mkdir(parents=True)
            if scenario == "retention-cleanup":
                write_retention_scenario(scenario_dir, peer, order, transport)
            elif scenario == "sips-routing":
                write_tls_scenario(scenario_dir, row_dir, peer, order)
            else:
                write_external_scenario(scenario_dir, scenario, peer, order, transport)
        if transport == "tls":
            verifier = reporting._proxy_interop_tls_verifier()
            verifier_result = row_dir / "tls-verifier-result.json"
            write_json(
                verifier_result,
                verifier.verify_row(row_dir, peer, order),
            )
            sips_scenario = row_dir / "scenarios/sips-routing"
            verifier_copy = sips_scenario / verifier_result.name
            shutil.copyfile(verifier_result, verifier_copy)
            sips_result_path = sips_scenario / "result.json"
            sips_result = json.loads(sips_result_path.read_text())
            sips_result["inputs"][verifier_copy.name] = file_record(verifier_copy)
            write_json(sips_result_path, sips_result)
        if (peer, order, transport) == ("kamailio", "rvoip-first", "udp"):
            write_supplemental_scenario(
                row_dir, peer, order, transport, sha, fingerprint
            )
        row = {
            "peer": peer,
            "order": order,
            "transport": transport,
            "status": "PASS",
            "duration_seconds": "131",
            "artifact": relative,
            "scenarios": sorted(scenarios),
        }
        rows.append(row)
        matrix.append(
            f"{peer}\t{order}\t{transport}\tPASS\t131\t{relative}\t"
            f"{','.join(sorted(scenarios))}"
        )
    (proxy / "matrix.tsv").write_text("\n".join(matrix) + "\n")
    (proxy / "summary.md").write_text("# fixture proxy interop\n")
    binary_sha = "c" * 64
    (proxy / "cargo-build.log").write_text("Finished release fixture\n")
    (proxy / "cargo-build-command.txt").write_text(
        "/fixture/cargo build --manifest-path /fixture/Cargo.toml "
        "--release --package rvoip-sip-proxy "
        "--example stateful_proxy_interop\n"
    )
    (proxy / "proxy-binary.sha256").write_text(binary_sha + "\n")
    (proxy / "proxy-binary.path").write_text(reporting.PROXY_INTEROP_BINARY_PATH + "\n")
    (proxy / "proxy-binary-check.txt").write_text(
        f"start_sha256={binary_sha}\nend_sha256={binary_sha}\nunchanged=true\n"
    )
    (proxy / "environment.txt").write_text(
        "\n".join(
            [
                "started_at_utc: 2026-07-26T00:00:00Z",
                f"source_sha: {sha}",
                f"source_fingerprint: {fingerprint}",
                "source_dirty: false",
                "host_os: Linux",
                "docker_context: fixture",
                "network_topology: fixture",
                "host_address: 127.0.0.1",
                "peer_address: 127.0.0.1",
                "capture_interfaces: lo",
                "peer_port: 25070",
                "rvoip_port: 25060",
                "uas_port: 25080",
                "uac_port: 25090",
                "tls_peer_boundary_port: 25170",
                "tls_uas_boundary_port: 25180",
                "tls_uac_boundary_port: 25190",
                "retention_drain_seconds: 130",
                "peers: kamailio opensips",
                "orders: rvoip-first peer-first",
                "transports: udp tcp tls",
                f"kamailio_image: {reporting.PROXY_INTEROP_IMAGES['kamailio']}",
                "kamailio_platform: linux/amd64",
                f"opensips_image: {reporting.PROXY_INTEROP_IMAGES['opensips']}",
                "opensips_platform: linux/amd64",
                "cargo_path: /fixture/cargo",
                "cargo: cargo 1.95.0",
                "rustc_path: /fixture/rustc",
                "rustc: rustc 1.95.0",
                "sipp_path: /fixture/sipp",
                "sipp: SIPp v3.7.7",
                "tcpdump_path: /fixture/tcpdump",
                "tcpdump: tcpdump version 4.99.5",
                "tshark_path: /fixture/tshark",
                "tshark: TShark 4.4.0",
                "docker_path: /fixture/docker",
                "docker: 28.0.0 linux/amd64",
                "",
            ]
        )
    )
    (proxy / "source-check.txt").write_text("unchanged=true\n")
    write_runtime_state_fixture(proxy)
    write_json(
        proxy / "summary.json",
        {
            "schema": "rvoip-sip-proxy-interop-v1",
            "result": {
                "status": "PASS",
                "passed_rows": 12,
                "failed_rows": 0,
                "gate_failures": 0,
            },
            "source": {
                "start_sha": sha,
                "end_sha": sha,
                "start_fingerprint_sha256": fingerprint,
                "end_fingerprint_sha256": fingerprint,
                "dirty_at_start": False,
                "unchanged": True,
            },
            "configuration": {
                "kamailio_image": reporting.PROXY_INTEROP_IMAGES["kamailio"],
                "opensips_image": reporting.PROXY_INTEROP_IMAGES["opensips"],
                "kamailio_platform": "linux/amd64",
                "opensips_platform": "linux/amd64",
                "retention_drain_seconds": 130,
            },
            "rows": rows,
        },
    )
    return proxy


class PolicyTests(unittest.TestCase):
    def test_top_level_readme_declares_bounded_four_peer_attestation(self) -> None:
        readme = (SCRIPT_DIR.parents[3] / "README.md").read_text()
        self.assertIn("### SIP interoperability attestation", readme)
        for display_name in ("Asterisk", "FreeSWITCH", "Kamailio", "OpenSIPS"):
            self.assertRegex(
                readme,
                rf"\| \*\*{display_name}\*\* \|",
            )
        self.assertIn("This is bounded interoperability", readme)
        self.assertIn("not a claim of compatibility", readme)

    def test_legacy_report_schema_cannot_bypass_four_peer_attestation(self) -> None:
        with self.assertRaisesRegex(
            reporting.ReportError, "four-peer interoperability attestation v2"
        ):
            reporting.validate_report_schema_scope(
                reporting.SCHEMA_REPORT_ATTESTATION_LEGACY,
                {"records": [{"id": "interop.proxy-stateful-matrix"}]},
            )
        reporting.validate_report_schema_scope(
            reporting.SCHEMA_REPORT_ATTESTATION_LEGACY,
            {"records": [{"id": "interop.asterisk-matrix"}]},
        )

    def test_catalog_is_unique_reachable_and_current_full_selects_108(self) -> None:
        policy = reporting.load_policy(POLICY_PATH)
        gates = reporting.expand_catalog(policy)
        self.assertEqual(len({gate["id"] for gate in gates}), len(gates))
        self.assertEqual(len({gate["name"] for gate in gates}), len(gates))
        config = {
            key: definition.get("full_default", definition.get("default"))
            for key, definition in policy["configuration"].items()
        }
        config.update(
            {
                "beta_gate_mode": "full",
                "beta_require_canonical_2k_evidence": True,
                "beta_run_local_pbx": True,
                "beta_restore_local_pbx": True,
                "beta_run_sipp": True,
                "beta_run_strict_ua": True,
                "beta_run_perf_all": True,
                "beta_run_burst_smoke": True,
                "beta_run_burst_matrix": True,
                "beta_run_long_soak": True,
            }
        )
        selected = [
            gate for gate in gates if reporting.gate_selected(gate, "full", config)
        ]
        self.assertEqual(len(selected), 108)
        proxy_gate = next(
            gate for gate in gates if gate["id"] == "interop.proxy-stateful-matrix"
        )
        self.assertEqual(
            proxy_gate["name"],
            reporting.PROXY_INTEROP_GATE_NAME,
        )

    def test_every_condition_and_validator_is_defined(self) -> None:
        policy = reporting.load_policy(POLICY_PATH)
        definitions = policy["configuration"]
        for gate in reporting.expand_catalog(policy):
            self.assertTrue(set(gate["validators"]) <= reporting.KNOWN_VALIDATORS)
            for operator in ("when", "unless"):
                key = gate["condition"].get(operator)
                if key:
                    self.assertIn(key, definitions)

    def test_typed_configuration_and_safe_paths_fail_closed(self) -> None:
        self.assertIs(reporting.convert_typed("0", {"type": "boolean"}), False)
        self.assertEqual(
            reporting.convert_typed("30 100,300", {"type": "string-list"}),
            ["30", "100", "300"],
        )
        with self.assertRaises(reporting.ReportError):
            reporting.convert_typed("sometimes", {"type": "boolean"})
        self.assertTrue(reporting.safe_relative("perf-results/result.json"))
        self.assertFalse(reporting.safe_relative("../outside.json"))
        self.assertFalse(reporting.safe_relative("/absolute/path"))

    def test_latency_policy_and_extraction_are_complete_and_fail_closed(self) -> None:
        policy = reporting.load_policy(POLICY_PATH)
        self.assertEqual(
            reporting.canonical_latency_limits(policy)["setup_latency"],
            {"p50": 13.97, "p95": 15.36, "p99": 16.69},
        )
        report = {
            "latency_ns": {
                "setup_latency": {"p50": 1, "p95": 2, "p99": 3},
                "full_cycle": {"p50": 4, "p95": 5, "p99": 6},
            }
        }
        self.assertEqual(
            reporting.latency_percentiles(report, "fixture")["full_cycle"]["p99"],
            6,
        )
        del report["latency_ns"]["setup_latency"]["p99"]
        with self.assertRaises(reporting.ReportError):
            reporting.latency_percentiles(report, "fixture")

    def test_native_gate_fragments_are_typed_and_deduplicated(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            args = type(
                "Args",
                (),
                {
                    "policy": POLICY_PATH,
                    "results_dir": root / "parts",
                    "sequence": 1,
                    "name": "format check",
                    "status": "PASS",
                    "started": "2026-07-25T00:00:00Z",
                    "ended": "2026-07-25T00:00:01Z",
                    "duration": 1,
                    "exit_status": 0,
                    "log": "format_check.log",
                    "log_sha256": "a" * 64,
                    "argv": ["cargo", "fmt", "--check"],
                },
            )()
            reporting.record_gate(args)
            output = root / "gate-results.json"
            reporting.finalize_gates(args.results_dir, output, "full")
            payload = json.loads(output.read_text())
            self.assertEqual(payload["schema"], reporting.SCHEMA_RESULTS)
            self.assertEqual(payload["passed"], 1)
            self.assertEqual(payload["records"][0]["id"], "build.format")
            self.assertEqual(
                payload["records"][0]["sanitized_argv"],
                ["cargo", "fmt", "--check"],
            )

    def test_proxy_pbx_matrix_rows_are_verified_like_the_b2bua_providers(self) -> None:
        """The registrar-proxy labs append to the same pbx/matrix.tsv as the
        B2BUA providers, so their gates need the same row-level check. Without
        it a proxy gate that exited 0 having recorded nothing would pass."""
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "pbx").mkdir()
            header = "status\tprovider\tapi\tscenario\ttransport\trole"
            rows = [
                "PASS\tkamailio\tendpoint\tamr_call\tUDP\tcaller",
                "PASS\tkamailio\tendpoint\tamr_call\tUDP\tcallee",
                "PASS\topensips\tendpoint\tamr_call\tUDP\tcaller",
                "FAIL\topensips\tendpoint\tamr_call\tUDP\tcallee",
            ]
            (root / "pbx/matrix.tsv").write_text("\n".join([header, *rows]) + "\n")

            kamailio = reporting.interop_observed_check(
                root, "local Kamailio PBX matrix"
            )
            self.assertIsNotNone(kamailio)
            self.assertEqual(kamailio["check"], "kamailio PBX matrix rows")
            self.assertEqual(kamailio["observed"], {"rows": 2, "pass": 2, "skip": 0})
            self.assertTrue(kamailio["passed"])

            # A FAIL row must fail the gate, not be averaged away.
            opensips = reporting.interop_observed_check(
                root, "local OpenSIPS PBX matrix"
            )
            self.assertIsNotNone(opensips)
            self.assertFalse(opensips["passed"])

    def test_proxy_pbx_matrix_with_no_rows_does_not_pass(self) -> None:
        """A lab that recorded nothing is not a pass. This is the mutation the
        whole check exists for: PASS count zero must fail even with no FAILs."""
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "pbx").mkdir()
            (root / "pbx/matrix.tsv").write_text(
                "status\tprovider\tapi\tscenario\ttransport\trole\n"
                "PASS\tasterisk\tendpoint\tbasic_call\tUDP\tcaller\n"
            )
            observed = reporting.interop_observed_check(
                root, "local Kamailio PBX matrix"
            )
            self.assertEqual(observed["observed"], {"rows": 0, "pass": 0, "skip": 0})
            self.assertFalse(observed["passed"])

    def test_proxy_interop_report_validation_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            proxy = write_proxy_interop_fixture(root)
            observed = reporting.validate_proxy_interop_result(root)
            self.assertEqual(observed["rows"], 12)
            self.assertEqual(
                set(observed["transports"]),
                {"udp", "tcp", "tls"},
            )
            self.assertEqual(len(observed["tls_packet_aggregates"]), 4)
            for artifact, aggregate in observed["tls_packet_aggregates"].items():
                peer = artifact.split("/", 1)[0]
                self.assertEqual(
                    aggregate["observed_sni"],
                    sorted(
                        {
                            "rvoip.proxy.test",
                            f"{peer}.proxy.test",
                            "sipp.proxy.test",
                        }
                    ),
                )
                self.assertEqual(
                    len(aggregate["expected_leaf_certificate_sha256"]),
                    3,
                )
                self.assertGreaterEqual(aggregate["capture_count"], 10)
            tls_packet = json.loads(
                (
                    proxy / "kamailio/rvoip-first/tls/scenarios/"
                    "options-readiness/packet-evidence.json"
                ).read_text()
            )
            tls_assertions = {
                assertion["name"] for assertion in tls_packet["assertions"]
            }
            self.assertTrue(
                reporting.PROXY_INTEROP_SIP_PACKET_ASSERTIONS
                | reporting.PROXY_INTEROP_TLS_PACKET_ASSERTIONS
                <= tls_assertions
            )
            self.assertTrue(
                {
                    "tls-client-hello-observed",
                    "expected-sni-observed",
                    "tls-certificates-observed",
                }.isdisjoint(tls_assertions)
            )
            evidence_paths = reporting.proxy_interop_evidence_paths(root)
            self.assertIn(
                "proxy-interop/opensips/rvoip-first/tls/tls-verifier-result.json",
                evidence_paths,
            )
            completed = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT_DIR / "beta_release_report.py"),
                    "validate-proxy-interop",
                    "--report-root",
                    str(root),
                ],
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertIn(
                "stateful proxy interoperability validation: PASS",
                completed.stdout,
            )

            missing_tls = proxy / "opensips/rvoip-first/tls/tls-verifier-result.json"
            tls_payload = json.loads(missing_tls.read_text())
            invalid_aggregate = json.loads(missing_tls.read_text())
            invalid_aggregate["packet_aggregate"]["expected_sni"].pop()
            write_json(missing_tls, invalid_aggregate)
            with self.assertRaisesRegex(
                reporting.ReportError, "TLS packet aggregate is invalid"
            ):
                reporting.validate_proxy_interop_result(root)
            write_json(missing_tls, tls_payload)

            invalid_aggregate = json.loads(missing_tls.read_text())
            invalid_aggregate["packet_aggregate"][
                "expected_leaf_certificate_sha256"
            ].pop()
            write_json(missing_tls, invalid_aggregate)
            with self.assertRaisesRegex(
                reporting.ReportError, "TLS packet aggregate is invalid"
            ):
                reporting.validate_proxy_interop_result(root)
            write_json(missing_tls, tls_payload)

            missing_tls.unlink()
            with self.assertRaisesRegex(
                reporting.ReportError, "TLS verifier result is missing"
            ):
                reporting.validate_proxy_interop_result(root)
            tls_payload["actual_sip"]["rvoip_observed_messages"] += 1
            write_json(missing_tls, tls_payload)
            with self.assertRaisesRegex(
                reporting.ReportError, "TLS verifier result drifted"
            ):
                reporting.validate_proxy_interop_result(root)

    def test_four_peer_interop_attestation_is_explicit_and_fail_closed(self) -> None:
        sha = "a" * 64
        proxy_observed = {
            "rows": 12,
            "peers": ["kamailio", "opensips"],
            "orders": sorted(reporting.PROXY_INTEROP_ORDERS),
            "transports": sorted(reporting.PROXY_INTEROP_TRANSPORTS),
            "scenarios_by_peer": {
                "kamailio": ["invite-success", "retention-cleanup"],
                "opensips": ["invite-success", "retention-cleanup"],
            },
            "retention_drain_seconds": 130,
        }

        def gate(
            gate_id: str,
            name: str,
            check: str,
            observed: object,
            evidence: list[dict[str, object]],
        ) -> dict[str, object]:
            return {
                "id": gate_id,
                "name": name,
                "status": "PASS",
                "observed_checks": [
                    {"check": check, "observed": observed, "passed": True}
                ],
                "evidence": evidence,
            }

        pbx_evidence = [
            {"path": "logs/pbx.log", "sha256": sha, "bytes": 1},
            {"path": "pbx/matrix.tsv", "sha256": sha, "bytes": 1},
            {"path": "pbx/summary.md", "sha256": sha, "bytes": 1},
        ]
        proxy_evidence = [
            {"path": "logs/proxy.log", "sha256": sha, "bytes": 1},
            {"path": "proxy-interop/matrix.tsv", "sha256": sha, "bytes": 1},
            {"path": "proxy-interop/summary.json", "sha256": sha, "bytes": 1},
            {
                "path": "proxy-interop/kamailio/rvoip-first/udp/result.json",
                "sha256": sha,
                "bytes": 1,
            },
            {
                "path": "proxy-interop/opensips/rvoip-first/udp/result.json",
                "sha256": sha,
                "bytes": 1,
            },
        ]
        gates = {
            "records": [
                gate(
                    "interop.asterisk-matrix",
                    "local Asterisk PBX matrix",
                    "asterisk PBX matrix rows",
                    {"rows": 4, "pass": 4, "skip": 0},
                    pbx_evidence,
                ),
                gate(
                    "interop.freeswitch-matrix",
                    "local FreeSWITCH PBX matrix",
                    "freeswitch PBX matrix rows",
                    {"rows": 4, "pass": 4, "skip": 0},
                    pbx_evidence,
                ),
                gate(
                    "interop.proxy-stateful-matrix",
                    reporting.PROXY_INTEROP_GATE_NAME,
                    "real Kamailio/OpenSIPS stateful-proxy matrix",
                    proxy_observed,
                    proxy_evidence,
                ),
            ]
        }
        peers = []
        for peer_product in reporting.INTEROP_ATTESTATION_SPECS:
            proxy = peer_product in reporting.PROXY_INTEROP_PEERS
            peers.append(
                {
                    "product": peer_product,
                    "version": f"{peer_product} fixture",
                    "image_digest": f"sha256:{sha}",
                    "config_sha256": sha,
                    "evidence_paths": [
                        (
                            f"proxy-interop/{peer_product}/peer-version.txt"
                            if proxy
                            else (
                                "environment/docker-after-"
                                f"{peer_product}-matrix/rvoip-{peer_product}-peer.json"
                            )
                        )
                    ],
                }
            )
            if not proxy:
                peers.append(
                    {
                        "product": peer_product,
                        "version": (
                            "1" * 40 if peer_product == "asterisk" else "2" * 40
                        ),
                        "image_digest": None,
                        "config_sha256": sha,
                        "evidence_paths": [f"environment/local-pbx/{peer_product}"],
                    }
                )
        source = {
            "source": {
                "start": {
                    "git_commit": "b" * 40,
                    "git_tree": "c" * 40,
                    "source_fingerprint_sha256": "d" * 64,
                }
            },
            "peers": peers,
        }
        binding = {
            "tested_commit": "b" * 40,
            "tested_tree": "c" * 40,
            "source_fingerprint_sha256": "d" * 64,
        }
        value = reporting.build_interop_peer_attestation(source, gates)
        reporting.validate_interop_peer_attestation(value, gates, peers, binding)
        self.assertEqual(
            value["attested_products"],
            ["asterisk", "freeswitch", "kamailio", "opensips"],
        )
        self.assertTrue(all(record["status"] == "PASS" for record in value["records"]))
        rendered = "\n".join(reporting.render_interop_attestation_rows(value))
        for display_name in ("Asterisk", "FreeSWITCH", "Kamailio", "OpenSIPS"):
            self.assertIn(display_name, rendered)

        tampered_attestation = json.loads(json.dumps(value))
        tampered_attestation["records"][0]["scope"] += " tampered"
        with self.assertRaisesRegex(
            reporting.ReportError, "attestation record is incomplete"
        ):
            reporting.validate_interop_peer_attestation(tampered_attestation, gates)

        invalid_hash = json.loads(json.dumps(value))
        invalid_hash["records"][0]["attestation_sha256"] = "0" * 64
        with self.assertRaisesRegex(
            reporting.ReportError, "attestation hash is invalid"
        ):
            reporting.validate_interop_peer_attestation(invalid_hash, gates)

        tampered_coverage = json.loads(json.dumps(value))
        tampered_coverage["records"][0]["coverage"]["matrix_rows"] += 1
        unsigned_record = {
            key: item
            for key, item in tampered_coverage["records"][0].items()
            if key != "attestation_sha256"
        }
        tampered_coverage["records"][0]["attestation_sha256"] = reporting.sha256_bytes(
            reporting.canonical_json(unsigned_record)
        )
        with self.assertRaisesRegex(reporting.ReportError, "disagrees with its gate"):
            reporting.validate_interop_peer_attestation(tampered_coverage, gates, peers)

        tampered_source = json.loads(json.dumps(value))
        tampered_source["source"]["git_commit"] = "e" * 40
        with self.assertRaisesRegex(reporting.ReportError, "source binding is invalid"):
            reporting.validate_interop_peer_attestation(
                tampered_source, gates, peers, binding
            )

        missing_identity = json.loads(json.dumps(source))
        missing_identity["peers"] = [
            peer for peer in missing_identity["peers"] if peer["product"] != "opensips"
        ]
        with self.assertRaisesRegex(
            reporting.ReportError, "OpenSIPS interoperability identity"
        ):
            reporting.build_interop_peer_attestation(missing_identity, gates)

        missing_pbx_source = json.loads(json.dumps(source))
        missing_pbx_source["peers"] = [
            peer
            for peer in missing_pbx_source["peers"]
            if not (peer["product"] == "asterisk" and peer["image_digest"] is None)
        ]
        with self.assertRaisesRegex(
            reporting.ReportError, "Asterisk.*local source revision"
        ):
            reporting.build_interop_peer_attestation(missing_pbx_source, gates)

        incomplete_gates = json.loads(json.dumps(gates))
        incomplete_gates["records"] = [
            record
            for record in incomplete_gates["records"]
            if record["id"] != "interop.asterisk-matrix"
        ]
        with self.assertRaisesRegex(
            reporting.ReportError, "requires the Asterisk, FreeSWITCH"
        ):
            reporting.build_interop_peer_attestation(source, incomplete_gates)

    def test_proxy_tls_verifier_recomputes_row_packet_aggregate(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            proxy = write_proxy_interop_fixture(root)
            row_dir = proxy / "kamailio/rvoip-first/tls"
            for scenario in reporting.PROXY_INTEROP_TLS_PACKET_SCENARIOS:
                scenario_dir = row_dir / "scenarios" / scenario
                packet_path = scenario_dir / "packet-evidence.json"
                packet = json.loads(packet_path.read_text())
                packet["observed_sni"] = [
                    value
                    for value in packet["observed_sni"]
                    if value != "sipp.proxy.test"
                ]
                for assertion in packet["assertions"]:
                    if assertion["name"] == "tls-handshake-sni-valid-when-observed":
                        assertion["observed"]["observed"] = packet["observed_sni"]
                write_json(packet_path, packet)
                result_path = scenario_dir / "result.json"
                result = json.loads(result_path.read_text())
                result["inputs"][packet_path.name] = file_record(packet_path)
                write_json(result_path, result)

            with self.assertRaisesRegex(
                reporting.ReportError,
                "TLS verification failed.*SNI aggregate is incomplete",
            ):
                reporting.validate_proxy_interop_result(root)

    def test_proxy_scenario_results_are_identity_and_hash_bound(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            proxy = write_proxy_interop_fixture(root)
            scenario_dir = (
                proxy / "kamailio/rvoip-first/udp/scenarios/options-readiness"
            )
            result_path = scenario_dir / "result.json"
            original_result = result_path.read_bytes()
            packet_path = scenario_dir.parents[1] / "options-readiness--loopback.pcap"
            original_packet = packet_path.read_bytes()

            result_path.unlink()
            with self.assertRaisesRegex(
                reporting.ReportError, "result.json is missing"
            ):
                reporting.validate_proxy_interop_result(root)
            result_path.write_bytes(original_result)

            payload = json.loads(original_result)
            payload["peer"] = "opensips"
            write_json(result_path, payload)
            with self.assertRaisesRegex(reporting.ReportError, "identity mismatch"):
                reporting.validate_proxy_interop_result(root)
            result_path.write_bytes(original_result)

            packet_path.write_bytes(original_packet + b"tampered")
            with self.assertRaisesRegex(
                reporting.ReportError, "packet capture hash drift"
            ):
                reporting.validate_proxy_interop_result(root)
            packet_path.write_bytes(original_packet)

            payload = json.loads(original_result)
            payload["assertions"][0]["passed"] = False
            write_json(result_path, payload)
            with self.assertRaisesRegex(reporting.ReportError, "assertion failed"):
                reporting.validate_proxy_interop_result(root)
            result_path.write_bytes(original_result)
            reporting.validate_proxy_interop_result(root)

    def test_proxy_packet_assertions_are_scenario_specific(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            proxy = write_proxy_interop_fixture(root)
            scenario_dir = (
                proxy / "kamailio/rvoip-first/udp/scenarios/options-readiness"
            )
            packet_path = scenario_dir / "packet-evidence.json"
            result_path = scenario_dir / "result.json"
            original_packet = packet_path.read_bytes()
            original_result = result_path.read_bytes()

            packet = json.loads(original_packet)
            packet["assertions"][0]["name"] = "generic-success"
            write_json(packet_path, packet)
            result = json.loads(original_result)
            result["inputs"][packet_path.name] = file_record(packet_path)
            write_json(result_path, result)
            with self.assertRaisesRegex(
                reporting.ReportError,
                "missing required assertions.*scenario-call-id-observed",
            ):
                reporting.validate_proxy_interop_result(root)

            packet_path.write_bytes(original_packet)
            result_path.write_bytes(original_result)
            packet = json.loads(original_packet)
            required_status = next(
                item
                for item in packet["assertions"]
                if item["name"] == "required-statuses-observed"
            )
            required_status["observed"]["required"] = [999]
            write_json(packet_path, packet)
            result = json.loads(original_result)
            result["inputs"][packet_path.name] = file_record(packet_path)
            write_json(result_path, result)
            with self.assertRaisesRegex(
                reporting.ReportError,
                "SIP packet assertions disagree with scenario requirements",
            ):
                reporting.validate_proxy_interop_result(root)

    def test_proxy_packet_captures_cannot_be_reused_across_scenarios(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            proxy = write_proxy_interop_fixture(root)
            row_dir = proxy / "kamailio/rvoip-first/udp"
            source_capture = row_dir / "ack-non2xx--loopback.pcap"
            destination_capture = row_dir / "options-readiness--loopback.pcap"
            destination_capture.write_bytes(source_capture.read_bytes())
            scenario_dir = row_dir / "scenarios/options-readiness"
            packet_path = scenario_dir / "packet-evidence.json"
            packet = json.loads(packet_path.read_text())
            packet["captures"][0] = {
                "filename": destination_capture.name,
                **file_record(destination_capture),
            }
            write_json(packet_path, packet)
            result_path = scenario_dir / "result.json"
            result = json.loads(result_path.read_text())
            result["inputs"][packet_path.name] = file_record(packet_path)
            write_json(result_path, result)

            with self.assertRaisesRegex(
                reporting.ReportError,
                "packet capture is reused across scenarios",
            ):
                reporting.validate_proxy_interop_result(root)

    def test_proxy_tls_claim_requires_actual_sips_packet_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            proxy = write_proxy_interop_fixture(root)
            scenario_dir = proxy / "kamailio/rvoip-first/tls/scenarios/sips-routing"
            packet_path = scenario_dir / "packet-evidence.json"
            result_path = scenario_dir / "result.json"
            original_packet = packet_path.read_bytes()
            original_result = result_path.read_bytes()

            packet = json.loads(original_packet)
            packet["assertions"] = [
                item
                for item in packet["assertions"]
                if item["name"] != "sips-request-uri-at-uas-boundary"
            ]
            write_json(packet_path, packet)
            result = json.loads(original_result)
            result["inputs"][packet_path.name] = file_record(packet_path)
            write_json(result_path, result)
            with self.assertRaisesRegex(
                reporting.ReportError,
                "missing required assertions.*sips-request-uri-at-uas-boundary",
            ):
                reporting.validate_proxy_interop_result(root)

            packet_path.write_bytes(original_packet)
            result_path.write_bytes(original_result)
            packet = json.loads(original_packet)
            packet["observed_sips_request_uris"] = ["sip:probe@example.test"]
            preserved = next(
                item
                for item in packet["assertions"]
                if item["name"] == "sips-request-preserved-end-to-end"
            )
            preserved["observed"] = packet["observed_sips_request_uris"]
            write_json(packet_path, packet)
            result = json.loads(original_result)
            result["inputs"][packet_path.name] = file_record(packet_path)
            write_json(result_path, result)
            with self.assertRaisesRegex(
                reporting.ReportError,
                "SIPS packet observations do not prove end-to-end secure routing",
            ):
                reporting.validate_proxy_interop_result(root)

    def test_proxy_advanced_scenario_cannot_move_to_another_matrix_row(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            proxy = write_proxy_interop_fixture(root)
            summary_path = proxy / "summary.json"
            summary = json.loads(summary_path.read_text())
            target = next(
                row
                for row in summary["rows"]
                if (
                    row["peer"],
                    row["order"],
                    row["transport"],
                )
                == ("kamailio", "rvoip-first", "udp")
            )
            target["scenarios"].append("sequential-fork")
            target["scenarios"].sort()
            write_json(summary_path, summary)

            with self.assertRaisesRegex(
                reporting.ReportError,
                r"row scenario contract failed: \('kamailio', "
                r"'rvoip-first', 'udp'\).*misplaced=.*sequential-fork",
            ):
                reporting.validate_proxy_interop_result(root)

    def test_proxy_supplemental_retention_and_runtime_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            proxy = write_proxy_interop_fixture(root)
            supplemental = (
                proxy / "kamailio/rvoip-first/udp/supplemental/sequential-fork"
            )
            supplemental_result = supplemental / "result.json"
            original_result = supplemental_result.read_bytes()
            log = supplemental / "exact-test.log"
            original_log = log.read_bytes()

            payload = json.loads(original_result)
            payload["source"]["fingerprint_sha256"] = "0" * 64
            write_json(supplemental_result, payload)
            with self.assertRaisesRegex(
                reporting.ReportError, "supplemental scenario.*contract is invalid"
            ):
                reporting.validate_proxy_interop_result(root)
            supplemental_result.write_bytes(original_result)

            log.write_bytes(original_log + b"tampered")
            with self.assertRaisesRegex(
                reporting.ReportError, "exact-test log is invalid"
            ):
                reporting.validate_proxy_interop_result(root)
            log.write_bytes(original_log)

            retention_result = (
                proxy / "kamailio/rvoip-first/udp/scenarios/"
                "retention-cleanup/result.json"
            )
            retention_original = retention_result.read_bytes()
            retention = json.loads(retention_original)
            retention["phases"]["post_retention"] = {
                "counters": {"transactions": 1},
                "nonzero": {"transactions": 1},
            }
            write_json(retention_result, retention)
            with self.assertRaisesRegex(
                reporting.ReportError, "retention did not converge"
            ):
                reporting.validate_proxy_interop_result(root)
            retention_result.write_bytes(retention_original)

            runtime_check = proxy / "runtime-state-check.json"
            runtime_original = runtime_check.read_bytes()
            runtime = json.loads(runtime_original)
            runtime["clean"] = False
            write_json(runtime_check, runtime)
            with self.assertRaisesRegex(
                reporting.ReportError, "runtime state did not converge"
            ):
                reporting.validate_proxy_interop_result(root)
            runtime_check.write_bytes(runtime_original)

            private_key = proxy / "accidental.key.pem"
            private_key.write_text(
                "-----BEGIN PRIVATE KEY-----\nfixture\n-----END PRIVATE KEY-----\n"
            )
            with self.assertRaisesRegex(
                reporting.ReportError, "prohibited sensitive or raw state"
            ):
                reporting.validate_proxy_interop_result(root)
            private_key.unlink()
            reporting.validate_proxy_interop_result(root)

    def test_proxy_build_and_peer_version_provenance_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            proxy = write_proxy_interop_fixture(root)
            check_path = proxy / "proxy-binary-check.txt"
            original_check = check_path.read_bytes()
            check_path.write_text(
                "start_sha256=" + "c" * 64 + "\n"
                "end_sha256=" + "d" * 64 + "\n"
                "unchanged=false\n"
            )
            with self.assertRaisesRegex(reporting.ReportError, "proxy binary changed"):
                reporting.validate_proxy_interop_result(root)
            check_path.write_bytes(original_check)

            version_path = proxy / "kamailio/rvoip-first/udp/peer-version.txt"
            original_version = version_path.read_bytes()
            version_path.write_text("version: kamailio 6.1.4\n")
            with self.assertRaisesRegex(reporting.ReportError, "peer version drifted"):
                reporting.validate_proxy_interop_result(root)
            version_path.write_bytes(original_version)

            environment_path = proxy / "environment.txt"
            original_environment = environment_path.read_bytes()
            environment_path.write_text(
                original_environment.decode().replace(
                    "sipp_path: /fixture/sipp", "sipp_path: fixture/sipp"
                )
            )
            with self.assertRaisesRegex(
                reporting.ReportError, "tool path is not absolute"
            ):
                reporting.validate_proxy_interop_result(root)
            environment_path.write_bytes(original_environment)

            command_path = proxy / "cargo-build-command.txt"
            original_command = command_path.read_bytes()
            command_path.write_text(
                "/fixture/cargo build --manifest-path /fixture/Cargo.toml "
                "--release --workspace\n"
            )
            with self.assertRaisesRegex(
                reporting.ReportError, "Cargo build command is not exact"
            ):
                reporting.validate_proxy_interop_result(root)
            command_path.write_bytes(original_command)
            reporting.validate_proxy_interop_result(root)

    def test_proxy_tls_verifier_source_and_opensips_provenance_are_bound(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            proxy = write_proxy_interop_fixture(root)
            scenario_dir = proxy / "opensips/rvoip-first/tls/scenarios/sips-routing"
            result_path = scenario_dir / "result.json"
            original_result = result_path.read_bytes()
            verifier_copy = scenario_dir / "verify_tls_evidence.py"
            original_verifier = verifier_copy.read_bytes()
            verifier_copy.write_bytes(original_verifier + b"\n# tampered\n")
            payload = json.loads(original_result)
            payload["inputs"]["verify_tls_evidence.py"] = file_record(verifier_copy)
            write_json(result_path, payload)
            with self.assertRaisesRegex(
                reporting.ReportError, "verifier source is not bound"
            ):
                reporting.validate_proxy_interop_result(root)
            verifier_copy.write_bytes(original_verifier)
            result_path.write_bytes(original_result)

            provenance_path = (
                proxy / "opensips/rvoip-first/tls/opensips-tls-image-provenance.json"
            )
            original_provenance = provenance_path.read_bytes()
            provenance = json.loads(original_provenance)
            provenance["modules"]["proto_tls.so"]["sha256"] = "0" * 64
            write_json(provenance_path, provenance)
            scenario_provenance = scenario_dir / "opensips-tls-image-provenance.json"
            scenario_provenance.write_bytes(provenance_path.read_bytes())
            payload = json.loads(original_result)
            payload["inputs"][scenario_provenance.name] = file_record(
                scenario_provenance
            )
            write_json(result_path, payload)
            with self.assertRaisesRegex(
                reporting.ReportError, "TLS verification failed"
            ):
                reporting.validate_proxy_interop_result(root)

    def test_proxy_interop_requires_exact_scenario_placement(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            proxy = write_proxy_interop_fixture(root)
            summary_path = proxy / "summary.json"
            summary = json.loads(summary_path.read_text())
            core = sorted(reporting.PROXY_INTEROP_EVERY_ROW_SCENARIOS)
            for row in summary["rows"]:
                if row["peer"] == "opensips":
                    row_scenarios = set(core)
                    if row["transport"] == "tls":
                        row_scenarios.add("sips-routing")
                    row_dir = proxy / row["artifact"] / "scenarios"
                    for scenario_dir in row_dir.iterdir():
                        if scenario_dir.name not in row_scenarios:
                            shutil.rmtree(scenario_dir)
                    row["scenarios"] = sorted(row_scenarios)
            summary_path.write_text(json.dumps(summary, indent=2) + "\n")

            with self.assertRaisesRegex(
                reporting.ReportError,
                r"row scenario contract failed: \('opensips', 'peer-first', "
                r"'tcp'\); missing=.*timer-c-calling",
            ):
                reporting.validate_proxy_interop_result(root)


@unittest.skipUnless(
    available_current_report(), "current verified beta report is unavailable"
)
class CurrentCandidateIntegrationTests(unittest.TestCase):
    def setUp(self) -> None:
        report = available_current_report()
        assert report
        self.report_root = report
        active_policy = reporting.load_policy(POLICY_PATH)
        self.policy_path = (
            CRATE_DIR
            / "docs/releases/beta"
            / reporting.current_snapshot_relative(active_policy)
            / "inputs/beta-release-policy.yaml"
        )
        self.assertTrue(self.policy_path.is_file())

    def test_current_package_maps_exactly_108_required_gates(self) -> None:
        policy = reporting.load_policy(self.policy_path)
        attestation = reporting.validate_source_attestation(self.report_root, policy)
        effective = reporting.effective_configuration(
            self.report_root, attestation, policy
        )
        self.assertFalse(
            [
                item["key"]
                for item in effective["values"]
                if item["key"] not in policy["configuration"]
            ],
            "every effective candidate value must have a typed policy definition",
        )
        gates = reporting.build_gate_results(
            self.report_root, attestation, policy, effective
        )
        self.assertEqual(
            (
                gates["required_count"],
                gates["passed"],
                gates["failed"],
                gates["skipped"],
            ),
            (108, 108, 0, 0),
        )
        self.assertEqual(
            sum(
                gate["evidence_strength"].startswith("legacy-v1")
                for gate in gates["records"]
            ),
            2,
        )

    def test_immutable_v1_report_remains_verifiable(self) -> None:
        snapshot = self.policy_path.parent.parent
        self.assertEqual(
            json.loads((snapshot / "report-attestation.json").read_text())["schema"],
            reporting.SCHEMA_REPORT_ATTESTATION_LEGACY,
        )
        reporting.verify_generated(snapshot, self.policy_path)

    def test_generation_is_deterministic_and_tamper_evident(self) -> None:
        with (
            tempfile.TemporaryDirectory() as first,
            tempfile.TemporaryDirectory() as second,
        ):
            first_path = Path(first)
            second_path = Path(second)
            reporting.generate(self.report_root, self.policy_path, first_path)
            reporting.generate(self.report_root, self.policy_path, second_path)
            for name in reporting.ALL_GENERATED_FILES:
                self.assertEqual(
                    (first_path / name).read_bytes(),
                    (second_path / name).read_bytes(),
                    name,
                )
            reporting.verify_generated(first_path, self.policy_path)
            release = first_path / "BETA_RELEASE_REPORT.md"
            release.write_text(release.read_text() + "\ntampered\n")
            with self.assertRaises(reporting.ReportError):
                reporting.verify_generated(first_path, self.policy_path)

    def test_generated_outputs_have_no_user_absolute_paths(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            reporting.generate(self.report_root, self.policy_path, output)
            for name in reporting.REPORT_FILES + reporting.MACHINE_FILES:
                text = (output / name).read_text()
                self.assertNotIn("/Users/", text, name)

    def test_performance_report_promotes_latency_values_and_limits(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            reporting.generate(self.report_root, self.policy_path, output)
            performance = (output / "BETA_PERFORMANCE_REPORT.md").read_text()
            self.assertIn("### Canonical 2K latency acceptance", performance)
            self.assertIn("p50 observed", performance)
            self.assertIn("p95 limit", performance)
            self.assertIn("p99 observed", performance)
            self.assertIn("| 1 | Call setup | 0.636 | ≤ 13.970 |", performance)
            self.assertIn("## Complete call-setup profile matrix", performance)
            self.assertIn("## Regression evidence", performance)
            self.assertIn("Baseline ms | Limit ms | Observed ms", performance)
            evidence = json.loads((output / "release-evidence.json").read_text())
            self.assertEqual(
                len(evidence["performance"]["regression"]["latency_checks"]), 6
            )
            self.assertTrue(
                all(
                    check["passed"]
                    for run in evidence["performance"]["canonical_2k"]
                    for check in run["latency_checks"]
                )
            )


if __name__ == "__main__":
    unittest.main()
