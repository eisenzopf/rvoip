#!/usr/bin/env python3
"""Tests for the proxy interoperability runtime-state fence."""

from __future__ import annotations

import importlib.util
import pathlib
import unittest


SCRIPT_DIR = pathlib.Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "proxy_interop_state_snapshot", SCRIPT_DIR / "state_snapshot.py"
)
assert SPEC and SPEC.loader
state = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(state)


def fixture() -> dict:
    return {
        "schema": state.SCHEMA,
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


class StateSnapshotTests(unittest.TestCase):
    def test_identical_inventory_is_clean_despite_capture_time(self) -> None:
        before = fixture()
        after = fixture()
        after["captured_at_utc"] = "2026-07-26T01:00:00Z"

        result = state.compare(before, after)

        self.assertTrue(result["clean"])
        self.assertTrue(result["preexisting_state_preserved"])
        self.assertTrue(result["no_added_leftovers"])
        self.assertEqual(result["added_leftovers"], {})

    def test_detects_added_runtime_resources_and_listener(self) -> None:
        before = fixture()
        after = fixture()
        after["docker"]["containers"].append(
            {
                "id": "leaked-container",
                "name": "rvoip-proxy-test",
                "image": "kamailio:test",
                "state": "running",
            }
        )
        after["docker"]["networks"].append(
            {
                "id": "leaked-network",
                "name": "rvoip-proxy-test",
                "driver": "bridge",
                "scope": "local",
            }
        )
        after["docker"]["volumes"].append(
            {"name": "leaked-volume", "driver": "local", "scope": "local"}
        )
        after["listeners"].append(
            {
                "transport": "udp",
                "port": 25070,
                "pid": "202",
                "process": "stateful_proxy",
                "endpoint": "*:25070",
            }
        )

        result = state.compare(before, after)

        self.assertFalse(result["clean"])
        self.assertTrue(result["preexisting_state_preserved"])
        self.assertFalse(result["no_added_leftovers"])
        self.assertEqual(
            set(result["added_leftovers"]),
            {"containers", "networks", "volumes", "listeners"},
        )

    def test_detects_removed_and_changed_preexisting_state(self) -> None:
        before = fixture()
        after = fixture()
        after["docker"]["containers"][0]["state"] = "exited"
        after["docker"]["networks"] = []
        after["listeners"] = []

        result = state.compare(before, after)

        self.assertFalse(result["clean"])
        self.assertFalse(result["preexisting_state_preserved"])
        self.assertTrue(result["no_added_leftovers"])
        self.assertEqual(
            set(result["removed_preexisting"]),
            {"networks", "listeners"},
        )
        self.assertEqual(set(result["changed_preexisting"]), {"containers"})

    def test_rejects_mismatched_ports_and_duplicate_identities(self) -> None:
        before = fixture()
        after = fixture()
        after["ports"] = [25060]
        with self.assertRaises(state.StateError):
            state.compare(before, after)

        after = fixture()
        after["docker"]["containers"].append(
            dict(after["docker"]["containers"][0])
        )
        with self.assertRaises(state.StateError):
            state.compare(before, after)


if __name__ == "__main__":
    unittest.main()
