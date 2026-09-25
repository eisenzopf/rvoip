#!/usr/bin/env bash
# Produce three fresh, exact-candidate 2,000-CPS runs for a release receipt.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/../../../.." && pwd)"
RUNNER="${SCRIPT_DIR}/perf_call_setup_2k_profile.sh"
EVIDENCE_TOOL="${SCRIPT_DIR}/canonical_2k_evidence.py"
OUTPUT="${RVOIP_CANONICAL_EVAL_OUTPUT:?RVOIP_CANONICAL_EVAL_OUTPUT is required}"
RVOIP_CANONICAL_LOGIN_HOME="$(python3 -c 'import os, pwd; print(pwd.getpwuid(os.getuid()).pw_dir)')"
if [[ -z "${RVOIP_CANONICAL_LOGIN_HOME}" || ! -d "${RVOIP_CANONICAL_LOGIN_HOME}" ]]; then
  echo "unable to resolve the current account's home directory" >&2
  exit 1
fi

mkdir -p "${OUTPUT}/run-logs"
python3 "${EVIDENCE_TOOL}" fingerprint \
  --workspace-root "${WORKSPACE_ROOT}" \
  --out "${OUTPUT}/beta-start.json"

run_dirs=()
for pass in 1 2 3; do
  log="${OUTPUT}/run-logs/pass-${pass}.log"
  worker_env=(
    HOME="${RVOIP_CANONICAL_LOGIN_HOME}"
    USER="${USER:-}"
    LOGNAME="${LOGNAME:-${USER:-}}"
    PATH="${PATH}"
    TMPDIR=/tmp
    LANG=C.UTF-8
    SHELL=/bin/bash
    RVOIP_PERF_PROFILE_BUILD_ONLY=0
  )
  # Preserve only the pinned toolchain and compiler-cache plumbing supplied by
  # the release worker. Workload, compiler-profile, allocator, and performance
  # overrides remain absent so the canonical runner can enforce its clean
  # recipe. None of these allow-listed values contains a registry credential.
  for name in \
    CARGO_HOME RUSTUP_HOME RUSTC_WRAPPER \
    SCCACHE_BASEDIRS SCCACHE_CACHE_SIZE SCCACHE_DIR SCCACHE_GCS_BUCKET \
    SCCACHE_GCS_KEY_PREFIX SCCACHE_GCS_RW_MODE SCCACHE_GCS_SERVICE_ACCOUNT \
    SCCACHE_IDLE_TIMEOUT SCCACHE_MULTILEVEL_CHAIN \
    SCCACHE_MULTILEVEL_WRITE_ERROR_POLICY RVOIP_SCCACHE_ACTIVE \
    RVOIP_PERF_PREBUILT_MANIFEST RVOIP_RELEASE_CANDIDATE \
    RVOIP_RELEASE_ENVIRONMENT_ID \
    LD_LIBRARY_PATH LIBRARY_PATH PKG_CONFIG_PATH
  do
    if [[ -n "${!name:-}" ]]; then
      worker_env+=("${name}=${!name}")
    fi
  done
  env -i "${worker_env[@]}" \
    "${RUNNER}" clean 2>&1 | tee "${log}"
  run_dir="$(python3 - "${log}" <<'PY'
import json
import pathlib
import sys

prefix = "[perf-2k] mode=clean status=0 artifacts="
lines = pathlib.Path(sys.argv[1]).read_text(
    encoding="utf-8", errors="replace"
).splitlines()
matches = [line[len(prefix):] for line in lines if line.startswith(prefix)]
if len(matches) != 1:
    raise SystemExit(f"expected one canonical PASS trailer, found {len(matches)}")
path = pathlib.Path(matches[0]).resolve()
manifest = json.loads((path / "manifest.json").read_text(encoding="utf-8"))
if (manifest.get("mode"), manifest.get("status"), manifest.get("overall_status")) != (
    "clean", 0, "PASS"
):
    raise SystemExit(f"canonical run is not a clean PASS: {path}")
print(path)
PY
)"
  run_dirs+=(--run-dir "${run_dir}")
done

python3 "${EVIDENCE_TOOL}" import \
  --workspace-root "${WORKSPACE_ROOT}" \
  --beta-start "${OUTPUT}/beta-start.json" \
  --artifact-dir "${OUTPUT}" \
  "${run_dirs[@]}"

python3 "${EVIDENCE_TOOL}" verify-source \
  --workspace-root "${WORKSPACE_ROOT}" \
  --beta-start "${OUTPUT}/beta-start.json"
