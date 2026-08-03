#!/usr/bin/env bash
set -Eeuo pipefail

EXPECTED_RESOURCE="${1:?expected GCP resource class is required}"
ARTIFACT_DIR="${2:?artifact directory is required}"

case "$EXPECTED_RESOURCE" in
  gcp-performance|gcp-performance-soak-long)
    EXPECTED_VCPUS=8
    EXPECTED_MEMORY_GIB=28
    EXPECTED_DISK_GIB=180
    ;;
  gcp-performance-soak|gcp-interop)
    EXPECTED_VCPUS=4
    EXPECTED_MEMORY_GIB=14
    EXPECTED_DISK_GIB=180
    ;;
  gcp-proxy-interop)
    EXPECTED_VCPUS=2
    EXPECTED_MEMORY_GIB=7
    EXPECTED_DISK_GIB=90
    ;;
  *)
    echo "unsupported preflight resource class: $EXPECTED_RESOURCE" >&2
    exit 2
    ;;
esac

test "${RVOIP_RELEASE_RESOURCE_CLASS:-}" = "$EXPECTED_RESOURCE"
test -n "${RVOIP_RELEASE_CANDIDATE:-}"
test -n "${RVOIP_RELEASE_SHARD_ID:-}"
test -n "${RVOIP_RELEASE_GATES:-}"
test "$(git rev-parse HEAD)" = "$RVOIP_RELEASE_CANDIDATE"
test -z "${CARGO_REGISTRY_TOKEN:-}"
test -z "${CRATES_IO_TOKEN:-}"

ACTUAL_VCPUS="$(nproc)"
NOFILE_SOFT="$(ulimit -Sn)"
NOFILE_HARD="$(ulimit -Hn)"
MEMORY_KIB="$(awk '/^MemTotal:/ { print $2 }' /proc/meminfo)"
SWAP_TOTAL_KIB="$(awk '/^SwapTotal:/ { print $2 }' /proc/meminfo)"
DISK_BYTES="$(df --output=size -B1 / | tail -n 1 | tr -d ' ')"
FS_NR_OPEN="$(sysctl -n fs.nr_open)"
FS_FILE_MAX="$(sysctl -n fs.file-max)"
FS_FILE_NR="$(sysctl -n fs.file-nr)"
IP_LOCAL_PORT_RANGE="$(sysctl -n net.ipv4.ip_local_port_range)"
IP_LOCAL_RESERVED_PORTS="$(sysctl -n net.ipv4.ip_local_reserved_ports)"
RMEM_DEFAULT="$(sysctl -n net.core.rmem_default)"
RMEM_MAX="$(sysctl -n net.core.rmem_max)"
WMEM_DEFAULT="$(sysctl -n net.core.wmem_default)"
WMEM_MAX="$(sysctl -n net.core.wmem_max)"
UDP_MEM="$(sysctl -n net.ipv4.udp_mem)"
CPU_MODEL="$(awk -F: '/^model name/ { sub(/^[ \t]+/, "", $2); print $2; exit }' /proc/cpuinfo)"

test "$ACTUAL_VCPUS" -ge "$EXPECTED_VCPUS"
test "$NOFILE_SOFT" -ge 262144
test "$NOFILE_HARD" -ge "$NOFILE_SOFT"
test "$FS_NR_OPEN" -ge "$NOFILE_HARD"
test "$MEMORY_KIB" -ge "$(( EXPECTED_MEMORY_GIB * 1024 * 1024 ))"
test "$DISK_BYTES" -ge "$(( EXPECTED_DISK_GIB * 1024 * 1024 * 1024 ))"
test "$RMEM_MAX" -ge 8388608
test "$WMEM_MAX" -ge 8388608
test -n "$CPU_MODEL"

for telemetry in \
  /proc/net/snmp \
  /proc/net/sockstat \
  /proc/net/softnet_stat \
  /proc/pressure/cpu \
  /proc/pressure/memory \
  /proc/pressure/io \
  /sys/class/net/lo/statistics/rx_dropped \
  /sys/class/net/lo/statistics/tx_dropped; do
  test -r "$telemetry"
done

for program in cargo cmake git jq pkg-config protoc rustc; do
  command -v "$program" >/dev/null
done

if [[ "$EXPECTED_RESOURCE" == gcp-interop \
  || "$EXPECTED_RESOURCE" == gcp-proxy-interop ]]; then
  command -v sipp >/dev/null
  command -v tshark >/dev/null
  command -v docker >/dev/null
  docker compose version >/dev/null
  systemctl is-active --quiet docker
elif [[ ",$RVOIP_RELEASE_GATES," == *",preflight.performance-01,"* ]]; then
  # This worker exercises the conditional SIPp dependency path used by the
  # real perf.sipp-parity release gate.
  command -v sipp >/dev/null
fi

mkdir -p "$ARTIFACT_DIR"
cargo metadata --locked --no-deps --format-version 1 \
  > "$ARTIFACT_DIR/cargo-metadata.json"

# Preserve the host contract independently of the summarized JSON below.
cat /proc/self/limits > "$ARTIFACT_DIR/process-limits.txt"
cat /proc/meminfo > "$ARTIFACT_DIR/meminfo.txt"
cat /proc/stat > "$ARTIFACT_DIR/proc-stat.txt"
cat /proc/pressure/cpu > "$ARTIFACT_DIR/pressure-cpu.txt"
cat /proc/pressure/memory > "$ARTIFACT_DIR/pressure-memory.txt"
cat /proc/pressure/io > "$ARTIFACT_DIR/pressure-io.txt"
lscpu > "$ARTIFACT_DIR/lscpu.txt"
python3 scripts/release/linux_performance_host.py snapshot \
  --output "$ARTIFACT_DIR/linux-udp-snapshot.txt"

# Prove that this kernel grants the largest current SIP recipe request. Linux
# reports doubled values for SO_RCVBUF/SO_SNDBUF accounting; the pass condition
# intentionally checks only that the returned capacity is not below the
# application request.
python3 - "$ARTIFACT_DIR/socket-buffer-probe.json" <<'PY'
import json
import socket
import sys

requested = 8_388_608
probe = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
try:
    probe.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, requested)
    probe.setsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF, requested)
    effective_receive = probe.getsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF)
    effective_send = probe.getsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF)
finally:
    probe.close()
if effective_receive < requested or effective_send < requested:
    raise SystemExit(
        "kernel did not grant required SIP UDP buffers: "
        f"requested={requested} receive={effective_receive} send={effective_send}"
    )
payload = {
    "schema": "rvoip-linux-socket-buffer-probe-v1",
    "requested_receive_bytes": requested,
    "requested_send_bytes": requested,
    "effective_receive_bytes": effective_receive,
    "effective_send_bytes": effective_send,
    "linux_returned_value_includes_accounting_overhead": True,
    "status": "PASS",
}
with open(sys.argv[1], "w", encoding="utf-8") as output:
    json.dump(payload, output, indent=2, sort_keys=True)
    output.write("\n")
PY

python3 - "$ARTIFACT_DIR/cargo-metadata.json" <<'PY'
import json
import sys

metadata = json.load(open(sys.argv[1], encoding="utf-8"))
members = set(metadata["workspace_members"])
publishable = [
    package
    for package in metadata["packages"]
    if package["id"] in members and package.get("publish") != []
]
if len(publishable) != 44:
    raise SystemExit(f"expected 44 publishable workspace packages, found {len(publishable)}")
PY

# Opening thousands of descriptors catches the stock systemd soft-limit bug
# in seconds instead of after a high-density performance gate has compiled.
python3 - <<'PY'
import socket

sockets = []
try:
    for _ in range(4096):
        item = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        item.bind(("127.0.0.1", 0))
        sockets.append(item)
finally:
    for item in sockets:
        item.close()
PY

python3 - \
  "$ARTIFACT_DIR/system.json" \
  "$EXPECTED_RESOURCE" \
  "$ACTUAL_VCPUS" \
  "$NOFILE_SOFT" \
  "$NOFILE_HARD" \
  "$MEMORY_KIB" \
  "$SWAP_TOTAL_KIB" \
  "$DISK_BYTES" \
  "$FS_NR_OPEN" \
  "$FS_FILE_MAX" \
  "$FS_FILE_NR" \
  "$IP_LOCAL_PORT_RANGE" \
  "$IP_LOCAL_RESERVED_PORTS" \
  "$RMEM_DEFAULT" \
  "$RMEM_MAX" \
  "$WMEM_DEFAULT" \
  "$WMEM_MAX" \
  "$UDP_MEM" \
  "$CPU_MODEL" <<'PY'
import json
import os
import platform
import sys

(
    path,
    resource,
    vcpus,
    nofile_soft,
    nofile_hard,
    memory_kib,
    swap_total_kib,
    disk_bytes,
    fs_nr_open,
    fs_file_max,
    fs_file_nr,
    ip_local_port_range,
    ip_local_reserved_ports,
    rmem_default,
    rmem_max,
    wmem_default,
    wmem_max,
    udp_mem,
    cpu_model,
) = sys.argv[1:]
with open(os.path.join(os.path.dirname(path), "socket-buffer-probe.json"), encoding="utf-8") as source:
    socket_buffer_probe = json.load(source)
payload = {
    "schema": "rvoip-release-infrastructure-preflight-v1",
    "candidate_sha": os.environ["RVOIP_RELEASE_CANDIDATE"],
    "cpu_model": cpu_model,
    "gate_ids": sorted(filter(None, os.environ["RVOIP_RELEASE_GATES"].split(","))),
    "fs_file_max": int(fs_file_max),
    "fs_file_nr": [int(value) for value in fs_file_nr.split()],
    "fs_nr_open": int(fs_nr_open),
    "ip_local_port_range": [int(value) for value in ip_local_port_range.split()],
    "ip_local_reserved_ports": ip_local_reserved_ports,
    "kernel": platform.release(),
    "memory_kib": int(memory_kib),
    "nofile_hard": int(nofile_hard),
    "nofile_limit": int(nofile_soft),
    "nofile_soft": int(nofile_soft),
    "publishing_credentials_present": False,
    "resource_class": resource,
    "rmem_default": int(rmem_default),
    "rmem_max": int(rmem_max),
    "root_disk_bytes": int(disk_bytes),
    "rust_test_threads": os.environ.get("RUST_TEST_THREADS"),
    "shard_id": os.environ["RVOIP_RELEASE_SHARD_ID"],
    "socket_buffer_probe": socket_buffer_probe,
    "swap_total_kib": int(swap_total_kib),
    "udp_mem": [int(value) for value in udp_mem.split()],
    "vcpus": int(vcpus),
    "wmem_default": int(wmem_default),
    "wmem_max": int(wmem_max),
}
with open(path, "w", encoding="utf-8") as output:
    json.dump(payload, output, indent=2, sort_keys=True)
    output.write("\n")
PY
