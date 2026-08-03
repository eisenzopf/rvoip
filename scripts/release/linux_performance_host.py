#!/usr/bin/env python3
"""Capture and validate Linux UDP performance-host evidence.

The release workers deliberately read stable procfs/sysfs counters instead of
parsing distro-specific ``netstat`` output.  Snapshots are written as simple
``key=value`` files to preserve the existing burst-artifact contract.
"""

from __future__ import annotations

import argparse
import time
from pathlib import Path
from typing import Iterable


SNAPSHOT_SCHEMA = "rvoip-linux-udp-snapshot-v1"
DELTA_SCHEMA = "rvoip-linux-udp-delta-v1"

COUNTER_KEYS = (
    "udp_datagrams_received",
    "udp_dropped_no_socket",
    "udp_receive_errors",
    "udp_datagram_output",
    "udp_rcvbuf_errors",
    "udp_sndbuf_errors",
    "udp_dropped_full_socket_buffers",
    "softnet_dropped_total",
    "softnet_time_squeeze_total",
    "loopback_rx_dropped",
    "loopback_tx_dropped",
)

GAUGE_KEYS = (
    "udp_open_sockets",
    "udp_memory_pages",
)

ZERO_DROP_KEYS = (
    "udp_receive_errors",
    "udp_rcvbuf_errors",
    "udp_sndbuf_errors",
    "softnet_dropped_total",
    "loopback_rx_dropped",
    "loopback_tx_dropped",
)


class EvidenceError(RuntimeError):
    """Raised when mandatory Linux performance evidence is unavailable."""


def _read_lines(path: Path, description: str) -> list[str]:
    try:
        return path.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        raise EvidenceError(f"cannot read {description} {path}: {error}") from error


def parse_udp_snmp(path: Path) -> dict[str, int]:
    lines = _read_lines(path, "UDP SNMP counters")
    for index, header_line in enumerate(lines[:-1]):
        if not header_line.startswith("Udp:"):
            continue
        value_line = lines[index + 1]
        if not value_line.startswith("Udp:"):
            continue
        headers = header_line.split()[1:]
        values = value_line.split()[1:]
        if len(headers) != len(values):
            raise EvidenceError("/proc/net/snmp UDP header/value lengths differ")
        try:
            result = dict(zip(headers, map(int, values), strict=True))
        except ValueError as error:
            raise EvidenceError("/proc/net/snmp UDP counters are not integers") from error
        required = {
            "InDatagrams",
            "NoPorts",
            "InErrors",
            "OutDatagrams",
            "RcvbufErrors",
            "SndbufErrors",
        }
        missing = sorted(required - result.keys())
        if missing:
            raise EvidenceError(f"/proc/net/snmp lacks UDP counters: {', '.join(missing)}")
        return result
    raise EvidenceError("/proc/net/snmp does not contain a UDP counter pair")


def parse_udp_sockstat(path: Path) -> dict[str, int]:
    for line in _read_lines(path, "UDP socket statistics"):
        fields = line.split()
        if not fields or fields[0] != "UDP:":
            continue
        values = fields[1:]
        if len(values) % 2:
            raise EvidenceError("/proc/net/sockstat UDP row is malformed")
        try:
            result = {
                values[index]: int(values[index + 1])
                for index in range(0, len(values), 2)
            }
        except ValueError as error:
            raise EvidenceError("/proc/net/sockstat UDP values are not integers") from error
        missing = sorted({"inuse", "mem"} - result.keys())
        if missing:
            raise EvidenceError(f"/proc/net/sockstat lacks UDP fields: {', '.join(missing)}")
        return result
    raise EvidenceError("/proc/net/sockstat does not contain a UDP row")


def parse_softnet(path: Path) -> tuple[int, int]:
    dropped = 0
    time_squeeze = 0
    lines = _read_lines(path, "softnet statistics")
    if not lines:
        raise EvidenceError("/proc/net/softnet_stat is empty")
    for line in lines:
        fields = line.split()
        if len(fields) < 3:
            raise EvidenceError("/proc/net/softnet_stat row has fewer than three fields")
        try:
            dropped += int(fields[1], 16)
            time_squeeze += int(fields[2], 16)
        except ValueError as error:
            raise EvidenceError("/proc/net/softnet_stat fields are not hexadecimal") from error
    return dropped, time_squeeze


def read_integer(path: Path, description: str) -> int:
    lines = _read_lines(path, description)
    if len(lines) != 1:
        raise EvidenceError(f"{description} {path} must contain exactly one line")
    try:
        return int(lines[0].strip())
    except ValueError as error:
        raise EvidenceError(f"{description} {path} is not an integer") from error


def capture_snapshot(proc_root: Path = Path("/proc"), sys_root: Path = Path("/sys")) -> dict[str, int | str]:
    udp = parse_udp_snmp(proc_root / "net/snmp")
    sockstat = parse_udp_sockstat(proc_root / "net/sockstat")
    softnet_dropped, softnet_time_squeeze = parse_softnet(proc_root / "net/softnet_stat")
    loopback = sys_root / "class/net/lo/statistics"
    rcvbuf_errors = udp["RcvbufErrors"]
    sndbuf_errors = udp["SndbufErrors"]
    return {
        "schema": SNAPSHOT_SCHEMA,
        "platform": "linux",
        "timestamp_epoch": int(time.time()),
        "source": "procfs-sysfs",
        "udp_datagrams_received": udp["InDatagrams"],
        "udp_delivered": udp["InDatagrams"],
        "udp_dropped_no_socket": udp["NoPorts"],
        "udp_receive_errors": udp["InErrors"],
        "udp_datagram_output": udp["OutDatagrams"],
        "udp_rcvbuf_errors": rcvbuf_errors,
        "udp_sndbuf_errors": sndbuf_errors,
        "udp_dropped_full_socket_buffers": rcvbuf_errors + sndbuf_errors,
        "udp_open_sockets": sockstat["inuse"],
        "udp_memory_pages": sockstat["mem"],
        "softnet_dropped_total": softnet_dropped,
        "softnet_time_squeeze_total": softnet_time_squeeze,
        "loopback_rx_dropped": read_integer(loopback / "rx_dropped", "loopback RX drops"),
        "loopback_tx_dropped": read_integer(loopback / "tx_dropped", "loopback TX drops"),
    }


def write_key_values(path: Path, values: dict[str, int | str]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    ordered = ["schema", "platform", "timestamp_epoch", "source"]
    keys = [key for key in ordered if key in values]
    keys.extend(sorted(set(values) - set(keys)))
    path.write_text(
        "".join(f"{key}={values[key]}\n" for key in keys),
        encoding="utf-8",
    )


def read_key_values(path: Path) -> dict[str, str]:
    result: dict[str, str] = {}
    for raw in _read_lines(path, "Linux UDP snapshot"):
        if not raw or raw.startswith("#") or "=" not in raw:
            continue
        key, value = raw.split("=", 1)
        result[key] = value
    if result.get("schema") != SNAPSHOT_SCHEMA:
        raise EvidenceError(f"snapshot {path} has an unsupported schema")
    return result


def _required_integer(values: dict[str, str], key: str, path: Path) -> int:
    try:
        return int(values[key])
    except KeyError as error:
        raise EvidenceError(f"snapshot {path} lacks {key}") from error
    except ValueError as error:
        raise EvidenceError(f"snapshot {path} has non-integer {key}") from error


def calculate_delta(before_path: Path, after_path: Path, require_zero_drops: bool) -> dict[str, int | str]:
    before = read_key_values(before_path)
    after = read_key_values(after_path)
    result: dict[str, int | str] = {
        "schema": DELTA_SCHEMA,
        "platform": "linux",
        "before": str(before_path),
        "after": str(after_path),
    }
    deltas: dict[str, int] = {}
    for key in (*COUNTER_KEYS, *GAUGE_KEYS):
        before_value = _required_integer(before, key, before_path)
        after_value = _required_integer(after, key, after_path)
        delta = after_value - before_value
        if key in COUNTER_KEYS and delta < 0:
            raise EvidenceError(f"monotonic Linux counter {key} moved backwards")
        result[f"{key}_before"] = before_value
        result[f"{key}_after"] = after_value
        result[f"{key}_delta"] = delta
        deltas[key] = delta
    if require_zero_drops:
        nonzero = {key: deltas[key] for key in ZERO_DROP_KEYS if deltas[key] != 0}
        if nonzero:
            details = ", ".join(f"{key}={value}" for key, value in sorted(nonzero.items()))
            result["zero_drop_validation"] = "FAIL"
            result["unexpected_drop_deltas"] = details
        else:
            result["zero_drop_validation"] = "PASS"
            result["unexpected_drop_deltas"] = ""
    else:
        result["zero_drop_validation"] = "NOT_REQUESTED"
        result["unexpected_drop_deltas"] = ""
    return result


def validate_delta(result: dict[str, int | str]) -> None:
    if result.get("zero_drop_validation") == "FAIL":
        raise EvidenceError(
            f"unexpected Linux network drop deltas: {result['unexpected_drop_deltas']}"
        )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)

    snapshot = subparsers.add_parser("snapshot")
    snapshot.add_argument("--output", type=Path, required=True)
    snapshot.add_argument("--proc-root", type=Path, default=Path("/proc"))
    snapshot.add_argument("--sys-root", type=Path, default=Path("/sys"))

    delta = subparsers.add_parser("delta")
    delta.add_argument("--before", type=Path, required=True)
    delta.add_argument("--after", type=Path, required=True)
    delta.add_argument("--output", type=Path, required=True)
    delta.add_argument("--require-zero-drops", action="store_true")
    return parser


def main(argv: Iterable[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        if args.command == "snapshot":
            write_key_values(args.output, capture_snapshot(args.proc_root, args.sys_root))
        else:
            result = calculate_delta(args.before, args.after, args.require_zero_drops)
            write_key_values(args.output, result)
            validate_delta(result)
    except EvidenceError as error:
        raise SystemExit(f"Linux performance evidence error: {error}") from error
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
