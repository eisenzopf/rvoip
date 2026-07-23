#!/usr/bin/env python3
"""Create an allowlisted, secret-free peer snapshot from ``docker inspect``.

The raw inspect document is read only from stdin and is never written to disk.
Only fields needed to identify the peer image, runtime state, published ports,
and network attachment are retained. In particular, Config.Env, labels,
commands, mounts, and health-check output are deliberately excluded.
"""

from __future__ import annotations

import argparse
import json
import sys
from typing import Any


SCHEMA = "rvoip-docker-peer-snapshot-v1"


class SnapshotError(RuntimeError):
    """Raised when Docker inspect input is not structurally usable."""


def mapping(value: Any) -> dict[str, Any]:
    return value if isinstance(value, dict) else {}


def scalar(value: Any) -> str | int | bool | None:
    return value if isinstance(value, (str, int, bool)) else None


def compact(value: dict[str, Any]) -> dict[str, Any]:
    return {key: child for key, child in value.items() if child is not None}


def published_ports(value: Any) -> dict[str, list[dict[str, str]]]:
    result: dict[str, list[dict[str, str]]] = {}
    for container_port, bindings in sorted(mapping(value).items()):
        if not isinstance(container_port, str) or not isinstance(bindings, list):
            continue
        safe_bindings = []
        for binding in bindings:
            item = mapping(binding)
            host_ip = item.get("HostIp")
            host_port = item.get("HostPort")
            if isinstance(host_ip, str) and isinstance(host_port, str):
                safe_bindings.append({"host_ip": host_ip, "host_port": host_port})
        result[container_port] = safe_bindings
    return result


def networks(value: Any) -> dict[str, dict[str, Any]]:
    result: dict[str, dict[str, Any]] = {}
    allowed = {
        "NetworkID": "network_id",
        "EndpointID": "endpoint_id",
        "Gateway": "gateway",
        "IPAddress": "ip_address",
        "IPPrefixLen": "ip_prefix_len",
        "IPv6Gateway": "ipv6_gateway",
        "GlobalIPv6Address": "global_ipv6_address",
        "GlobalIPv6PrefixLen": "global_ipv6_prefix_len",
        "MacAddress": "mac_address",
    }
    for name, raw_network in sorted(mapping(value).items()):
        if not isinstance(name, str):
            continue
        network = mapping(raw_network)
        result[name] = compact(
            {output: scalar(network.get(source)) for source, output in allowed.items()}
        )
    return result


def sanitize_inspect(payload: Any, product: str) -> dict[str, Any]:
    if not isinstance(payload, list) or len(payload) != 1:
        raise SnapshotError("docker inspect must contain exactly one container")
    container = mapping(payload[0])
    if not container:
        raise SnapshotError("docker inspect container entry must be an object")

    config = mapping(container.get("Config"))
    host_config = mapping(container.get("HostConfig"))
    restart = mapping(host_config.get("RestartPolicy"))
    state = mapping(container.get("State"))
    health = mapping(state.get("Health"))
    network_settings = mapping(container.get("NetworkSettings"))

    image_id = container.get("Image")
    if not isinstance(image_id, str) or not image_id:
        raise SnapshotError("docker inspect Image is missing")

    return {
        "schema": SCHEMA,
        "product": product,
        "container": compact(
            {
                "name": scalar(container.get("Name")),
                "id": scalar(container.get("Id")),
                "created": scalar(container.get("Created")),
                "platform": scalar(container.get("Platform")),
            }
        ),
        "image": compact(
            {
                "id": image_id,
                "reference": scalar(config.get("Image")),
            }
        ),
        "configuration": {
            "network_mode": scalar(host_config.get("NetworkMode")),
            "published_ports": published_ports(host_config.get("PortBindings")),
            "exposed_ports": sorted(
                key for key in mapping(config.get("ExposedPorts")) if isinstance(key, str)
            ),
            "restart_policy": compact(
                {
                    "name": scalar(restart.get("Name")),
                    "maximum_retry_count": scalar(restart.get("MaximumRetryCount")),
                }
            ),
        },
        "state": compact(
            {
                "status": scalar(state.get("Status")),
                "running": scalar(state.get("Running")),
                "paused": scalar(state.get("Paused")),
                "restarting": scalar(state.get("Restarting")),
                "oom_killed": scalar(state.get("OOMKilled")),
                "dead": scalar(state.get("Dead")),
                "exit_code": scalar(state.get("ExitCode")),
                "started_at": scalar(state.get("StartedAt")),
                "finished_at": scalar(state.get("FinishedAt")),
                "health_status": scalar(health.get("Status")),
            }
        ),
        "network": {
            "published_ports": published_ports(network_settings.get("Ports")),
            "networks": networks(network_settings.get("Networks")),
        },
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--product", required=True)
    args = parser.parse_args(argv)
    try:
        payload = json.load(sys.stdin)
        snapshot = sanitize_inspect(payload, args.product)
    except (OSError, ValueError, SnapshotError) as error:
        print(f"docker peer snapshot: FAIL: {error}", file=sys.stderr)
        return 1
    json.dump(snapshot, sys.stdout, indent=2, sort_keys=True)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
