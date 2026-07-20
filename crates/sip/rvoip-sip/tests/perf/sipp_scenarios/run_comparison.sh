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
SIPP_SHARD_CPS="${RVOIP_PERF_SIPP_SHARD_CPS:-0}"
SIPP_EXTRA_ARGS="${RVOIP_PERF_SIPP_EXTRA_ARGS:--timer_resol 1 -max_recv_loops 10000 -max_sched_loops 10000}"
INTER_RUN_PAUSE_SECS="${RVOIP_PERF_INTER_RUN_PAUSE_SECS:-2}"
RUN_FAILURES=0

if ! [[ "$STEADY_SECS" =~ ^[1-9][0-9]*$ ]]; then
  echo "RVOIP_PERF_STEADY_SECS must be a positive integer (got '$STEADY_SECS')" >&2
  exit 1
fi
if ! [[ "$SIPP_SHARD_CPS" =~ ^[0-9]+$ ]]; then
  echo "RVOIP_PERF_SIPP_SHARD_CPS must be a non-negative integer (got '$SIPP_SHARD_CPS')" >&2
  exit 1
fi

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
    echo "- sipp_shard_cps: $SIPP_SHARD_CPS (0 preserves the historical single-runner topology)"
    echo "- sipp_extra_args: $SIPP_EXTRA_ARGS"
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

evaluate_stat_csvs() {
  local expected_calls="$1"
  local min_success_pct="$2"
  shift 2
  python3 - "$expected_calls" "$min_success_pct" "$@" <<'PY'
import csv
import sys

expected_raw, threshold_raw, *paths = sys.argv[1:]
expected = int(expected_raw)
threshold = float(threshold_raw)
totals = {"total": 0, "success": 0, "failed": 0, "current": 0}

try:
    for path in paths:
        with open(path, newline="") as f:
            rows = list(csv.reader(f, delimiter=";"))
        if len(rows) < 2:
            print(f"FAIL empty_or_missing_data path={path}")
            raise SystemExit(0)
        values = dict(zip(rows[0], rows[-1]))

        def as_int(name: str) -> int:
            raw = values.get(name, "")
            return int(raw) if raw else 0

        totals["total"] += as_int("TotalCallCreated")
        totals["success"] += as_int("SuccessfulCall(C)")
        totals["failed"] += as_int("FailedCall(C)")
        totals["current"] += as_int("CurrentCall")
except (OSError, ValueError) as error:
    print(f"FAIL unreadable_stat error={error}")
    raise SystemExit(0)

total = totals["total"]
success = totals["success"]
failed = totals["failed"]
current = totals["current"]
basis = max(expected, total, 1)
success_pct = success / basis * 100.0
if total != expected or success_pct + 1e-9 < threshold or failed != 0 or current != 0:
    print(
        "FAIL "
        f"success={success} total={total} expected={expected} "
        f"failed={failed} current={current} success_pct={success_pct:.3f} "
        f"threshold={threshold} shards={len(paths)}"
    )
else:
    print(
        "PASS "
        f"success={success} total={total} expected={expected} "
        f"failed={failed} current={current} success_pct={success_pct:.3f} "
        f"threshold={threshold} shards={len(paths)}"
    )
PY
}

ceil_div() {
  local numerator="$1"
  local denominator="$2"
  echo $(( (numerator + denominator - 1) / denominator ))
}

runner_count_for_cps() {
  local cps="$1"
  if [[ "$SIPP_SHARD_CPS" -eq 0 ]]; then
    echo 1
  else
    ceil_div "$cps" "$SIPP_SHARD_CPS"
  fi
}

resolve_stat_file() {
  local prefix="$1"
  local candidate
  if [[ -f "$prefix" ]]; then
    printf '%s' "$prefix"
    return
  fi
  for candidate in "${prefix}"_*.csv "${prefix}".csv; do
    if [[ -f "$candidate" ]]; then
      printf '%s' "$candidate"
      return
    fi
  done
}

join_by_comma() {
  local IFS=,
  echo "$*"
}

run_sipp_shard() {
  local stat_prefix="$1"
  local shard_cps="$2"
  local shard_calls="$3"
  local call_limit="$4"
  local local_port="$5"
  local run_timeout="$6"
  local screen="$7"
  local errlog="$8"
  local -a cmd extra_args

  cmd=(
    "$SIPP_BIN"
    -sf "$SCENARIO"
    -r "$shard_cps"
    -m "$shard_calls"
    -l "$call_limit"
    -p "$local_port"
    -nostdin
  )
  if [[ -n "$SIPP_EXTRA_ARGS" ]]; then
    read -r -a extra_args <<< "$SIPP_EXTRA_ARGS"
    cmd+=("${extra_args[@]}")
  fi
  cmd+=(
    -trace_stat
    -stf "$stat_prefix"
    -trace_screen
    -screen_file "$screen"
    -trace_err
    -error_file "$errlog"
    -timeout "${run_timeout}s"
    -timeout_error
    "${TARGET_HOST}:${TARGET_PORT}"
  )
  "${cmd[@]}" >/dev/null 2>&1
}

for CPS in $CPS_LEVELS; do
  if ! [[ "$CPS" =~ ^[1-9][0-9]*$ ]]; then
    echo "RVOIP_PERF_CPS values must be positive integers (got '$CPS')" >&2
    exit 1
  fi
  CALLS=$(( CPS * STEADY_SECS ))
  if [[ "$CALLS" -lt 10 ]]; then CALLS=10; fi
  START_EPOCH="$(date +%s)"
  RUNNERS="$(runner_count_for_cps "$CPS")"
  BASE_PORT=$(( 35000 + RANDOM % 1000 ))
  REMAINING_CPS="$CPS"
  REMAINING_CALLS="$CALLS"
  PIDS=()
  STAT_PREFIXES=()
  SCREENS=()
  ERRLOGS=()

  echo "[run_comparison] === ${TAG} @ ${CPS} CPS (${CALLS} calls, ${RUNNERS} runner(s), sipp:${BASE_PORT}+) ==="

  # Drive the run. Timeout = STEADY_SECS + 30 s buffer so we don't get
  # stuck if the SUT silently drops calls (sipp would otherwise retry
  # forever).
  RUN_TIMEOUT=$(( STEADY_SECS + 30 ))
  for (( SHARD=0; SHARD<RUNNERS; SHARD++ )); do
    REMAINING_SHARDS=$(( RUNNERS - SHARD ))
    SHARD_CPS=$(( (REMAINING_CPS + REMAINING_SHARDS - 1) / REMAINING_SHARDS ))
    REMAINING_CPS=$(( REMAINING_CPS - SHARD_CPS ))
    if [[ "$RUNNERS" -eq 1 ]]; then
      SHARD_CALLS="$CALLS"
      STAT_PREFIX="$RESULTS_DIR/${TAG}_${CPS}cps"
    else
      SHARD_CALLS=$(( SHARD_CPS * STEADY_SECS ))
      STAT_PREFIX="$RESULTS_DIR/${TAG}_${CPS}cps_s${SHARD}"
    fi
    REMAINING_CALLS=$(( REMAINING_CALLS - SHARD_CALLS ))
    if [[ "$SHARD" -eq $(( RUNNERS - 1 )) && "$REMAINING_CALLS" -ne 0 ]]; then
      # This only matters for the minimum-ten-calls smoke case; normal shards
      # are an exact rate * duration partition.
      SHARD_CALLS=$(( SHARD_CALLS + REMAINING_CALLS ))
      REMAINING_CALLS=0
    fi
    CALL_LIMIT_HEADROOM="$SHARD_CPS"
    if [[ "$CALL_LIMIT_HEADROOM" -lt 10 ]]; then CALL_LIMIT_HEADROOM=10; fi
    CALL_LIMIT=$(( SHARD_CALLS + CALL_LIMIT_HEADROOM ))
    SIPP_LOCAL_PORT=$(( BASE_PORT + SHARD ))
    SCREEN="${STAT_PREFIX}_screen.log"
    ERRLOG="${STAT_PREFIX}_errors.log"
    echo "[run_comparison] shard=${SHARD} rate=${SHARD_CPS} calls=${SHARD_CALLS} call_limit=${CALL_LIMIT} sipp:${SIPP_LOCAL_PORT}"
    run_sipp_shard "$STAT_PREFIX" "$SHARD_CPS" "$SHARD_CALLS" "$CALL_LIMIT" \
      "$SIPP_LOCAL_PORT" "$RUN_TIMEOUT" "$SCREEN" "$ERRLOG" &
    PIDS+=("$!")
    STAT_PREFIXES+=("$STAT_PREFIX")
    SCREENS+=("$SCREEN")
    ERRLOGS+=("$ERRLOG")
  done

  RC=0
  MISSING_STATS=0
  STAT_FILES=()
  for (( SHARD=0; SHARD<RUNNERS; SHARD++ )); do
    SHARD_RC=0
    if wait "${PIDS[$SHARD]}"; then
      SHARD_RC=0
    else
      SHARD_RC=$?
    fi
    if [[ "$SHARD_RC" -ne 0 && "$RC" -eq 0 ]]; then RC="$SHARD_RC"; fi
    STAT_FILE="$(resolve_stat_file "${STAT_PREFIXES[$SHARD]}")"
    if [[ -z "$STAT_FILE" ]]; then
      MISSING_STATS=$((MISSING_STATS + 1))
      echo "[run_comparison] shard=${SHARD} rc=${SHARD_RC} missing_stat=${STAT_PREFIXES[$SHARD]}" >&2
    else
      STAT_FILES+=("$STAT_FILE")
      echo "[run_comparison] shard=${SHARD} rc=${SHARD_RC} stat=${STAT_FILE}"
    fi
  done

  if [[ "$MISSING_STATS" -ne 0 ]]; then
    EVAL_RESULT="FAIL missing_stat_files=${MISSING_STATS}/${RUNNERS}"
  else
    EVAL_RESULT="$(evaluate_stat_csvs "$CALLS" "$MIN_SUCCESS_PCT" "${STAT_FILES[@]}")"
  fi
  RUN_STATUS="${EVAL_RESULT%% *}"
  if [[ "$RC" -ne 0 ]]; then
    RUN_STATUS=FAIL
    EVAL_RESULT="$EVAL_RESULT nonzero_sipp_rc=$RC"
  fi

  STAT_FILE="$(join_by_comma "${STAT_FILES[@]}")"
  SCREEN="$(join_by_comma "${SCREENS[@]}")"
  ERRLOG="$(join_by_comma "${ERRLOGS[@]}")"
  echo "[run_comparison] -> ${STAT_FILE:-no-stat-file} (rc=$RC; $EVAL_RESULT)"
  if [[ "$RUN_STATUS" != "PASS" ]]; then
    RUN_FAILURES=$((RUN_FAILURES + 1))
  fi
  DURATION="$(( $(date +%s) - START_EPOCH ))"
  record_run "$RUN_STATUS" "$CPS" "$CALLS" "$DURATION" "$RC" "$STAT_FILE" "$SCREEN" "$ERRLOG"

  # Small inter-run pause so the SUT settles between CPS bands.
  sleep "$INTER_RUN_PAUSE_SECS"
done

if compgen -G "$RESULTS_DIR/${TAG}_*cps*.csv" >/dev/null || compgen -G "$RESULTS_DIR/${TAG}_*cps*" >/dev/null; then
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
