#!/usr/bin/env bash
# rvoip vs Asterisk performance comparison using sipp as the common driver.
#
# - sipp drives a custom UAC scenario (uac_perf.xml) against both
#   targets at 30 / 100 / 300 CPS.
# - Targets share the same wire-level test shape (INVITE → 200 → ACK
#   → 100 ms hold → BYE → 200), so the differences captured reflect
#   the SUT's signalling path, not the driver.
# - Each run writes a per-CPS stat CSV + the screen log to RESULTS_DIR;
#   the aggregator at the end emits a markdown comparison table.
#
# Usage:
#   ./run_comparison.sh [TARGET_HOST] [TARGET_PORT] [TAG]
#
# The script is meant to be invoked twice — once per target — and the
# resulting CSVs live side-by-side under RESULTS_DIR for the aggregator.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/../../../../.." && pwd)"
SCENARIO="$SCRIPT_DIR/uac_perf.xml"
RESULTS_DIR="${RVOIP_PERF_RESULTS:-$SCRIPT_DIR/results}"
mkdir -p "$RESULTS_DIR"
RUN_STARTED_UTC="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
RUN_STARTED_EPOCH="$(date +%s)"
RUN_ENV="$RESULTS_DIR/environment.md"
RUN_MATRIX="$RESULTS_DIR/runs.tsv"
RUN_SUMMARY="$RESULTS_DIR/run_summary.md"
ANALYSIS_MD="$RESULTS_DIR/analysis.md"

TARGET_HOST="${1:-127.0.0.1}"
TARGET_PORT="${2:-5060}"
TAG="${3:-target}"
SIPP_BIN="${SIPP_BIN:-sipp}"
STEADY_SECS="${RVOIP_PERF_STEADY_SECS:-15}"
# CPS levels: doc says 30/100/300; can be overridden for quick smoke.
CPS_LEVELS="${RVOIP_PERF_CPS:-30 100 300}"
MIN_SUCCESS_PCT="${RVOIP_PERF_MIN_SUCCESS_PCT:-99.9}"
RUN_FAILURES=0

redacted_env() {
  env | LC_ALL=C sort | awk -F= '
    /^(RVOIP_|SIPP_|BETA_|PBX_|SIP_)/ {
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

sipp_version() {
  "$SIPP_BIN" -v 2>&1 | awk 'NF { print; found=1; exit } END { if (!found) print "unknown" }'
}

write_environment_report() {
  {
    echo "# SIPp Performance Environment"
    echo
    echo "- started_at_utc: $RUN_STARTED_UTC"
    echo "- results_dir: $RESULTS_DIR"
    echo "- target: $TARGET_HOST:$TARGET_PORT"
    echo "- tag: $TAG"
    echo "- steady_secs: $STEADY_SECS"
    echo "- cps_levels: $CPS_LEVELS"
    echo "- min_success_pct: $MIN_SUCCESS_PCT"
    echo "- scenario: $SCENARIO"
    echo "- workspace: $WORKSPACE_ROOT"
    echo "- workspace_git: $(git -C "$WORKSPACE_ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
    echo "- rustc: $(rustc --version 2>/dev/null || echo unknown)"
    echo "- cargo: $(cargo --version 2>/dev/null || echo unknown)"
    echo "- host: $(uname -a 2>/dev/null || echo unknown)"
    echo "- driver: local"
    echo "- sipp: $(sipp_version)"
    if command -v tshark >/dev/null 2>&1; then
      echo "- tshark: $(tshark -v 2>&1 | head -1)"
    else
      echo "- tshark: not found"
    fi
    echo
    echo "## Redacted Runtime Environment"
    echo
    echo '```text'
    redacted_env
    echo '```'
  } >"$RUN_ENV"
  capture_command "$RESULTS_DIR/git-status.txt" git -C "$WORKSPACE_ROOT" status --short
}

init_report() {
  printf 'status\ttag\ttarget_host\ttarget_port\tcps\tcalls\tduration_s\tsipp_rc\tstat_file\tscreen_log\terror_log\n' >"$RUN_MATRIX"
  write_environment_report
}

record_run() {
  local status="$1"
  local cps="$2"
  local calls="$3"
  local duration="$4"
  local rc="$5"
  local stat_file="$6"
  local screen="$7"
  local errlog="$8"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$status" "$TAG" "$TARGET_HOST" "$TARGET_PORT" "$cps" "$calls" "$duration" "$rc" \
    "$stat_file" "$screen" "$errlog" >>"$RUN_MATRIX"
}

write_run_summary() {
  local exit_status="$1"
  local ended_at
  local duration
  local pass_count
  local fail_count
  local total_count
  ended_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  duration="$(( $(date +%s) - RUN_STARTED_EPOCH ))"
  pass_count="$(awk -F '\t' 'NR > 1 && $1 == "PASS" { n++ } END { print n + 0 }' "$RUN_MATRIX" 2>/dev/null || echo 0)"
  fail_count="$(awk -F '\t' 'NR > 1 && $1 != "PASS" { n++ } END { print n + 0 }' "$RUN_MATRIX" 2>/dev/null || echo 0)"
  total_count="$(awk 'NR > 1 { n++ } END { print n + 0 }' "$RUN_MATRIX" 2>/dev/null || echo 0)"
  {
    echo "# SIPp Run Summary"
    echo
    echo "- started_at_utc: $RUN_STARTED_UTC"
    echo "- ended_at_utc: $ended_at"
    echo "- duration_seconds: $duration"
    echo "- exit_status: $exit_status"
    echo "- environment: \`environment.md\`"
    echo "- matrix: \`runs.tsv\`"
    echo "- parsed_analysis: \`analysis.md\`"
    echo
    echo "## Result"
    echo
    echo "- total_cps_points: $total_count"
    echo "- pass_points: $pass_count"
    echo "- failed_points: $fail_count"
    echo
    echo "## Runs"
    echo
    echo "| Status | Tag | Target | CPS | Calls | Duration | SIPp rc | Stat | Screen | Errors |"
    echo "|--------|-----|--------|-----|-------|----------|---------|------|--------|--------|"
    awk -F '\t' 'NR > 1 {
      printf "| %s | %s | %s:%s | %s | %s | %ss | %s | `%s` | `%s` | `%s` |\n", $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11
    }' "$RUN_MATRIX"
  } >"$RUN_SUMMARY"
}

if ! command -v "$SIPP_BIN" >/dev/null 2>&1; then
  echo "sipp not found on PATH (looked for '$SIPP_BIN'). brew install sipp." >&2
  exit 1
fi
if ! [[ -f "$SCENARIO" ]]; then
  echo "scenario not found: $SCENARIO" >&2
  exit 1
fi

init_report
echo "[run_comparison] target=$TARGET_HOST:$TARGET_PORT tag=$TAG steady=${STEADY_SECS}s cps=$CPS_LEVELS"

evaluate_stat_csv() {
  local stat_file="$1"
  local expected_calls="$2"
  local min_success_pct="$3"
  python3 - "$stat_file" "$expected_calls" "$min_success_pct" <<'PY'
import csv
import sys

path, expected_raw, threshold_raw = sys.argv[1:4]
expected = int(expected_raw)
threshold = float(threshold_raw)
with open(path, newline="") as f:
    rows = list(csv.reader(f, delimiter=";"))
if len(rows) < 2:
    print("FAIL empty_or_missing_data")
    raise SystemExit(0)
header = rows[0]
last = rows[-1]
values = dict(zip(header, last))

def as_int(name: str) -> int:
    raw = values.get(name, "")
    return int(raw) if raw else 0

total = as_int("TotalCallCreated")
success = as_int("SuccessfulCall(C)")
failed = as_int("FailedCall(C)")
current = as_int("CurrentCall")
basis = max(expected, total, 1)
success_pct = success / basis * 100.0
if success_pct + 1e-9 < threshold or failed != 0 or current != 0:
    print(
        "FAIL "
        f"success={success} total={total} expected={expected} "
        f"failed={failed} current={current} success_pct={success_pct:.3f} "
        f"threshold={threshold}"
    )
else:
    print(
        "PASS "
        f"success={success} total={total} expected={expected} "
        f"failed={failed} current={current} success_pct={success_pct:.3f} "
        f"threshold={threshold}"
    )
PY
}

for CPS in $CPS_LEVELS; do
  CALLS=$(( CPS * STEADY_SECS ))
  if [[ "$CALLS" -lt 10 ]]; then CALLS=10; fi
  START_EPOCH="$(date +%s)"

  # Pick a per-run local sipp port to avoid contention across re-runs.
  SIPP_LOCAL_PORT=$(( 35000 + RANDOM % 1000 ))
  STAT_PREFIX="$RESULTS_DIR/${TAG}_${CPS}cps"
  SCREEN="$RESULTS_DIR/${TAG}_${CPS}cps_screen.log"
  ERRLOG="$RESULTS_DIR/${TAG}_${CPS}cps_errors.log"

  echo "[run_comparison] === ${TAG} @ ${CPS} CPS (${CALLS} calls, sipp:${SIPP_LOCAL_PORT}) ==="

  # Drive the run. Timeout = STEADY_SECS + 30 s buffer so we don't get
  # stuck if the SUT silently drops calls (sipp would otherwise retry
  # forever).
  RUN_TIMEOUT=$(( STEADY_SECS + 30 ))
  set +e
  "$SIPP_BIN" \
    -sf "$SCENARIO" \
    -r "$CPS" \
    -m "$CALLS" \
    -p "$SIPP_LOCAL_PORT" \
    -nostdin \
    -trace_stat \
    -stf "$STAT_PREFIX" \
    -trace_screen \
    -screen_file "$SCREEN" \
    -trace_err \
    -error_file "$ERRLOG" \
    -timeout "${RUN_TIMEOUT}s" \
    -timeout_error \
    "${TARGET_HOST}:${TARGET_PORT}" >/dev/null 2>&1
  RC=$?
  set -e

  if [[ -f "$STAT_PREFIX" ]]; then
    STAT_FILE="$STAT_PREFIX"
  else
    STAT_FILE="$(ls -1 "${STAT_PREFIX}"_*.csv "${STAT_PREFIX}".csv 2>/dev/null | head -1 || true)"
  fi
  if [[ -z "$STAT_FILE" ]]; then
    echo "[run_comparison] FAIL: no stat CSV produced for ${TAG}@${CPS}cps (rc=$RC)" >&2
    RUN_STATUS=FAIL
    RUN_FAILURES=$((RUN_FAILURES + 1))
  else
    EVAL_RESULT="$(evaluate_stat_csv "$STAT_FILE" "$CALLS" "$MIN_SUCCESS_PCT")"
    echo "[run_comparison] -> $STAT_FILE (rc=$RC; $EVAL_RESULT)"
    RUN_STATUS="${EVAL_RESULT%% *}"
    if [[ "$RUN_STATUS" != "PASS" ]]; then
      RUN_FAILURES=$((RUN_FAILURES + 1))
    fi
  fi
  DURATION="$(( $(date +%s) - START_EPOCH ))"
  record_run "$RUN_STATUS" "$CPS" "$CALLS" "$DURATION" "$RC" "$STAT_FILE" "$SCREEN" "$ERRLOG"

  # Small inter-run pause so the SUT settles between CPS bands.
  sleep 2
done

if compgen -G "$RESULTS_DIR/${TAG}_*cps*.csv" >/dev/null || compgen -G "$RESULTS_DIR/${TAG}_*cps" >/dev/null; then
  "$SCRIPT_DIR/analyze.py" "$RESULTS_DIR" "$ANALYSIS_MD" >/dev/null 2>&1 || true
else
  {
    echo "# SIPp Analysis"
    echo
    echo "No SIPp stat CSV files were produced."
  } >"$ANALYSIS_MD"
fi
if [[ "$RUN_FAILURES" -eq 0 ]]; then
  write_run_summary 0
else
  write_run_summary 1
fi
echo "[run_comparison] done. results in $RESULTS_DIR"
if [[ "$RUN_FAILURES" -ne 0 ]]; then
  exit 1
fi
