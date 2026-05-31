#!/usr/bin/env bash
# rvoip-sip beta-candidate release gate.
#
# This script is intentionally release-gate-first: it records deterministic
# commands and artifacts even when an external lab dependency is unavailable.
# Missing external prerequisites are reported as SKIP by default. Set
# BETA_GATE_REQUIRE_EXTERNAL=1 to make skipped external gates fail the run.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
WORKSPACE_ROOT="$(cd "$CRATE_DIR/../.." && pwd)"

MODE="${BETA_GATE_MODE:-local}"
REQUIRE_EXTERNAL="${BETA_GATE_REQUIRE_EXTERNAL:-0}"
BETA_FUZZ_TOOLCHAIN="${BETA_FUZZ_TOOLCHAIN:-nightly}"
TIMESTAMP="$(date -u +%Y%m%dT%H%M%SZ)"
ARTIFACT_DIR="${BETA_GATE_ARTIFACT_DIR:-$WORKSPACE_ROOT/target/beta-gate/$TIMESTAMP}"
SUMMARY="$ARTIFACT_DIR/summary.md"
ENV_REPORT="$ARTIFACT_DIR/environment/environment.md"
FAILURES=0
SKIPS=0
SIPP_LISTENER_PID=""

cleanup_background() {
  if [ -n "$SIPP_LISTENER_PID" ] && kill -0 "$SIPP_LISTENER_PID" >/dev/null 2>&1; then
    kill -INT "$SIPP_LISTENER_PID" >/dev/null 2>&1 || true
    wait "$SIPP_LISTENER_PID" >/dev/null 2>&1 || true
  fi
}
trap cleanup_background EXIT

usage() {
  cat <<'EOF'
Usage: beta_gate.sh [--local|--full|--interop|--perf|--security] [--require-external]

Modes:
  --local    Fast local gate: format/check/tests/docs/examples/compliance smoke.
  --full     Local gate plus interop and perf gates.
  --interop  External interop gates only.
  --perf     Performance gates only.
  --security Dependency audit and parser fuzz-smoke gates only.

Environment:
  BETA_GATE_ARTIFACT_DIR         Output directory. Defaults to target/beta-gate/<timestamp>.
  BETA_REPORT_DIR                Crate-local report directory. Defaults to crates/rvoip-sip/beta-report.
  BETA_REPORT_PACKAGE=0          Disable copying completed artifacts into BETA_REPORT_DIR.
  BETA_GATE_REQUIRE_EXTERNAL=1   Treat skipped external gates as failures.
  BETA_RUN_PBX=1                 Run examples/pbx/run.sh when PBX configs are present.
  BETA_RUN_LOCAL_PBX=1           Manage ~/Developer/asterisk and ~/Developer/freeswitch sequentially.
  BETA_RESTORE_LOCAL_PBX=0       Do not restore the PBX container that was running before the gate.
  BETA_PBX_API                   PBX API subset: endpoint|stream_peer|callback|all. Defaults to all.
  BETA_PBX_SCENARIO              PBX scenario subset. Defaults to all.
  BETA_PBX_PROVIDER              PBX provider subset: asterisk|freeswitch|both. Defaults to both.
  BETA_ASTERISK_DIR              Local Asterisk checkout. Defaults to ~/Developer/asterisk.
  BETA_FREESWITCH_DIR            Local FreeSWITCH checkout. Defaults to ~/Developer/freeswitch.
  BETA_PBX_LOG_TAIL              Docker log lines captured around PBX lifecycle events. Defaults to 1000.
  BETA_CAPTURE_DOCKER_LOGS=0     Disable local PBX Docker inspect/log snapshots.
  BETA_RUN_SIPP=1                Run SIPp. Defaults to a managed local rvoip target.
  BETA_SIPP_TARGET_HOST          SIPp target host. Defaults to managed local rvoip target.
  BETA_SIPP_TARGET_PORT          SIPp target port. Defaults to 35060 for managed target.
  BETA_SIPP_CPS                  CPS list for standalone SIPp gate.
  BETA_SIPP_PERF_PROFILE         Managed SIPp target recipe. Defaults to pbx-media-server.
  BETA_PERF_PROFILE_MATRIX       Perf profile:CPS matrix. Defaults to endpoint, pbx-media-server,
                                  and signaling-only-server-high-performance.
  BETA_PERFORMANCE_RECIPE_FILE   Optional YAML recipe book path.
  RVOIP_PERF_MIN_SUCCESS_PCT     SIPp pass threshold. Defaults to 99.9.
  BETA_RUN_STRICT_UA=0           Disable the baresip strict-UA gate; fails with --require-external.
  BETA_RUN_LONG_SOAK=0           Disable the ignored soak test; fails with --require-external.
  BETA_RUN_FUZZ_SMOKE=0          Disable parser fuzz-smoke coverage; fails with --require-external.
  BETA_FUZZ_TOOLCHAIN            Rust toolchain used by cargo-fuzz. Defaults to nightly.
  BETA_FUZZ_SMOKE_RUNS           libFuzzer runs per parser target. Defaults to 1000.
  BETA_FUZZ_SMOKE_SECONDS        libFuzzer max_total_time per parser target. Defaults to 10.
  RVOIP_PERF_SOAK_DURATION_SECS  Soak duration. Defaults to the perf test default.
  RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR
                                  Soak RSS growth threshold. Defaults to Config's 10 MB/hr.
  RVOIP_PERF_APP_EVENT_CHANNEL_CAPACITY
                                  App-facing event buffer capacity for perf soaks.
                                  Defaults to Config's recipe value.
  RVOIP_PERF_RSS_TAIL_WINDOW_SECS
                                  Sustained RSS slope window. Defaults to 60.
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --local) MODE=local ;;
    --full) MODE=full ;;
    --interop) MODE=interop ;;
    --perf) MODE=perf ;;
    --security) MODE=security ;;
    --require-external) REQUIRE_EXTERNAL=1 ;;
    --help|-h) usage; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

mkdir -p "$ARTIFACT_DIR"
cat > "$SUMMARY" <<EOF
# rvoip-sip Beta Gate Summary

- timestamp: $TIMESTAMP
- mode: $MODE
- workspace: $WORKSPACE_ROOT
- artifact_dir: $ARTIFACT_DIR
- environment: \`environment/environment.md\`
EOF

slugify() {
  printf '%s' "$1" | tr '[:upper:] /:' '[:lower:]___' | tr -cd 'a-z0-9_.-'
}

record() {
  local status="$1"
  local name="$2"
  local log="$3"
  local duration="${4:--}"
  printf '| %s | %s | %s | `%s` |\n' "$status" "$name" "$duration" "${log#$ARTIFACT_DIR/}" >> "$SUMMARY"
}

run_gate() {
  local name="$1"
  shift
  local log="$ARTIFACT_DIR/$(slugify "$name").log"
  local started_at
  local ended_at
  local start_epoch
  local end_epoch
  local duration
  local status
  echo
  echo "==> $name"
  started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  start_epoch="$(date +%s)"
  {
    echo "gate: $name"
    echo "started_at_utc: $started_at"
    echo "workspace: $WORKSPACE_ROOT"
    echo "command: $*"
    echo
    echo "+ $*"
  } > "$log"
  set +e
  (cd "$WORKSPACE_ROOT" && "$@" >> "$log" 2>&1)
  status=$?
  set -e
  ended_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  end_epoch="$(date +%s)"
  duration="$((end_epoch - start_epoch))s"
  {
    echo
    echo "ended_at_utc: $ended_at"
    echo "duration_seconds: $((end_epoch - start_epoch))"
    echo "exit_status: $status"
  } >> "$log"
  if [ "$status" -eq 0 ]; then
    record "PASS" "$name" "$log" "$duration"
    return 0
  else
    record "FAIL" "$name" "$log" "$duration"
    FAILURES=$((FAILURES + 1))
    echo "FAIL: $name (see $log)" >&2
    return 1
  fi
}

skip_gate() {
  local name="$1"
  local reason="$2"
  local log="$ARTIFACT_DIR/$(slugify "$name").log"
  {
    echo "SKIP: $name"
    echo "$reason"
  } > "$log"
  record "SKIP" "$name" "$log" "-"
  SKIPS=$((SKIPS + 1))
  echo "SKIP: $name - $reason"
  if [ "$REQUIRE_EXTERNAL" = "1" ]; then
    FAILURES=$((FAILURES + 1))
  fi
}

capture_command() {
  local output="$1"
  shift
  {
    echo "+ $*"
    "$@"
  } > "$output" 2>&1 || true
}

redacted_env() {
  env | LC_ALL=C sort | awk -F= '
    /^(BETA_|PBX_|RVOIP_|SIPP_|ASTERISK_|FREESWITCH_|SIP_|TLS_)/ {
      key=$1
      value=substr($0, length($1) + 2)
      redacted=key
      upper=toupper(key)
      if (upper ~ /(PASSWORD|PASS|SECRET|TOKEN|CREDENTIAL|PRIVATE|AUTHORIZATION)/) {
        print key"=<redacted>"
      } else {
        print key"="value
      }
    }
  '
}

captured_payload() {
  local file="$1"
  if [ ! -f "$file" ]; then
    printf 'not captured\n'
    return
  fi
  awk 'NR == 1 && /^\+ / { next } { print }' "$file"
}

captured_first_line() {
  local value
  value="$(captured_payload "$1" | awk 'NF { print; exit }')"
  printf '%s' "${value:-none}"
}

captured_status_label() {
  local payload
  payload="$(captured_payload "$1")"
  if [ "$payload" = "not captured" ]; then
    printf 'not captured'
  elif [ -z "$payload" ]; then
    printf 'clean'
  else
    printf 'dirty'
  fi
}

markdown_payload_block() {
  local title="$1"
  local file="$2"
  echo "## $title"
  echo
  echo '```text'
  captured_payload "$file"
  echo '```'
  echo
}

markdown_file_block() {
  local title="$1"
  local file="$2"
  echo "## $title"
  echo
  if [ -f "$file" ]; then
    echo '```text'
    cat "$file"
    echo '```'
  else
    echo 'not captured'
  fi
  echo
}

markdown_local_pbx_config() {
  local name="$1"
  local source_dir="$2"
  local out_dir="$3"
  echo "## Local PBX Config: $name"
  echo
  echo "- source_dir: $source_dir"
  if [ -d "$out_dir" ]; then
    echo "- captured_files:"
    find "$out_dir" -maxdepth 1 -type f -print | sort | while IFS= read -r file; do
      echo "  - ${file#$ARTIFACT_DIR/}"
    done
  else
    echo "- captured_files: none"
  fi
  echo
  for file in README.md docker-compose.yml rvoip-local.env freeswitch-local.env git-rev.txt git-status.txt; do
    if [ -f "$out_dir/$file" ]; then
      markdown_file_block "$name $file" "$out_dir/$file"
    fi
  done
}

redact_file() {
  local input="$1"
  local output="$2"
  if [ ! -f "$input" ]; then
    return
  fi
  sed -E \
    -e 's/([Pp][Aa][Ss][Ss][Ww][Oo][Rr][Dd][[:space:]]*[:=][[:space:]]*).*/\1<redacted>/' \
    -e 's/([Ss][Ee][Cc][Rr][Ee][Tt][[:space:]]*[:=][[:space:]]*).*/\1<redacted>/' \
    -e 's/([Tt][Oo][Kk][Ee][Nn][[:space:]]*[:=][[:space:]]*).*/\1<redacted>/' \
    -e 's/password123/<redacted>/g' \
    "$input" > "$output" || true
}

capture_docker_snapshot() {
  local label="$1"
  local dir="$ARTIFACT_DIR/environment/docker-$label"
  local tail_lines="${BETA_PBX_LOG_TAIL:-1000}"
  if [ "${BETA_CAPTURE_DOCKER_LOGS:-1}" = "0" ]; then
    return
  fi
  mkdir -p "$dir"
  if ! command -v docker >/dev/null 2>&1; then
    echo "docker not found" > "$dir/README.txt"
    return
  fi
  capture_command "$dir/docker-ps.txt" docker ps --all
  for container in rvoip-asterisk rvoip-freeswitch; do
    if docker inspect "$container" >/dev/null 2>&1; then
      capture_command "$dir/${container}-inspect.json" docker inspect "$container"
      capture_command "$dir/${container}-logs-tail.txt" docker logs --tail "$tail_lines" "$container"
    else
      echo "$container not found" > "$dir/${container}-missing.txt"
    fi
  done
}

copy_local_pbx_config_evidence() {
  local name="$1"
  local dir="$2"
  local out="$ARTIFACT_DIR/environment/local-pbx/$name"
  mkdir -p "$out"
  for file in README.md docker-compose.yml rvoip-local.env freeswitch-local.env; do
    if [ -f "$dir/$file" ]; then
      redact_file "$dir/$file" "$out/$file"
    fi
  done
  if [ -d "$dir/.git" ]; then
    capture_command "$out/git-rev.txt" git -C "$dir" rev-parse --short HEAD
    capture_command "$out/git-status.txt" git -C "$dir" status --short
  fi
}

write_environment_report() {
  local env_dir="$ARTIFACT_DIR/environment"
  local asterisk_dir="${BETA_ASTERISK_DIR:-$HOME/Developer/asterisk}"
  local freeswitch_dir="${BETA_FREESWITCH_DIR:-$HOME/Developer/freeswitch}"
  mkdir -p "$env_dir"

  capture_command "$env_dir/git-rev.txt" git -C "$WORKSPACE_ROOT" rev-parse --short HEAD
  capture_command "$env_dir/git-status.txt" git -C "$WORKSPACE_ROOT" status --short
  capture_command "$env_dir/rustc-version.txt" rustc --version --verbose
  capture_command "$env_dir/cargo-version.txt" cargo --version --verbose
  capture_command "$env_dir/host-uname.txt" uname -a
  if command -v sw_vers >/dev/null 2>&1; then
    capture_command "$env_dir/macos-version.txt" sw_vers
  fi
  if command -v sysctl >/dev/null 2>&1; then
    capture_command "$env_dir/host-hardware.txt" sysctl -n machdep.cpu.brand_string hw.physicalcpu hw.logicalcpu hw.memsize
  fi
  if command -v docker >/dev/null 2>&1; then
    capture_command "$env_dir/docker-version.txt" docker version
    capture_command "$env_dir/docker-ps-start.txt" docker ps --all
  fi
  if command -v docker-compose >/dev/null 2>&1; then
    capture_command "$env_dir/docker-compose-version.txt" docker-compose version
  elif docker compose version >/dev/null 2>&1; then
    capture_command "$env_dir/docker-compose-version.txt" docker compose version
  fi
  redacted_env > "$env_dir/beta-env-redacted.txt"
  copy_local_pbx_config_evidence asterisk "$asterisk_dir"
  copy_local_pbx_config_evidence freeswitch "$freeswitch_dir"
  capture_docker_snapshot start

  {
    cat <<EOF
# Beta Gate Environment

- timestamp_utc: $TIMESTAMP
- mode: $MODE
- workspace: $WORKSPACE_ROOT
- artifact_dir: $ARTIFACT_DIR
- git_revision: \`$(captured_first_line "$env_dir/git-rev.txt")\`
- git_status: \`$(captured_status_label "$env_dir/git-status.txt")\`
- rustc: \`$(captured_first_line "$env_dir/rustc-version.txt")\`
- cargo: \`$(captured_first_line "$env_dir/cargo-version.txt")\`
- host: \`$(captured_first_line "$env_dir/host-uname.txt")\`
- docker: \`$(captured_first_line "$env_dir/docker-version.txt")\`

Docker snapshots captured during local PBX lifecycle events are stored under
\`environment/docker-<phase>/\`. Secrets in copied local env/config files are
redacted by key name before being written into this artifact tree.
EOF

    echo
    markdown_payload_block "Git Status" "$env_dir/git-status.txt"
    markdown_payload_block "Rust Toolchain" "$env_dir/rustc-version.txt"
    markdown_payload_block "Cargo Toolchain" "$env_dir/cargo-version.txt"
    markdown_payload_block "Host Kernel" "$env_dir/host-uname.txt"
    if [ -f "$env_dir/macos-version.txt" ]; then
      markdown_payload_block "macOS Version" "$env_dir/macos-version.txt"
    fi
    if [ -f "$env_dir/host-hardware.txt" ]; then
      markdown_payload_block "Host Hardware" "$env_dir/host-hardware.txt"
    fi
    if [ -f "$env_dir/docker-version.txt" ]; then
      markdown_payload_block "Docker Version" "$env_dir/docker-version.txt"
    fi
    if [ -f "$env_dir/docker-compose-version.txt" ]; then
      markdown_payload_block "Docker Compose Version" "$env_dir/docker-compose-version.txt"
    fi
    if [ -f "$env_dir/docker-ps-start.txt" ]; then
      markdown_payload_block "Initial Docker State" "$env_dir/docker-ps-start.txt"
    fi
    markdown_file_block "Redacted Gate Environment" "$env_dir/beta-env-redacted.txt"
    markdown_local_pbx_config asterisk "$asterisk_dir" "$env_dir/local-pbx/asterisk"
    markdown_local_pbx_config freeswitch "$freeswitch_dir" "$env_dir/local-pbx/freeswitch"

    cat <<EOF
## Raw Evidence Files

The inlined values above are also retained as raw evidence files under
\`environment/\` so scripts can consume the same captured data without parsing
Markdown.
EOF
  } > "$ENV_REPORT"
}

write_summary_gate_table_header() {
  local env_dir="$ARTIFACT_DIR/environment"
  {
    cat <<EOF

## Environment Snapshot

- git_revision: \`$(captured_first_line "$env_dir/git-rev.txt")\`
- git_status: \`$(captured_status_label "$env_dir/git-status.txt")\`
- rustc: \`$(captured_first_line "$env_dir/rustc-version.txt")\`
- cargo: \`$(captured_first_line "$env_dir/cargo-version.txt")\`
- host: \`$(captured_first_line "$env_dir/host-uname.txt")\`
- docker: \`$(captured_first_line "$env_dir/docker-version.txt")\`
- beta_profile_matrix: \`${BETA_PERF_PROFILE_MATRIX:-endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000}\`
- beta_performance_recipe_file: \`${BETA_PERFORMANCE_RECIPE_FILE:-bundled config/performance-recipes.yaml}\`
- beta_pbx_provider: \`${BETA_PBX_PROVIDER:-both}\`
- beta_pbx_api: \`${BETA_PBX_API:-all}\`
- beta_pbx_scenario: \`${BETA_PBX_SCENARIO:-all}\`
- beta_run_local_pbx: \`${BETA_RUN_LOCAL_PBX:-0}\`
- beta_run_sipp: \`${BETA_RUN_SIPP:-1}\`
- beta_run_strict_ua: \`${BETA_RUN_STRICT_UA:-1}\`
- beta_run_long_soak: \`${BETA_RUN_LONG_SOAK:-1}\`
- rvoip_perf_soak_duration_secs: \`${RVOIP_PERF_SOAK_DURATION_SECS:-perf test default}\`
- rvoip_perf_max_rss_growth_mb_per_hr: \`${RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR:-Config default (10)}\`
- rvoip_perf_app_event_channel_capacity: \`${RVOIP_PERF_APP_EVENT_CHANNEL_CAPACITY:-Config default}\`
- rvoip_perf_rss_tail_window_secs: \`${RVOIP_PERF_RSS_TAIL_WINDOW_SECS:-60}\`

Full environment evidence, Docker state, redacted runtime variables, and local
PBX config snapshots are in \`environment/environment.md\`.

## Gates

| Status | Gate | Duration | Log |
|--------|------|----------|-----|
EOF
  } >> "$SUMMARY"
}

beta_report_root() {
  printf '%s' "${BETA_REPORT_DIR:-$CRATE_DIR/beta-report}"
}

beta_report_run_dir() {
  printf '%s/%s' "$(beta_report_root)" "$TIMESTAMP"
}

write_report_manifest() {
  local report_dir="$1"
  local manifest="$report_dir/report-manifest.md"
  cat > "$manifest" <<EOF
# rvoip-sip Beta Report Manifest

- timestamp: $TIMESTAMP
- mode: $MODE
- workspace: $WORKSPACE_ROOT
- source_artifact_dir: $ARTIFACT_DIR
- report_dir: $report_dir
- summary: \`summary.md\`
- environment: \`environment/environment.md\`
- generated_at_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)

## Primary Evidence

- \`summary.md\`
- \`environment/environment.md\`
- \`pbx/summary.md\`
- \`pbx/matrix.tsv\`
- \`sipp/environment.md\`
- \`sipp/run_summary.md\`
- \`sipp/analysis.md\`
- \`strict-ua/summary.md\`
- \`security/cargo-audit.txt\`
- \`security/fuzz/\`
- \`perf-results/\`

The report directory is a packaged copy of the beta-gate artifact tree plus
the current raw \`target/perf-results\` files. Logs, matrices, redacted
environment evidence, PBX lifecycle snapshots, scenario metadata, and perf
JSON/markdown outputs are kept with their original relative paths where
possible.
EOF
}

package_beta_report() {
  if [ "${BETA_REPORT_PACKAGE:-1}" = "0" ]; then
    return 0
  fi

  local root
  local report_dir
  local artifact_abs
  local report_abs
  root="$(beta_report_root)"
  report_dir="$(beta_report_run_dir)"
  mkdir -p "$report_dir"
  artifact_abs="$(cd "$ARTIFACT_DIR" && pwd -P)"
  report_abs="$(cd "$report_dir" && pwd -P)"

  if [ "$artifact_abs" != "$report_abs" ]; then
    (cd "$ARTIFACT_DIR" && tar cf - .) | (cd "$report_dir" && tar xf -)
  fi

  if [ -d "$WORKSPACE_ROOT/target/perf-results" ]; then
    mkdir -p "$report_dir/perf-results"
    (cd "$WORKSPACE_ROOT/target/perf-results" && tar cf - .) | \
      (cd "$report_dir/perf-results" && tar xf -)
  fi

  write_report_manifest "$report_dir"
  printf '%s\n' "$TIMESTAMP" > "$root/latest.txt"
}

container_running() {
  local name="$1"
  docker ps --format '{{.Names}}' 2>/dev/null | grep -Fxq "$name"
}

pbx_provider_enabled() {
  local provider="$1"
  local selected="${BETA_PBX_PROVIDER:-both}"
  case "$selected" in
    both|all) return 0 ;;
    ast|asterisk) [ "$provider" = "asterisk" ] ;;
    fs|free-switch|freeswitch) [ "$provider" = "freeswitch" ] ;;
    *) return 1 ;;
  esac
}

run_local_pbx_gate() {
  local asterisk_dir="${BETA_ASTERISK_DIR:-$HOME/Developer/asterisk}"
  local freeswitch_dir="${BETA_FREESWITCH_DIR:-$HOME/Developer/freeswitch}"
  local pbx_api="${BETA_PBX_API:-all}"
  local pbx_scenario="${BETA_PBX_SCENARIO:-all}"
  local pbx_output_root="$ARTIFACT_DIR/pbx"
  local restore="${BETA_RESTORE_LOCAL_PBX:-1}"
  local initially_asterisk=0
  local initially_freeswitch=0

  if [ ! -x "$asterisk_dir/scripts/up.sh" ] || [ ! -x "$asterisk_dir/scripts/down.sh" ]; then
    skip_gate "local Asterisk PBX matrix" "Asterisk scripts not found under $asterisk_dir."
    return
  fi
  if [ ! -x "$freeswitch_dir/scripts/up.sh" ] || [ ! -x "$freeswitch_dir/scripts/down.sh" ]; then
    skip_gate "local FreeSWITCH PBX matrix" "FreeSWITCH scripts not found under $freeswitch_dir."
    return
  fi

  if container_running rvoip-asterisk; then initially_asterisk=1; fi
  if container_running rvoip-freeswitch; then initially_freeswitch=1; fi
  mkdir -p "$pbx_output_root"
  rm -f "$pbx_output_root/matrix.tsv" "$pbx_output_root/summary.md"
  capture_docker_snapshot before-local-pbx

  restore_local_pbx() {
    if [ "$restore" != "1" ]; then
      return
    fi
    if [ "$initially_asterisk" = "1" ]; then
      run_gate "restore local FreeSWITCH down" "$freeswitch_dir/scripts/down.sh" || true
      run_gate "restore local Asterisk up" "$asterisk_dir/scripts/up.sh" || true
      capture_docker_snapshot after-restore
    elif [ "$initially_freeswitch" = "1" ]; then
      run_gate "restore local Asterisk down" "$asterisk_dir/scripts/down.sh" || true
      run_gate "restore local FreeSWITCH up" "$freeswitch_dir/scripts/up.sh" || true
      capture_docker_snapshot after-restore
    else
      run_gate "restore local Asterisk down" "$asterisk_dir/scripts/down.sh" || true
      run_gate "restore local FreeSWITCH down" "$freeswitch_dir/scripts/down.sh" || true
      capture_docker_snapshot after-restore
    fi
  }

  if pbx_provider_enabled asterisk; then
    run_gate "local FreeSWITCH down before Asterisk" "$freeswitch_dir/scripts/down.sh" || true
    if run_gate "local Asterisk up" "$asterisk_dir/scripts/up.sh"; then
      capture_docker_snapshot after-asterisk-up
      run_gate "local Asterisk PBX matrix" \
        env PBX_OUT_ROOT="$pbx_output_root" \
        PBX_REPORT_APPEND=1 \
        "$CRATE_DIR/examples/pbx/run.sh" \
        --pbx asterisk --api "$pbx_api" --scenario "$pbx_scenario" || true
      capture_docker_snapshot after-asterisk-matrix
    fi
    run_gate "local Asterisk down after matrix" "$asterisk_dir/scripts/down.sh" || true
    capture_docker_snapshot after-asterisk-down
  fi

  if pbx_provider_enabled freeswitch; then
    run_gate "local Asterisk down before FreeSWITCH" "$asterisk_dir/scripts/down.sh" || true
    if run_gate "local FreeSWITCH up" "$freeswitch_dir/scripts/up.sh"; then
      capture_docker_snapshot after-freeswitch-up
      run_gate "local FreeSWITCH PBX matrix" \
        env PBX_OUT_ROOT="$pbx_output_root" \
        PBX_REPORT_APPEND=1 \
        "$CRATE_DIR/examples/pbx/run.sh" \
        --pbx freeswitch --api "$pbx_api" --scenario "$pbx_scenario" || true
      capture_docker_snapshot after-freeswitch-matrix
    fi
    run_gate "local FreeSWITCH down after matrix" "$freeswitch_dir/scripts/down.sh" || true
    capture_docker_snapshot after-freeswitch-down
  fi

  restore_local_pbx
}

start_managed_sipp_target() {
  local host="${BETA_SIPP_TARGET_HOST:-127.0.0.1}"
  local port="${BETA_SIPP_TARGET_PORT:-35060}"
  local sipp_dir="$ARTIFACT_DIR/sipp"
  local log="$sipp_dir/rvoip_perf_listener.log"
  local started_at
  local start_epoch
  local duration
  mkdir -p "$sipp_dir"

  run_gate "SIPp standalone target build" cargo build -p rvoip-sip --release --example perf_listener

  echo
  echo "==> SIPp standalone target start"
  started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  start_epoch="$(date +%s)"
  local perf_profile="${BETA_SIPP_PERF_PROFILE:-pbx-media-server}"
  local recipe_file="${BETA_PERFORMANCE_RECIPE_FILE:-}"
  local listener_cmd=("$WORKSPACE_ROOT/target/release/examples/perf_listener" "$port" "$host" --diagnostics --perf-profile "$perf_profile")
  if [ -n "$recipe_file" ]; then
    listener_cmd+=(--recipe-file "$recipe_file")
  fi
  {
    echo "gate: SIPp standalone target start"
    echo "started_at_utc: $started_at"
    echo "workspace: $WORKSPACE_ROOT"
    echo "command: ${listener_cmd[*]}"
    echo
  } > "$log"
  "${listener_cmd[@]}" >> "$log" 2>&1 &
  SIPP_LISTENER_PID=$!
  for _ in $(seq 1 100); do
    if grep -q 'listening on' "$log" 2>/dev/null; then
      duration="$(($(date +%s) - start_epoch))s"
      record "PASS" "SIPp standalone target start" "$log" "$duration"
      BETA_SIPP_TARGET_HOST="$host"
      BETA_SIPP_TARGET_PORT="$port"
      export BETA_SIPP_TARGET_HOST BETA_SIPP_TARGET_PORT
      return 0
    fi
    if ! kill -0 "$SIPP_LISTENER_PID" >/dev/null 2>&1; then
      duration="$(($(date +%s) - start_epoch))s"
      record "FAIL" "SIPp standalone target start" "$log" "$duration"
      FAILURES=$((FAILURES + 1))
      echo "FAIL: SIPp standalone target exited before listening (see $log)" >&2
      return 1
    fi
    sleep 0.1
  done
  duration="$(($(date +%s) - start_epoch))s"
  record "FAIL" "SIPp standalone target start" "$log" "$duration"
  FAILURES=$((FAILURES + 1))
  echo "FAIL: SIPp standalone target did not become ready (see $log)" >&2
  return 1
}

stop_managed_sipp_target() {
  local log="$ARTIFACT_DIR/sipp/rvoip_perf_listener.log"
  local start_epoch
  local duration
  if [ -z "$SIPP_LISTENER_PID" ]; then
    return 0
  fi
  echo
  echo "==> SIPp standalone target stop"
  start_epoch="$(date +%s)"
  if kill -0 "$SIPP_LISTENER_PID" >/dev/null 2>&1; then
    kill -INT "$SIPP_LISTENER_PID" >/dev/null 2>&1 || true
    wait "$SIPP_LISTENER_PID" >/dev/null 2>&1 || true
  fi
  SIPP_LISTENER_PID=""
  duration="$(($(date +%s) - start_epoch))s"
  record "PASS" "SIPp standalone target stop" "$log" "$duration"
}

run_sipp_standalone_gate() {
  if [ "${BETA_RUN_SIPP:-1}" = "0" ]; then
    skip_gate "SIPp standalone matrix" "BETA_RUN_SIPP=0 disables required SIPp evidence."
    return
  fi
  if ! command -v "${SIPP_BIN:-sipp}" >/dev/null 2>&1; then
    run_gate "SIPp standalone matrix" bash -c "echo \"SIPp binary '${SIPP_BIN:-sipp}' not found on PATH\" >&2; exit 1"
    return
  fi

  local managed_target=0
  if [ -z "${BETA_SIPP_TARGET_HOST:-}" ] || [ -z "${BETA_SIPP_TARGET_PORT:-}" ]; then
    managed_target=1
    start_managed_sipp_target
  fi

  local cps="${BETA_SIPP_CPS:-30 100 300 1000 2000}"
  run_gate "SIPp standalone matrix" env \
    RVOIP_PERF_RESULTS="$ARTIFACT_DIR/sipp" \
    RVOIP_PERF_CPS="$cps" \
    RVOIP_PERF_MIN_SUCCESS_PCT="${RVOIP_PERF_MIN_SUCCESS_PCT:-99.9}" \
    "$CRATE_DIR/tests/perf/sipp_scenarios/run_comparison.sh" \
    "$BETA_SIPP_TARGET_HOST" "$BETA_SIPP_TARGET_PORT" rvoip

  if [ "$managed_target" = "1" ]; then
    stop_managed_sipp_target
  fi
}

run_proxy_descope_audit() {
  run_gate "Kamailio/OpenSIPS proxy de-scope audit" bash -c \
    "set -euo pipefail
     rg -q 'Kamailio/OpenSIPS.*planned validation targets, not release' crates/rvoip-sip/README.md
     rg -q 'Kamailio/OpenSIPS plus RTPengine.*Investigation' crates/rvoip-sip/docs/TOPOLOGY_PROFILES.md
     rg -q 'Kamailio/OpenSIPS.*Investigation only' crates/rvoip-sip/docs/INTEROP_CI_PLAN.md"
}

run_dependency_audit() {
  local security_dir="$ARTIFACT_DIR/security"
  mkdir -p "$security_dir"
  run_gate "dependency advisory audit" env SECURITY_DIR="$security_dir" bash -c '
    set -euo pipefail
    mkdir -p "$SECURITY_DIR"
    if ! cargo audit --version > "$SECURITY_DIR/cargo-audit-version.txt" 2>&1; then
      echo "cargo-audit is not available. Install it with: cargo install cargo-audit" >&2
      exit 127
    fi
    set +e
    cargo audit > "$SECURITY_DIR/cargo-audit.txt" 2>&1
    audit_status=$?
    cargo audit --json > "$SECURITY_DIR/cargo-audit.json" 2> "$SECURITY_DIR/cargo-audit-json.stderr"
    json_status=$?
    set -e
    cat "$SECURITY_DIR/cargo-audit.txt"
    if [ "$audit_status" -ne 0 ] || [ "$json_status" -ne 0 ]; then
      exit 1
    fi
  '
}

run_fuzz_smoke_target() {
  local target="$1"
  local fuzz_dir="$ARTIFACT_DIR/security/fuzz"
  mkdir -p "$fuzz_dir"
  run_gate "parser fuzz smoke ($target)" env \
    CRATE_DIR="$CRATE_DIR" \
    WORKSPACE_ROOT="$WORKSPACE_ROOT" \
    FUZZ_TARGET="$target" \
    FUZZ_LOG="$fuzz_dir/$target.log" \
    BETA_FUZZ_SMOKE_RUNS="${BETA_FUZZ_SMOKE_RUNS:-1000}" \
    BETA_FUZZ_SMOKE_SECONDS="${BETA_FUZZ_SMOKE_SECONDS:-10}" \
    BETA_FUZZ_TOOLCHAIN="${BETA_FUZZ_TOOLCHAIN:-nightly}" \
    bash -c '
      set -euo pipefail
      mkdir -p "$(dirname "$FUZZ_LOG")"
      if ! cargo +"$BETA_FUZZ_TOOLCHAIN" fuzz --version > "${FUZZ_LOG%.log}.version.txt" 2>&1; then
        echo "cargo-fuzz or Rust toolchain '$BETA_FUZZ_TOOLCHAIN' is not available." >&2
        echo "Install with: rustup toolchain install $BETA_FUZZ_TOOLCHAIN && cargo install cargo-fuzz" >&2
        exit 127
      fi
      cd "$CRATE_DIR"
      set +e
      CARGO_TARGET_DIR="$WORKSPACE_ROOT/target/fuzz" \
        cargo +"$BETA_FUZZ_TOOLCHAIN" fuzz run "$FUZZ_TARGET" -- \
          -runs="$BETA_FUZZ_SMOKE_RUNS" \
          -max_total_time="$BETA_FUZZ_SMOKE_SECONDS" \
          > "$FUZZ_LOG" 2>&1
      fuzz_status=$?
      set -e
      cat "$FUZZ_LOG"
      exit "$fuzz_status"
    '
}

run_fuzz_smoke_gates() {
  if [ "${BETA_RUN_FUZZ_SMOKE:-1}" = "0" ]; then
    skip_gate "parser fuzz smoke" "BETA_RUN_FUZZ_SMOKE=0 disables required parser fuzz-smoke evidence."
    return
  fi
  run_fuzz_smoke_target sip_message
  run_fuzz_smoke_target uri
  run_fuzz_smoke_target header
  run_fuzz_smoke_target sdp
}

run_security_gates() {
  run_dependency_audit || true
  run_fuzz_smoke_gates || true
}

run_local_gates() {
  run_gate "format check" cargo fmt --all -- --check
  run_gate "rvoip-sip all-target check" cargo check -p rvoip-sip --all-targets --features generated-validation,dev-insecure-tls
  run_gate "claimed lower-crate check" cargo check \
    -p rvoip-sip-core \
    -p rvoip-sip-transport \
    -p rvoip-sip-dialog \
    -p rvoip-media-core \
    -p rvoip-rtp-core \
    -p rvoip-auth-core \
    -p rvoip-sip-registrar \
    -p rvoip-sip-proxy \
    --all-targets
  run_gate "supporting SIP crate tests" cargo test \
    -p rvoip-auth-core \
    -p rvoip-sip-registrar \
    -p rvoip-sip-proxy \
    --all-targets
  run_gate "rvoip-sip unit tests" cargo test -p rvoip-sip --lib
  run_gate "rvoip-sip integration tests" cargo test -p rvoip-sip --tests --features generated-validation,dev-insecure-tls
  run_gate "rvoip-sip doctests" cargo test -p rvoip-sip --doc
  run_gate "rvoip-sip examples compile" cargo build -p rvoip-sip --examples --features dev-insecure-tls
  run_gate "rvoip-sip rustdoc" env RUSTDOCFLAGS="-D warnings" cargo doc -p rvoip-sip --no-deps --features generated-validation,dev-insecure-tls
  run_gate "sip-core RFC 4475 torture tests" cargo test -p rvoip-sip-core --features lenient_parsing --test torture_tests
  run_gate "sip-core generated message validation" cargo test -p rvoip-sip-core --features generated-validation --test generated_message_compliance
  run_gate "sip dialog generated validation" cargo test -p rvoip-sip-dialog --features generated-validation --test generated_sip_compliance
}

run_interop_gates() {
  if [ "${BETA_RUN_LOCAL_PBX:-0}" = "1" ]; then
    run_local_pbx_gate
  elif [ "${BETA_RUN_PBX:-0}" = "1" ]; then
    run_gate "PBX interop matrix" "$CRATE_DIR/examples/pbx/run.sh" --pbx both --api all --scenario all
  else
    skip_gate "PBX interop matrix" "Set BETA_RUN_LOCAL_PBX=1 for ~/Developer PBX lifecycle management, or BETA_RUN_PBX=1 after starting PBX containers yourself."
  fi

  run_sipp_standalone_gate

  if [ "${BETA_RUN_STRICT_UA:-1}" = "0" ]; then
    skip_gate "baresip strict-UA matrix" "BETA_RUN_STRICT_UA=0 disables required strict-UA evidence."
  else
    run_gate "baresip strict-UA matrix" env \
      RVOIP_STRICT_UA_RESULTS="$ARTIFACT_DIR/strict-ua" \
      "$CRATE_DIR/tests/interop/baresip/run_strict_ua.sh"
  fi

  run_proxy_descope_audit
}

run_perf_gates() {
  local profile_spec
  for profile_spec in ${BETA_PERF_PROFILE_MATRIX:-endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000}; do
    local profile="${profile_spec%%:*}"
    local cps="${profile_spec#*:}"
    local perf_env=(RVOIP_PERF_PROFILE="$profile" RVOIP_PERF_SWEEP_CPS="$cps")
    if [ -n "${BETA_PERFORMANCE_RECIPE_FILE:-}" ]; then
      perf_env+=(RVOIP_PERF_RECIPE_FILE="$BETA_PERFORMANCE_RECIPE_FILE")
    fi
    run_gate "perf call setup CPS ($profile)" env \
      "${perf_env[@]}" \
      cargo test -p rvoip-sip --release --features perf-tests --test perf_call_setup_cps -- --nocapture
  done
  run_gate "perf registration throughput" cargo test -p rvoip-sip --release --features perf-tests --test perf_registration_throughput -- --nocapture
  run_gate "perf concurrent active calls" cargo test -p rvoip-sip --release --features perf-tests --test perf_concurrent_active_calls -- --nocapture
  run_gate "perf RTP steady state" cargo test -p rvoip-sip --release --features perf-tests --test perf_rtp_steady_state -- --nocapture
  run_gate "perf backpressure step" cargo test -p rvoip-sip --release --features perf-tests --test perf_backpressure_step -- --nocapture
  run_gate "perf transport recovery" cargo test -p rvoip-sip --release --features perf-tests --test perf_transport_recovery -- --nocapture
  run_gate "perf session churn leak" cargo test -p rvoip-sip --release --features perf-tests --test perf_soak_30min perf_session_churn_leak -- --ignored --nocapture
  if [ "${BETA_RUN_LONG_SOAK:-1}" = "1" ]; then
    run_gate "perf soak candidate" cargo test -p rvoip-sip --release --features perf-tests --test perf_soak_30min perf_soak_30min -- --ignored --nocapture
  else
    skip_gate "perf soak" "BETA_RUN_LONG_SOAK=0 disables release-candidate soak evidence."
  fi
}

write_environment_report
write_summary_gate_table_header

case "$MODE" in
  local)
    run_local_gates
    ;;
  full)
    run_local_gates
    run_security_gates
    run_interop_gates
    run_perf_gates
    ;;
  interop)
    run_interop_gates
    ;;
  perf)
    run_perf_gates
    ;;
  security)
    run_security_gates
    ;;
  *)
    echo "Unknown mode: $MODE" >&2
    exit 2
    ;;
esac

cat >> "$SUMMARY" <<EOF

## Report Package

- enabled: \`${BETA_REPORT_PACKAGE:-1}\`
- report_dir: \`$(beta_report_run_dir)\`
- latest_pointer: \`$(beta_report_root)/latest.txt\`

## Result

- failures: $FAILURES
- skips: $SKIPS
EOF

package_beta_report

echo
echo "Summary: $SUMMARY"
if [ "${BETA_REPORT_PACKAGE:-1}" != "0" ]; then
  echo "Beta report: $(beta_report_run_dir)"
fi
if [ "$FAILURES" -ne 0 ]; then
  exit 1
fi
