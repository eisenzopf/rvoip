#!/usr/bin/env python3
"""Capture and compare secret-free runtime state for proxy interoperability.

The release harness uses this helper before and after its real Kamailio and
OpenSIPS matrix.  It deliberately records only allowlisted Docker inventory
fields and listener identities on the explicitly supplied test ports.  It
never persists ``docker inspect`` output, process arguments, or environment
variables.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import pathlib
import re
import shutil
import subprocess
import sys
from typing import Any, Callable, Iterable


SCHEMA = "rvoip-sip-proxy-interop-runtime-state-v1"


class StateError(RuntimeError):
    """Raised when a complete, trustworthy state snapshot cannot be made."""


def utc_now() -> str:
    return dt.datetime.now(dt.UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def run(command: list[str], *, empty_exit_codes: set[int] | None = None) -> str:
    result = subprocess.run(
        command,
        check=False,
        capture_output=True,
        text=True,
    )
    allowed = {0} | (empty_exit_codes or set())
    if result.returncode not in allowed:
        detail = result.stderr.strip() or result.stdout.strip() or "no diagnostic"
        raise StateError(f"{command[0]} failed ({result.returncode}): {detail}")
    return result.stdout


def json_lines(
    output: str,
    *,
    identity: str,
    allowed_fields: dict[str, str],
) -> list[dict[str, str]]:
    records: list[dict[str, str]] = []
    identities: set[str] = set()
    for line_number, raw in enumerate(output.splitlines(), 1):
        if not raw.strip():
            continue
        try:
            value = json.loads(raw)
        except json.JSONDecodeError as error:
            raise StateError(f"invalid Docker JSON on line {line_number}: {error}") from error
        if not isinstance(value, dict):
            raise StateError(f"Docker JSON line {line_number} is not an object")
        record = {
            destination: str(value[source])
            for source, destination in allowed_fields.items()
            if isinstance(value.get(source), (str, int, bool))
        }
        record_identity = record.get(identity)
        if not record_identity:
            raise StateError(
                f"Docker JSON line {line_number} lacks identity field {identity!r}"
            )
        if record_identity in identities:
            raise StateError(f"duplicate Docker identity {record_identity!r}")
        identities.add(record_identity)
        records.append(record)
    return sorted(records, key=lambda item: item[identity])


def docker_inventory() -> dict[str, list[dict[str, str]]]:
    if shutil.which("docker") is None:
        raise StateError("docker is required")
    containers = json_lines(
        run(
            [
                "docker",
                "container",
                "ls",
                "--all",
                "--no-trunc",
                "--format",
                "{{json .}}",
            ]
        ),
        identity="id",
        allowed_fields={
            "ID": "id",
            "Names": "name",
            "Image": "image",
            "State": "state",
        },
    )
    networks = json_lines(
        run(
            [
                "docker",
                "network",
                "ls",
                "--no-trunc",
                "--format",
                "{{json .}}",
            ]
        ),
        identity="id",
        allowed_fields={
            "ID": "id",
            "Name": "name",
            "Driver": "driver",
            "Scope": "scope",
        },
    )
    volumes = json_lines(
        run(["docker", "volume", "ls", "--format", "{{json .}}"]),
        identity="name",
        allowed_fields={
            "Name": "name",
            "Driver": "driver",
            "Scope": "scope",
        },
    )
    return {
        "containers": containers,
        "networks": networks,
        "volumes": volumes,
    }


def _lsof_listeners(ports: list[int]) -> list[dict[str, str | int]]:
    records: set[tuple[str, int, str, str, str]] = set()
    for transport, selector in (("tcp", "TCP"), ("udp", "UDP")):
        for port in ports:
            command = ["lsof", "-nP", f"-i{selector}:{port}", "-Fpcn"]
            if transport == "tcp":
                command.insert(3, "-sTCP:LISTEN")
            output = run(command, empty_exit_codes={1})
            pid = ""
            process = ""
            for raw in output.splitlines():
                if raw.startswith("p"):
                    pid = raw[1:]
                    process = ""
                elif raw.startswith("c"):
                    process = raw[1:]
                elif raw.startswith("n") and pid:
                    records.add((transport, port, pid, process, raw[1:]))
    return [
        {
            "transport": transport,
            "port": port,
            "pid": pid,
            "process": process,
            "endpoint": endpoint,
        }
        for transport, port, pid, process, endpoint in sorted(records)
    ]


def _ss_listeners(ports: list[int]) -> list[dict[str, str | int]]:
    output = run(["ss", "-H", "-l", "-n", "-p", "-t", "-u"])
    records: set[tuple[str, int, str, str, str]] = set()
    pid_pattern = re.compile(r'\("([^"]+)",pid=(\d+)')
    for raw in output.splitlines():
        fields = raw.split()
        if len(fields) < 5 or fields[0] not in {"tcp", "udp"}:
            continue
        transport = fields[0]
        endpoint = fields[4]
        match = re.search(r":(\d+)$", endpoint.rsplit("%", 1)[0])
        if match is None:
            continue
        port = int(match.group(1))
        if port not in ports:
            continue
        process_matches = pid_pattern.findall(raw)
        if process_matches:
            for process, pid in process_matches:
                records.add((transport, port, pid, process, endpoint))
        else:
            records.add((transport, port, "", "", endpoint))
    return [
        {
            "transport": transport,
            "port": port,
            "pid": pid,
            "process": process,
            "endpoint": endpoint,
        }
        for transport, port, pid, process, endpoint in sorted(records)
    ]


def listeners(ports: list[int]) -> tuple[str, list[dict[str, str | int]]]:
    if shutil.which("lsof") is not None:
        return "lsof", _lsof_listeners(ports)
    if shutil.which("ss") is not None:
        return "ss", _ss_listeners(ports)
    raise StateError("either lsof or ss is required to inspect test-port listeners")


def snapshot(ports: list[int]) -> dict[str, Any]:
    listener_collector, listener_records = listeners(ports)
    return {
        "schema": SCHEMA,
        "kind": "snapshot",
        "captured_at_utc": utc_now(),
        "ports": ports,
        "collectors": {
            "docker": "docker-list-json",
            "listeners": listener_collector,
        },
        "docker": docker_inventory(),
        "listeners": listener_records,
    }


def require_mapping(value: Any, name: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise StateError(f"{name} must be an object")
    return value


def require_records(value: Any, name: str) -> list[dict[str, Any]]:
    if not isinstance(value, list) or not all(isinstance(item, dict) for item in value):
        raise StateError(f"{name} must be an array of objects")
    return value


def indexed(
    records: Iterable[dict[str, Any]],
    identity: Callable[[dict[str, Any]], str],
    name: str,
) -> dict[str, dict[str, Any]]:
    result: dict[str, dict[str, Any]] = {}
    for record in records:
        key = identity(record)
        if not key:
            raise StateError(f"{name} record lacks an identity")
        if key in result:
            raise StateError(f"{name} contains duplicate identity {key!r}")
        result[key] = record
    return result


def diff_records(
    before: list[dict[str, Any]],
    after: list[dict[str, Any]],
    *,
    identity: Callable[[dict[str, Any]], str],
    name: str,
) -> dict[str, list[Any]]:
    initial = indexed(before, identity, name)
    final = indexed(after, identity, name)
    added = [final[key] for key in sorted(final.keys() - initial.keys())]
    removed = [initial[key] for key in sorted(initial.keys() - final.keys())]
    changed = [
        {"before": initial[key], "after": final[key]}
        for key in sorted(initial.keys() & final.keys())
        if initial[key] != final[key]
    ]
    return {
        "added": added,
        "removed": removed,
        "changed": changed,
    }


def listener_identity(record: dict[str, Any]) -> str:
    fields = ("transport", "port", "pid", "process", "endpoint")
    return "\x1f".join(str(record.get(field, "")) for field in fields)


def docker_identity(field: str) -> Callable[[dict[str, Any]], str]:
    return lambda record: str(record.get(field, ""))


def compare(before: dict[str, Any], after: dict[str, Any]) -> dict[str, Any]:
    for label, value in (("before", before), ("after", after)):
        if value.get("schema") != SCHEMA or value.get("kind") != "snapshot":
            raise StateError(f"{label} snapshot schema is invalid")
    if before.get("ports") != after.get("ports"):
        raise StateError("before and after snapshots cover different test ports")

    before_docker = require_mapping(before.get("docker"), "before.docker")
    after_docker = require_mapping(after.get("docker"), "after.docker")
    differences = {
        "containers": diff_records(
            require_records(before_docker.get("containers"), "before.docker.containers"),
            require_records(after_docker.get("containers"), "after.docker.containers"),
            identity=docker_identity("id"),
            name="containers",
        ),
        "networks": diff_records(
            require_records(before_docker.get("networks"), "before.docker.networks"),
            require_records(after_docker.get("networks"), "after.docker.networks"),
            identity=docker_identity("id"),
            name="networks",
        ),
        "volumes": diff_records(
            require_records(before_docker.get("volumes"), "before.docker.volumes"),
            require_records(after_docker.get("volumes"), "after.docker.volumes"),
            identity=docker_identity("name"),
            name="volumes",
        ),
        "listeners": diff_records(
            require_records(before.get("listeners"), "before.listeners"),
            require_records(after.get("listeners"), "after.listeners"),
            identity=listener_identity,
            name="listeners",
        ),
    }
    clean = all(
        not category["added"]
        and not category["removed"]
        and not category["changed"]
        for category in differences.values()
    )
    added_leftovers = {
        name: value["added"]
        for name, value in differences.items()
        if value["added"]
    }
    removed_preexisting = {
        name: value["removed"]
        for name, value in differences.items()
        if value["removed"]
    }
    changed_preexisting = {
        name: value["changed"]
        for name, value in differences.items()
        if value["changed"]
    }
    return {
        "schema": SCHEMA,
        "kind": "comparison",
        "compared_at_utc": utc_now(),
        "ports": before["ports"],
        "clean": clean,
        "preexisting_state_preserved": not removed_preexisting
        and not changed_preexisting,
        "no_added_leftovers": not added_leftovers,
        "added_leftovers": added_leftovers,
        "removed_preexisting": removed_preexisting,
        "changed_preexisting": changed_preexisting,
        "differences": differences,
    }


def read_json(path: pathlib.Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise StateError(f"cannot read {path}: {error}") from error
    return require_mapping(value, str(path))


def write_json(path: pathlib.Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    subcommands = root.add_subparsers(dest="command", required=True)

    capture = subcommands.add_parser("snapshot", help="capture current runtime state")
    capture.add_argument("--output", type=pathlib.Path, required=True)
    capture.add_argument("--port", type=int, action="append", required=True)

    comparison = subcommands.add_parser("compare", help="compare two snapshots")
    comparison.add_argument("--before", type=pathlib.Path, required=True)
    comparison.add_argument("--after", type=pathlib.Path, required=True)
    comparison.add_argument("--output", type=pathlib.Path, required=True)
    return root


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    try:
        if args.command == "snapshot":
            ports = sorted(set(args.port))
            if any(port < 1 or port > 65535 for port in ports):
                raise StateError("ports must be in the range 1..65535")
            value = snapshot(ports)
        else:
            value = compare(read_json(args.before), read_json(args.after))
        write_json(args.output, value)
    except StateError as error:
        print(f"proxy interop runtime state: FAIL: {error}", file=sys.stderr)
        return 2
    if args.command == "compare" and not value["clean"]:
        print("proxy interop runtime state: FAIL: state changed", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
