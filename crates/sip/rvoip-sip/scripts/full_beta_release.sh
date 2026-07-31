#!/usr/bin/env bash
# One-command, fail-closed local SIP beta qualification run.
#
# This wrapper prepares the exact Homebrew Docker/Colima environment used by
# the local PBX lab, produces three canonical 2,000-CPS evidence runs, and then
# executes every release gate with packaged reporting. It qualifies a beta
# candidate; it does not publish crates to crates.io.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
WORKSPACE_ROOT="$(cd "$CRATE_DIR/../../.." && pwd)"
CANONICAL_RUNNER="$SCRIPT_DIR/perf_call_setup_2k_profile.sh"
CANONICAL_HELPER="$SCRIPT_DIR/canonical_2k_evidence.py"
BETA_GATE="$SCRIPT_DIR/beta_gate.sh"
API_CHECK="$SCRIPT_DIR/check_public_api.sh"
ASTERISK_DIR="${HOME:?HOME must identify the local account}/Developer/asterisk"
FREESWITCH_DIR="$HOME/Developer/freeswitch"

# This local release runner intentionally supports the Homebrew/Colima stack
# only. There is no Docker Desktop or alternate-path fallback.
HOMEBREW_PREFIX="/opt/homebrew"
COLIMA_BIN="$HOMEBREW_PREFIX/bin/colima"
DOCKER_BIN="$HOMEBREW_PREFIX/opt/docker/bin/docker"
DOCKER_COMPOSE_BIN="$HOMEBREW_PREFIX/opt/docker-compose/bin/docker-compose"
SIPP_BIN="$HOMEBREW_PREFIX/bin/sipp"
BARESIP_BIN="$HOMEBREW_PREFIX/bin/baresip"
BARESIP_MODULE_PATH="$HOMEBREW_PREFIX/lib/baresip/modules"
export PATH="$HOMEBREW_PREFIX/opt/docker/bin:$HOMEBREW_PREFIX/opt/docker-compose/bin:$HOMEBREW_PREFIX/bin:$HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
export DOCKER_CLI_PLUGIN_EXTRA_DIRS="$HOMEBREW_PREFIX/lib/docker/cli-plugins"

EXPECTED_RELEASE_VERSION="0.3.5"
EXPECTED_PUBLIC_API_VERSION="cargo-public-api 0.52.0"
EXPECTED_NIGHTLY_VERSION="rustc 1.97.0-nightly (e22c616e4 2026-04-19)"
COLIMA_CPUS=8
COLIMA_MEMORY_GIB=16
COLIMA_DISK_GIB=100

PREFLIGHT_ONLY=0
STRICT_UA_HOST_IP="${RVOIP_STRICT_UA_HOST_IP:-}"
CANONICAL_RUN_DIRS=()
LOCK_DIR="$WORKSPACE_ROOT/target/full-beta-release.lock"
LOCK_HELD=0
ORIGINAL_DOCKER_CONTEXT=""
DOCKER_CONTEXT_CHANGED=0
DRIVER_TIMESTAMP="$(date -u +%Y%m%dT%H%M%SZ)"
DRIVER_DIR="$WORKSPACE_ROOT/target/full-beta-release/$DRIVER_TIMESTAMP"

usage() {
  cat <<'EOF'
Usage: full_beta_release.sh [options]

With no options, this script:
  1. Requires a clean, committed rvoip source tree.
  2. Requires the exact local Homebrew toolchain and PBX lab directories.
  3. Starts or repairs an 8-CPU/16-GiB Colima Docker VM with a network address.
  4. Runs three fresh canonical 2,000-CPS clean evidence passes.
  5. Runs the full local/security/PBX/SIPp/baresip/perf/soak beta gate.
  6. Generates, verifies, and packages the beta reports.

Options:
  --preflight-only
      Validate and prepare the complete local stack, then stop before tests.
  --strict-ua-host-ip IPV4
      Use an explicit host-assigned IPv4. By default, en0 is required.
  --canonical-run-dir ABSOLUTE_PATH
      Reuse one existing canonical PASS directory. Supply exactly three times.
      The three runs are fully revalidated and sorted chronologically.
  --help, -h
      Show this help.

This is deliberately fail-closed: no Docker Desktop fallback, no missing-tool
skips, no external-gate skips, and no automatic report promotion/publication.
It may restart/reconfigure the default Colima profile to 8 CPUs, 16 GiB RAM,
100 GiB disk, Docker/VZ, and --network-address. The prior Docker context is
restored when this wrapper exits.
EOF
}

die() {
  printf 'full-beta: ERROR: %s\n' "$*" >&2
  exit 1
}

note() {
  printf '\nfull-beta: %s\n' "$*"
}

cleanup() {
  if [ "$DOCKER_CONTEXT_CHANGED" = "1" ] \
    && [ -n "$ORIGINAL_DOCKER_CONTEXT" ] \
    && [ -x "$DOCKER_BIN" ]; then
    "$DOCKER_BIN" context use "$ORIGINAL_DOCKER_CONTEXT" >/dev/null 2>&1 || true
  fi
  if [ "$LOCK_HELD" = "1" ] && [ -d "$LOCK_DIR" ]; then
    rmdir "$LOCK_DIR" 2>/dev/null || true
  fi
}

trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' HUP TERM

while [ "$#" -gt 0 ]; do
  case "$1" in
    --preflight-only)
      PREFLIGHT_ONLY=1
      shift
      ;;
    --strict-ua-host-ip)
      [ "$#" -ge 2 ] || {
        echo "--strict-ua-host-ip requires an IPv4 value" >&2
        exit 2
      }
      STRICT_UA_HOST_IP="$2"
      shift 2
      ;;
    --canonical-run-dir)
      [ "$#" -ge 2 ] || {
        echo "--canonical-run-dir requires an absolute path" >&2
        exit 2
      }
      case "$2" in
        /*) ;;
        *)
          echo "--canonical-run-dir requires an absolute path: $2" >&2
          exit 2
          ;;
      esac
      CANONICAL_RUN_DIRS+=("$2")
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [ "${#CANONICAL_RUN_DIRS[@]}" -ne 0 ] \
  && [ "${#CANONICAL_RUN_DIRS[@]}" -ne 3 ]; then
  die "--canonical-run-dir must be omitted or supplied exactly three times"
fi

require_executable() {
  local path="$1"
  local label="$2"
  [ -x "$path" ] || die "$label is missing or not executable: $path"
}

require_command() {
  local name="$1"
  command -v "$name" >/dev/null 2>&1 \
    || die "required command '$name' is not available on PATH"
}

require_clean_source() {
  local status
  status="$(git -C "$WORKSPACE_ROOT" status --porcelain=v1 --untracked-files=all)"
  if [ -n "$status" ]; then
    printf '%s\n' "$status" >&2
    die "rvoip must be clean before beta evidence is generated; commit this wrapper and all intended release changes first"
  fi
}

require_canonical_environment() {
  local name
  local found=0
  while IFS= read -r name; do
    case "$name" in
      RVOIP_STRICT_UA_HOST_IP)
        ;;
      BETA_*|RVOIP_*|PBX_*|SIPP_*|BARESIP_*|RUSTFLAGS|RUSTDOCFLAGS|RUSTUP_TOOLCHAIN|RUSTUP_HOME|CARGO_HOME|CARGO_*|MIMALLOC_*|DOCKER_HOST|DOCKER_CONTEXT|DOCKER_CONFIG|DOCKER_CERT_PATH|DOCKER_TLS_VERIFY|COLIMA_*|LIMA_*|XDG_CONFIG_HOME)
        printf 'full-beta: noncanonical environment override is set: %s\n' "$name" >&2
        found=1
        ;;
    esac
  done < <(compgen -e)
  [ "$found" = "0" ] \
    || die "clear the listed build/profile overrides before canonical evidence"
}

validate_host_ip() {
  local candidate="$1"
  python3 - "$candidate" <<'PY'
import ipaddress
import socket
import sys

try:
    address = ipaddress.IPv4Address(sys.argv[1])
except Exception as error:
    raise SystemExit(f"invalid strict-UA IPv4: {error}")
if address.is_loopback or address.is_unspecified or address.is_multicast:
    raise SystemExit(f"strict-UA IPv4 is not usable: {address}")
sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
try:
    sock.bind((str(address), 0))
except OSError as error:
    raise SystemExit(
        f"strict-UA IPv4 {address} is not assigned/bindable on this host: {error}"
    )
finally:
    sock.close()
print(address)
PY
}

detect_strict_ua_host_ip() {
  local candidate="$STRICT_UA_HOST_IP"
  if [ -z "$candidate" ]; then
    candidate="$(/usr/sbin/ipconfig getifaddr en0 2>/dev/null)" \
      || die "en0 has no IPv4 address; pass --strict-ua-host-ip explicitly"
  fi
  STRICT_UA_HOST_IP="$(validate_host_ip "$candidate")" \
    || die "strict-UA host address validation failed"
}

colima_profile_line() {
  "$COLIMA_BIN" list 2>/dev/null | awk '$1 == "default" { print; exit }'
}

colima_profile_is_release_ready() {
  "$COLIMA_BIN" status --json 2>/dev/null | python3 -c '
import ipaddress
import json
import sys

value = json.load(sys.stdin)
try:
    address = ipaddress.IPv4Address(value.get("ip_address", ""))
except Exception:
    raise SystemExit(1)
ready = (
    value.get("driver") == "macOS Virtualization.Framework"
    and value.get("arch") == "aarch64"
    and value.get("runtime") == "docker"
    and value.get("kubernetes") is False
    and int(value.get("cpu", 0)) >= int(sys.argv[1])
    and int(value.get("memory", 0)) >= int(sys.argv[2]) * 1024**3
    and int(value.get("disk", 0)) >= int(sys.argv[3]) * 1024**3
    and not address.is_loopback
    and not address.is_unspecified
)
raise SystemExit(0 if ready else 1)
' "$COLIMA_CPUS" "$COLIMA_MEMORY_GIB" "$COLIMA_DISK_GIB"
}

ensure_colima_docker() {
  local colima_socket
  local compose_plugin_path
  local context_socket

  ORIGINAL_DOCKER_CONTEXT="$("$DOCKER_BIN" context show 2>/dev/null)" \
    || die "could not read the active Docker context"
  if [ "$ORIGINAL_DOCKER_CONTEXT" != "colima" ]; then
    DOCKER_CONTEXT_CHANGED=1
  fi
  if ! colima_profile_is_release_ready; then
    if "$COLIMA_BIN" status >/dev/null 2>&1; then
      note "restarting Colima with the required Docker resources and reachable VM address"
      "$COLIMA_BIN" stop
    else
      note "starting Colima with the required Docker resources and reachable VM address"
    fi
    "$COLIMA_BIN" start \
      --activate=false \
      --arch aarch64 \
      --vm-type vz \
      --runtime docker \
      --cpus "$COLIMA_CPUS" \
      --memory "$COLIMA_MEMORY_GIB" \
      --disk "$COLIMA_DISK_GIB" \
      --network-address
  fi

  colima_profile_is_release_ready \
    || die "Colima default profile is not release-ready after startup"

  if [ -n "${DOCKER_HOST:-}" ]; then
    die "DOCKER_HOST is set and would bypass the required Colima context; unset it"
  fi
  if [ "$("$DOCKER_BIN" context show 2>/dev/null)" != "colima" ]; then
    "$DOCKER_BIN" context use colima >/dev/null
  fi
  [ "$("$DOCKER_BIN" context show 2>/dev/null)" = "colima" ] \
    || die "Docker context is not 'colima'"
  colima_socket="$("$COLIMA_BIN" status --json | python3 -c \
    'import json,sys; print(json.load(sys.stdin)["docker_socket"])')"
  context_socket="$("$DOCKER_BIN" context inspect colima \
    --format '{{ (index .Endpoints "docker").Host }}')"
  [ "$context_socket" = "$colima_socket" ] \
    || die "Docker context 'colima' does not point at the active Colima socket"
  "$DOCKER_BIN" info >/dev/null \
    || die "Docker engine is not reachable through Colima"
  "$DOCKER_BIN" compose version >/dev/null \
    || die "the Docker Compose v2 plugin is unavailable"
  "$DOCKER_COMPOSE_BIN" version >/dev/null \
    || die "the exact Homebrew docker-compose binary is unavailable"
  compose_plugin_path="$("$DOCKER_BIN" info --format \
    '{{range .ClientInfo.Plugins}}{{if eq .Name "compose"}}{{.Path}}{{end}}{{end}}')"
  [ "$compose_plugin_path" = "$HOMEBREW_PREFIX/lib/docker/cli-plugins/docker-compose" ] \
    || die "Docker is not using the exact Homebrew Compose plugin"
  python3 - "$compose_plugin_path" "$DOCKER_COMPOSE_BIN" <<'PY'
import pathlib
import sys

plugin = pathlib.Path(sys.argv[1]).resolve()
binary = pathlib.Path(sys.argv[2]).resolve()
if plugin != binary:
    raise SystemExit(f"Compose plugin mismatch: {plugin} != {binary}")
PY
}

validate_pbx_stack() {
  local module
  for module in g711.so auconv.so auresamp.so aufile.so ausine.so uuid.so account.so menu.so; do
    [ -f "$BARESIP_MODULE_PATH/$module" ] \
      || die "required baresip module is missing: $BARESIP_MODULE_PATH/$module"
  done

  for path in \
    "$ASTERISK_DIR/scripts/up.sh" \
    "$ASTERISK_DIR/scripts/down.sh" \
    "$FREESWITCH_DIR/scripts/up.sh" \
    "$FREESWITCH_DIR/scripts/down.sh"; do
    require_executable "$path" "PBX lifecycle script"
  done
  [ -f "$ASTERISK_DIR/docker-compose.yml" ] \
    || die "Asterisk compose file is missing"
  [ -f "$FREESWITCH_DIR/docker-compose.yml" ] \
    || die "FreeSWITCH compose file is missing"
  [ -f "$FREESWITCH_DIR/.env.example" ] \
    || die "FreeSWITCH .env.example is missing"

  "$DOCKER_BIN" compose \
    -f "$ASTERISK_DIR/docker-compose.yml" config >/dev/null
  "$DOCKER_BIN" compose \
    --env-file "$FREESWITCH_DIR/.env.example" \
    -f "$FREESWITCH_DIR/docker-compose.yml" config >/dev/null
}

run_preflight() {
  local actual
  local git_root
  local workspace_version

  [ "$(uname -s)" = "Darwin" ] \
    || die "this local Colima/PBX release runner requires macOS"
  [ "$(uname -m)" = "arm64" ] \
    || die "this local PBX release runner requires Apple Silicon (arm64)"

  require_executable "$COLIMA_BIN" "Homebrew Colima"
  require_executable "$DOCKER_BIN" "Homebrew Docker CLI"
  require_executable "$DOCKER_COMPOSE_BIN" "Homebrew Docker Compose"
  require_executable "$SIPP_BIN" "Homebrew SIPp"
  require_executable "$BARESIP_BIN" "Homebrew baresip"
  require_executable "$CANONICAL_RUNNER" "canonical 2K runner"
  require_executable "$BETA_GATE" "beta gate"
  require_executable "$API_CHECK" "public API checker"
  [ -f "$CANONICAL_HELPER" ] || die "canonical evidence helper is missing"

  for command_name in \
    cargo rustc rustup python3 git rg tar openssl \
    cargo-public-api cargo-semver-checks cargo-audit cargo-fuzz; do
    require_command "$command_name"
  done
  python3 -c 'import sys; raise SystemExit(0 if sys.version_info >= (3, 11) else 1)' \
    || die "Python 3.11 or newer is required"
  workspace_version="$(python3 - "$WORKSPACE_ROOT/Cargo.toml" <<'PY'
import pathlib
import sys
import tomllib

manifest = tomllib.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
print(manifest["workspace"]["package"]["version"])
PY
)"
  [ "$workspace_version" = "$EXPECTED_RELEASE_VERSION" ] \
    || die "this wrapper qualifies $EXPECTED_RELEASE_VERSION, but the workspace is $workspace_version"
  cargo fmt --version >/dev/null \
    || die "rustfmt is unavailable for the active Rust toolchain"
  cargo +nightly fuzz --version >/dev/null \
    || die "cargo-fuzz or the nightly Rust toolchain is unavailable"

  actual="$(cargo public-api --version)"
  [ "$actual" = "$EXPECTED_PUBLIC_API_VERSION" ] \
    || die "cargo-public-api mismatch: expected '$EXPECTED_PUBLIC_API_VERSION', found '$actual'"
  actual="$(rustc +nightly --version)"
  [ "$actual" = "$EXPECTED_NIGHTLY_VERSION" ] \
    || die "nightly mismatch: expected '$EXPECTED_NIGHTLY_VERSION', found '$actual'"
  cargo semver-checks --version >/dev/null \
    || die "cargo-semver-checks is unavailable"
  cargo audit --version >/dev/null \
    || die "cargo-audit is unavailable"

  git_root="$(git -C "$WORKSPACE_ROOT" rev-parse --show-toplevel)"
  [ "$(cd "$git_root" && pwd -P)" = "$WORKSPACE_ROOT" ] \
    || die "script did not resolve the rvoip Git root correctly"
  git -C "$WORKSPACE_ROOT" cat-file -e \
    0df3e5ba7b29ce4dc0c641b36381aefcd4b66925^{commit} \
    || die "the required public API baseline commit is unavailable"

  require_canonical_environment
  detect_strict_ua_host_ip
  ensure_colima_docker
  validate_pbx_stack
  require_clean_source

  note "preflight PASS"
  printf '  Docker CLI: %s\n' "$DOCKER_BIN"
  printf '  Docker context: %s\n' "$("$DOCKER_BIN" context show)"
  printf '  Colima: %s\n' "$(colima_profile_line)"
  printf '  strict-UA host IP: %s\n' "$STRICT_UA_HOST_IP"
  printf '  Asterisk: %s\n' "$ASTERISK_DIR"
  printf '  FreeSWITCH: %s\n' "$FREESWITCH_DIR"
}

extract_canonical_run_dir() {
  local log="$1"
  python3 - "$log" <<'PY'
import json
import pathlib
import sys

prefix = "[perf-2k] mode=clean status=0 artifacts="
lines = pathlib.Path(sys.argv[1]).read_text(
    encoding="utf-8", errors="replace"
).splitlines()
matches = [line[len(prefix):] for line in lines if line.startswith(prefix)]
if len(matches) != 1:
    raise SystemExit(f"expected exactly one canonical success trailer, found {len(matches)}")
run_dir = pathlib.Path(matches[0])
if not run_dir.is_absolute() or not run_dir.is_dir():
    raise SystemExit(f"invalid canonical artifact directory: {run_dir}")
manifest = json.loads((run_dir / "manifest.json").read_text(encoding="utf-8"))
actual = (
    manifest.get("mode"),
    manifest.get("status"),
    manifest.get("overall_status"),
)
if actual != ("clean", 0, "PASS"):
    raise SystemExit(f"canonical manifest is not PASS: {run_dir} ({actual!r})")
print(run_dir.resolve())
PY
}

run_canonical_passes() {
  local pass
  local log
  local status
  local run_dir

  mkdir -p "$DRIVER_DIR"
  for pass in 1 2 3; do
    log="$DRIVER_DIR/canonical-2k-pass-$pass.log"
    note "canonical 2K clean pass $pass of 3"
    set +e
    env -i \
      HOME="$HOME" \
      USER="${USER:-}" \
      LOGNAME="${LOGNAME:-${USER:-}}" \
      PATH="$PATH" \
      TMPDIR=/tmp \
      LANG=en_US.UTF-8 \
      SHELL=/bin/bash \
      RVOIP_PERF_PROFILE_BUILD_ONLY=0 \
      "$CANONICAL_RUNNER" clean 2>&1 | tee "$log"
    status=${PIPESTATUS[0]}
    set -e
    [ "$status" -eq 0 ] \
      || die "canonical 2K pass $pass failed; see $log"
    run_dir="$(extract_canonical_run_dir "$log")" \
      || die "canonical 2K pass $pass did not produce valid evidence; see $log"
    CANONICAL_RUN_DIRS+=("$run_dir")
  done
}

validate_and_order_canonical_runs() {
  python3 - "$WORKSPACE_ROOT" "$CANONICAL_HELPER" "${CANONICAL_RUN_DIRS[@]}" <<'PY'
import importlib.util
import pathlib
import sys

workspace = pathlib.Path(sys.argv[1]).resolve()
helper_path = pathlib.Path(sys.argv[2]).resolve()
raw_paths = sys.argv[3:]
if len(raw_paths) != 3:
    raise SystemExit("exactly three canonical run directories are required")

spec = importlib.util.spec_from_file_location("rvoip_canonical_2k", helper_path)
helper = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helper)

paths = [pathlib.Path(value).resolve() for value in raw_paths]
if len(set(paths)) != 3:
    raise SystemExit("canonical run directories must be distinct")
for path in paths:
    if any(character in str(path) for character in ":\n\r"):
        raise SystemExit(f"canonical path cannot be colon-delimited safely: {path}")

source = helper.capture_source_provenance(workspace)
if source.get("git_dirty") is not False:
    raise SystemExit("release source is no longer clean")
fingerprint = source.get("source_fingerprint_sha256")
if not helper.valid_fingerprint(fingerprint):
    raise SystemExit("current source fingerprint is unavailable")

try:
    runs = [helper.validate_run(path, fingerprint) for path in paths]
except helper.EvidenceError as error:
    raise SystemExit(f"canonical run validation failed: {error}")
runs.sort(key=lambda item: item["captured_at"])
if any(
    left["captured_at"] >= right["captured_at"]
    for left, right in zip(runs, runs[1:])
):
    raise SystemExit("canonical timestamps are not strictly chronological")
if len({item["executable_sha256"] for item in runs}) != 1:
    raise SystemExit("canonical runs do not share one byte-identical executable")

print(":".join(str(item["run_dir"]) for item in runs))
PY
}

run_full_beta_gate() {
  local canonical_run_dirs="$1"

  note "launching the complete beta gate"
  printf '  canonical evidence: %s\n' "$canonical_run_dirs"
  printf '  wrapper logs: %s\n' "$DRIVER_DIR"

  cd "$WORKSPACE_ROOT"
  env -i \
    HOME="$HOME" \
    USER="${USER:-}" \
    LOGNAME="${LOGNAME:-${USER:-}}" \
    PATH="$PATH" \
    TMPDIR=/tmp \
    LANG=en_US.UTF-8 \
    SHELL=/bin/bash \
    DOCKER_CLI_PLUGIN_EXTRA_DIRS="$DOCKER_CLI_PLUGIN_EXTRA_DIRS" \
    RVOIP_STRICT_UA_HOST_IP="$STRICT_UA_HOST_IP" \
    RVOIP_REQUIRE_API_TOOLS=1 \
    BARESIP_BIN="$BARESIP_BIN" \
    BARESIP_MODULE_PATH="$BARESIP_MODULE_PATH" \
    SIPP_BIN="$SIPP_BIN" \
    BETA_ASTERISK_DIR="$ASTERISK_DIR" \
    BETA_FREESWITCH_DIR="$FREESWITCH_DIR" \
    BETA_REPORT_PACKAGE=1 \
    BETA_DENY_WARNINGS=1 \
    BETA_TEST_LOG_FILTER=off \
    BETA_REQUIRE_CLEAN_SOURCE=1 \
    BETA_REQUIRE_CANONICAL_2K_EVIDENCE=1 \
    BETA_CANONICAL_2K_RUN_DIRS="$canonical_run_dirs" \
    BETA_CAPTURE_DOCKER_LOGS=1 \
    BETA_RUN_LOCAL_PBX=1 \
    BETA_RESTORE_LOCAL_PBX=1 \
    BETA_PBX_PROVIDER=both \
    BETA_PBX_API=all \
    BETA_PBX_SCENARIO=all \
    BETA_PBX_G729_PROFILES="g729a g729ab" \
    BETA_RUN_SIPP=1 \
    BETA_SIPP_CPS="30 100 300 1000 2000" \
    BETA_SIPP_DIAGNOSTICS=0 \
    BETA_RUN_STRICT_UA=1 \
    BETA_RUN_FUZZ_SMOKE=1 \
    BETA_FUZZ_TOOLCHAIN=nightly \
    BETA_FUZZ_SMOKE_RUNS=1000 \
    BETA_FUZZ_SMOKE_SECONDS=10 \
    BETA_RUN_PERF_ALL=1 \
    BETA_PERF_REGRESSION_FAIL=1 \
    BETA_PERF_REGRESSION_TOLERANCE_PCT=15 \
    BETA_PERF_LATENCY_TOLERANCE_PCT=25 \
    BETA_PERF_PROFILE_MATRIX="endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000" \
    BETA_PERF_REGRESSION_BASELINE_ROOT=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z \
    BETA_PERF_REGRESSION_BASELINE_MANIFEST=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json \
    BETA_RUN_BURST_SMOKE=1 \
    BETA_RUN_BURST_MATRIX=1 \
    BETA_BURST_MATRIX=all \
    BETA_RUN_LONG_SOAK=1 \
    BETA_PERF_MEDIA_CHURN_DURATION_SECS=120 \
    BETA_PERF_MEDIA_CHURN_ACTIVE_CALLS=30 \
    BETA_PERF_MONOLITHIC_SOAK_DURATION_SECS=3600 \
    BETA_PERF_MONOLITHIC_SOAK_ACTIVE_CALLS=30 \
    RVOIP_PERF_MIN_SUCCESS_PCT=99.9 \
    RVOIP_PERF_SOAK_DURATION_SECS=3600 \
    RVOIP_PERF_SOAK_ACTIVE_CALLS=500 \
    RVOIP_PERF_SOAK_MIN_HOLD_SECS=10 \
    RVOIP_PERF_SOAK_MAX_HOLD_SECS=360 \
    RVOIP_PERF_SOAK_CPS=0 \
    RVOIP_PERF_SOAK_DRAIN_CPS=10 \
    RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS=160 \
    RVOIP_PERF_MASS_TEARDOWN_CALLS=500 \
    RVOIP_PERF_MASS_TEARDOWN_SETUP_CPS=30 \
    RVOIP_PERF_SKIP_AUDIO_FRAME_DELIVERY=0 \
    RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR=15 \
    RVOIP_PERF_RSS_TAIL_WINDOW_SECS=60 \
    RVOIP_PERF_EXTERNAL_RESOURCE_SAMPLER=1 \
    "$BETA_GATE" --full --require-external
}

require_clean_source
mkdir -p "$WORKSPACE_ROOT/target"
if ! mkdir "$LOCK_DIR" 2>/dev/null; then
  die "another full beta release appears active (lock: $LOCK_DIR)"
fi
LOCK_HELD=1

run_preflight
if [ "$PREFLIGHT_ONLY" = "1" ]; then
  exit 0
fi

mkdir -p "$DRIVER_DIR"
if [ "${#CANONICAL_RUN_DIRS[@]}" -eq 0 ]; then
  run_canonical_passes
fi
require_clean_source
CANONICAL_RUN_DIRS_JOINED="$(validate_and_order_canonical_runs)" \
  || die "the three canonical runs failed strict prevalidation"
run_full_beta_gate "$CANONICAL_RUN_DIRS_JOINED"
