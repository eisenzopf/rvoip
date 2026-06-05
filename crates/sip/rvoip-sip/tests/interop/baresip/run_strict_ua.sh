#!/usr/bin/env bash
# baresip strict-UA interop gate.
#
# The default path starts a local rvoip-sip perf_listener and drives one
# standards-shaped baresip UAC call through INVITE, 200 OK, ACK, media start,
# BYE, and 200 OK. A caller can instead point the gate at an existing target
# with RVOIP_STRICT_UA_TARGET_HOST/RVOIP_STRICT_UA_TARGET_PORT.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/../../../../../.." && pwd)"
OUT_ROOT="${RVOIP_STRICT_UA_RESULTS:-$WORKSPACE_ROOT/target/strict-ua/$(date -u +%Y%m%dT%H%M%SZ)}"
SUMMARY="$OUT_ROOT/summary.md"
ENV_REPORT="$OUT_ROOT/environment.md"
MATRIX="$OUT_ROOT/matrix.tsv"
BARESIP_BIN="${BARESIP_BIN:-baresip}"
if [ -z "${BARESIP_MODULE_PATH:-}" ] && command -v brew >/dev/null 2>&1; then
  if prefix="$(brew --prefix baresip 2>/dev/null)"; then
    BARESIP_MODULE_PATH="$prefix/lib/baresip/modules"
  fi
fi
BARESIP_MODULE_PATH="${BARESIP_MODULE_PATH:-/opt/homebrew/lib/baresip/modules}"
CALL_SECONDS="${RVOIP_STRICT_UA_CALL_SECONDS:-8}"
TARGET_PORT="${RVOIP_STRICT_UA_TARGET_PORT:-35160}"
TARGET_HOST="${RVOIP_STRICT_UA_TARGET_HOST:-}"
MANAGED_TARGET=0
LISTENER_PID=""

mkdir -p "$OUT_ROOT"

redacted_env() {
  env | LC_ALL=C sort | awk -F= '
    /^(RVOIP_|BETA_|BARESIP_|SIP_)/ {
      key=$1
      value=substr($0, length($1) + 2)
      upper=toupper(key)
      if (upper ~ /(PASSWORD|PASS|SECRET|TOKEN|CREDENTIAL|PRIVATE|AUTHORIZATION)/) {
        print key"=<redacted>"
      } else {
        print key"="value
      }
    }
  '
}

capture_command() {
  local output="$1"
  shift
  {
    echo "+ $*"
    "$@"
  } >"$output" 2>&1 || true
}

detect_host_ipv4() {
  if [ -n "${RVOIP_STRICT_UA_HOST_IP:-}" ]; then
    printf '%s\n' "$RVOIP_STRICT_UA_HOST_IP"
    return
  fi
  if command -v ipconfig >/dev/null 2>&1; then
    local iface
    for iface in en0 bridge100; do
      if ipconfig getifaddr "$iface" >/dev/null 2>&1; then
        ipconfig getifaddr "$iface"
        return
      fi
    done
  fi
  if command -v hostname >/dev/null 2>&1; then
    hostname -I 2>/dev/null | awk '{print $1; exit}' || true
  fi
}

write_environment_report() {
  {
    echo "# baresip Strict-UA Environment"
    echo
    echo "- started_at_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "- workspace: $WORKSPACE_ROOT"
    echo "- output_root: $OUT_ROOT"
    echo "- target: $TARGET_HOST:$TARGET_PORT"
    echo "- managed_target: $MANAGED_TARGET"
    echo "- call_seconds: $CALL_SECONDS"
    echo "- baresip: $("$BARESIP_BIN" -h 2>&1 | head -1 || echo unknown)"
    echo "- rustc: $(rustc --version 2>/dev/null || echo unknown)"
    echo "- cargo: $(cargo --version 2>/dev/null || echo unknown)"
    echo "- host: $(uname -a 2>/dev/null || echo unknown)"
    echo
    echo "## Redacted Runtime Environment"
    echo
    echo '```text'
    redacted_env
    echo '```'
  } >"$ENV_REPORT"
  capture_command "$OUT_ROOT/git-status.txt" git -C "$WORKSPACE_ROOT" status --short
}

cleanup() {
  if [ -n "$LISTENER_PID" ] && kill -0 "$LISTENER_PID" >/dev/null 2>&1; then
    kill -INT "$LISTENER_PID" >/dev/null 2>&1 || true
    wait "$LISTENER_PID" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

require_baresip() {
  if ! command -v "$BARESIP_BIN" >/dev/null 2>&1; then
    echo "baresip binary '$BARESIP_BIN' not found on PATH." >&2
    exit 1
  fi
}

start_managed_target() {
  local listener_log="$OUT_ROOT/rvoip_perf_listener.log"
  echo "[strict-ua] building rvoip-sip perf_listener"
  cargo build -p rvoip-sip --release --example perf_listener >"$OUT_ROOT/perf_listener_build.log" 2>&1
  echo "[strict-ua] starting rvoip-sip perf_listener on $TARGET_HOST:$TARGET_PORT"
  "$WORKSPACE_ROOT/target/release/examples/perf_listener" \
    "$TARGET_PORT" "$TARGET_HOST" --diagnostics >"$listener_log" 2>&1 &
  LISTENER_PID=$!
  for _ in $(seq 1 100); do
    if grep -q 'listening on' "$listener_log" 2>/dev/null; then
      return
    fi
    if ! kill -0 "$LISTENER_PID" >/dev/null 2>&1; then
      echo "rvoip-sip perf_listener exited before listening. See $listener_log" >&2
      exit 1
    fi
    sleep 0.1
  done
  echo "rvoip-sip perf_listener did not become ready. See $listener_log" >&2
  exit 1
}

write_baresip_config() {
  local cfg="$OUT_ROOT/baresip"
  mkdir -p "$cfg"
  cat >"$cfg/config" <<EOF
sip_listen 0.0.0.0:0
sip_transports udp
sip_trans_def udp
call_local_timeout 10
call_max_calls 2
call_accept no
audio_player aufile,$OUT_ROOT/baresip_rx.wav
audio_source ausine,440
module_path $BARESIP_MODULE_PATH
module g711.so
module auconv.so
module auresamp.so
module aufile.so
module ausine.so
module uuid.so
module_app account.so
module_app menu.so
EOF
  cat >"$cfg/accounts" <<EOF
"Baresip Strict UA" <sip:baresip@$TARGET_HOST>;regint=0;audio_codecs=PCMU/8000/1,PCMA/8000/1
EOF
  : >"$cfg/contacts"
}

run_baresip_call() {
  local cfg="$OUT_ROOT/baresip"
  local log="$OUT_ROOT/baresip_uac.log"
  local target_uri="sip:rvoip@$TARGET_HOST:$TARGET_PORT"
  echo "[strict-ua] dialing $target_uri with baresip"
  set +e
  "$BARESIP_BIN" -f "$cfg" -4 -c -s -t "$CALL_SECONDS" -e "/dial $target_uri" >"$log" 2>&1
  local rc=$?
  set -e
  echo "$rc" >"$OUT_ROOT/baresip_exit_status.txt"
}

assert_log_contains() {
  local file="$1"
  local pattern="$2"
  local label="$3"
  if grep -Eq "$pattern" "$file"; then
    printf 'PASS\t%s\t%s\t%s\n' "$label" "$file" "$pattern" >>"$MATRIX"
  else
    printf 'FAIL\t%s\t%s\t%s\n' "$label" "$file" "$pattern" >>"$MATRIX"
  fi
}

analyze_results() {
  printf 'status\tcheck\tlog\tpattern\n' >"$MATRIX"
  assert_log_contains "$OUT_ROOT/baresip_uac.log" 'INVITE sip:rvoip@' 'baresip sent INVITE'
  assert_log_contains "$OUT_ROOT/baresip_uac.log" 'SIP/2.0 200 OK' 'baresip received 200 OK'
  assert_log_contains "$OUT_ROOT/baresip_uac.log" 'ACK sip:rvoip-perf-listener@|ACK sip:rvoip@' 'baresip sent ACK'
  assert_log_contains "$OUT_ROOT/baresip_uac.log" 'Call established:' 'baresip established call'
  assert_log_contains "$OUT_ROOT/baresip_uac.log" 'BYE sip:rvoip-perf-listener@|BYE sip:rvoip@' 'baresip sent BYE'
  assert_log_contains "$OUT_ROOT/baresip_uac.log" 'CSeq: [0-9]+ BYE' 'BYE transaction present'
  if [ "$MANAGED_TARGET" = "1" ]; then
    assert_log_contains "$OUT_ROOT/rvoip_perf_listener.log" 'accepted_total=1|final accepted_total=1' 'rvoip accepted strict-UA call'
  fi

  local failures
  failures="$(awk -F '\t' 'NR > 1 && $1 != "PASS" { n++ } END { print n + 0 }' "$MATRIX")"
  {
    echo "# baresip Strict-UA Summary"
    echo
    echo "- ended_at_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "- environment: \`environment.md\`"
    echo "- matrix: \`matrix.tsv\`"
    echo "- baresip_log: \`baresip_uac.log\`"
    if [ "$MANAGED_TARGET" = "1" ]; then
      echo "- rvoip_listener_log: \`rvoip_perf_listener.log\`"
    fi
    echo
    echo "## Result"
    echo
    echo "- failures: $failures"
    echo
    echo "## Checks"
    echo
    echo "| Status | Check | Log | Pattern |"
    echo "|--------|-------|-----|---------|"
    awk -F '\t' 'NR > 1 {
      printf "| %s | %s | `%s` | `%s` |\n", $1, $2, $3, $4
    }' "$MATRIX"
  } >"$SUMMARY"

  if [ "$failures" -ne 0 ]; then
    exit 1
  fi
}

require_baresip
if [ -z "$TARGET_HOST" ]; then
  TARGET_HOST="$(detect_host_ipv4)"
  MANAGED_TARGET=1
fi
if [ -z "$TARGET_HOST" ] || [ "$TARGET_HOST" = "127.0.0.1" ]; then
  echo "Could not determine a non-loopback IPv4 address for baresip. Set RVOIP_STRICT_UA_HOST_IP." >&2
  exit 1
fi

write_environment_report
if [ "$MANAGED_TARGET" = "1" ]; then
  start_managed_target
fi
write_baresip_config
run_baresip_call
if [ "$MANAGED_TARGET" = "1" ]; then
  cleanup
  LISTENER_PID=""
fi
analyze_results
echo "[strict-ua] PASS. Summary: $SUMMARY"
