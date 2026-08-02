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
NOFILE_LIMIT="$(ulimit -n)"
MEMORY_KIB="$(awk '/^MemTotal:/ { print $2 }' /proc/meminfo)"
DISK_BYTES="$(df --output=size -B1 / | tail -n 1 | tr -d ' ')"

test "$ACTUAL_VCPUS" -ge "$EXPECTED_VCPUS"
test "$NOFILE_LIMIT" -ge 262144
test "$MEMORY_KIB" -ge "$(( EXPECTED_MEMORY_GIB * 1024 * 1024 ))"
test "$DISK_BYTES" -ge "$(( EXPECTED_DISK_GIB * 1024 * 1024 * 1024 ))"

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
  "$NOFILE_LIMIT" \
  "$MEMORY_KIB" \
  "$DISK_BYTES" <<'PY'
import json
import os
import platform
import sys

path, resource, vcpus, nofile, memory_kib, disk_bytes = sys.argv[1:]
payload = {
    "schema": "rvoip-release-infrastructure-preflight-v1",
    "candidate_sha": os.environ["RVOIP_RELEASE_CANDIDATE"],
    "gate_ids": sorted(filter(None, os.environ["RVOIP_RELEASE_GATES"].split(","))),
    "kernel": platform.release(),
    "memory_kib": int(memory_kib),
    "nofile_limit": int(nofile),
    "publishing_credentials_present": False,
    "resource_class": resource,
    "root_disk_bytes": int(disk_bytes),
    "shard_id": os.environ["RVOIP_RELEASE_SHARD_ID"],
    "vcpus": int(vcpus),
}
with open(path, "w", encoding="utf-8") as output:
    json.dump(payload, output, indent=2, sort_keys=True)
    output.write("\n")
PY
