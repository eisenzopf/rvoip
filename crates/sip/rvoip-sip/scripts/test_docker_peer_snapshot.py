#!/usr/bin/env python3
"""Tests for the allowlisted Docker peer evidence snapshot."""

from __future__ import annotations

import importlib.util
import json
import pathlib
import unittest


SCRIPT_DIR = pathlib.Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "test_docker_peer_snapshot_impl", SCRIPT_DIR / "docker_peer_snapshot.py"
)
snapshot = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(snapshot)


class DockerPeerSnapshotTests(unittest.TestCase):
    def test_secret_bearing_and_unbounded_fields_are_absent(self) -> None:
        secret_values = {
            "super-secret-password",
            "secret-token-value",
            "authorization-value",
            "health-output-secret",
        }
        raw = [
            {
                "Name": "/rvoip-freeswitch",
                "Id": "container-id",
                "Image": "sha256:image-id",
                "Created": "2026-07-21T00:00:00Z",
                "Platform": "linux",
                "Config": {
                    "Image": "rvoip-freeswitch:local",
                    "Env": [
                        "FS_DEFAULT_PASSWORD=super-secret-password",
                        "TOKEN=secret-token-value",
                    ],
                    "Labels": {"authorization": "authorization-value"},
                    "Cmd": ["--password", "super-secret-password"],
                    "ExposedPorts": {"5060/udp": {}, "5063/tcp": {}},
                },
                "HostConfig": {
                    "NetworkMode": "freeswitch-local",
                    "PortBindings": {
                        "5060/udp": [{"HostIp": "0.0.0.0", "HostPort": "5060"}]
                    },
                    "RestartPolicy": {"Name": "unless-stopped", "MaximumRetryCount": 0},
                },
                "State": {
                    "Status": "running",
                    "Running": True,
                    "ExitCode": 0,
                    "Health": {
                        "Status": "healthy",
                        "Log": [{"Output": "health-output-secret"}],
                    },
                },
                "Mounts": [{"Source": "/secret/host/path"}],
                "NetworkSettings": {
                    "Ports": {
                        "5060/udp": [{"HostIp": "0.0.0.0", "HostPort": "5060"}]
                    },
                    "Networks": {
                        "freeswitch-local": {
                            "NetworkID": "network-id",
                            "EndpointID": "endpoint-id",
                            "Gateway": "172.20.0.1",
                            "IPAddress": "172.20.0.2",
                            "IPPrefixLen": 16,
                            "Aliases": ["secret-token-value"],
                        }
                    },
                },
            }
        ]

        value = snapshot.sanitize_inspect(raw, "freeswitch")
        encoded = json.dumps(value, sort_keys=True)
        self.assertEqual(value["schema"], snapshot.SCHEMA)
        self.assertEqual(value["image"]["id"], "sha256:image-id")
        self.assertEqual(value["state"]["health_status"], "healthy")
        for secret in secret_values:
            self.assertNotIn(secret, encoded)
        for forbidden_key in ("Env", "Labels", "Cmd", "Mounts", "Log"):
            self.assertNotIn(f'"{forbidden_key}"', encoded)

    def test_requires_one_container_with_an_image_id(self) -> None:
        for payload in ([], [{}, {}], [{}]):
            with self.subTest(payload=payload):
                with self.assertRaises(snapshot.SnapshotError):
                    snapshot.sanitize_inspect(payload, "asterisk")


if __name__ == "__main__":
    unittest.main()
