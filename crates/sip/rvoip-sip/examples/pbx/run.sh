#!/usr/bin/env sh
# Unified PBX interop matrix runner.
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
WORKSPACE_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../../../.." && pwd)
OUT_ROOT="${PBX_OUT_ROOT:-$SCRIPT_DIR/output}"
RUN_STARTED_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)
RUN_STARTED_EPOCH=$(date +%s)
RUN_SUMMARY="$OUT_ROOT/summary.md"
RUN_MATRIX="$OUT_ROOT/matrix.tsv"
RUN_ENV="$OUT_ROOT/environment.md"
EXAMPLE_BIN_DIR="${PBX_EXAMPLE_BIN_DIR:-}"

PBX_ARG=${PBX_PROVIDER:-asterisk}
API_ARG=${PBX_API:-all}
SCENARIO_ARG=${PBX_SCENARIO:-all}
TRANSPORT_ARG=${PBX_TRANSPORT_FILTER:-all}
REPEAT_COUNT=${PBX_REPEAT:-1}
PBX_REUSE_TLS_CERT=${PBX_REUSE_TLS_CERT:-1}
PBX_RUN_WITH_CARGO=${PBX_RUN_WITH_CARGO:-0}
PBX_TLS_PREWARM=${PBX_TLS_PREWARM:-1}
if [ "${PBX_DIAG:-0}" = "1" ]; then
  STOP_ON_FAIL=${PBX_STOP_ON_FAIL:-0}
else
  STOP_ON_FAIL=${PBX_STOP_ON_FAIL:-1}
fi
RUN_FAILURES=0
DIAG_PCAP_PID=""
DIAG_SAMPLE_PID=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    --pbx|--provider)
      PBX_ARG=$2
      shift 2
      ;;
    --api)
      API_ARG=$2
      shift 2
      ;;
    --scenario)
      SCENARIO_ARG=$2
      shift 2
      ;;
    --transport)
      TRANSPORT_ARG=$2
      shift 2
      ;;
    --repeat)
      REPEAT_COUNT=$2
      shift 2
      ;;
    --stop-on-fail)
      STOP_ON_FAIL=$2
      shift 2
      ;;
    --help|-h)
      echo "Usage: $0 [--pbx asterisk|freeswitch|both] [--api endpoint|stream_peer|callback|all] [--scenario registration|basic_call|hold_resume|ring_cancel|dtmf|reject|blind_transfer|all] [--transport UDP|TLS|all] [--repeat N] [--stop-on-fail 0|1]"
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

case "$TRANSPORT_ARG" in
  all|udp|UDP|tls|TLS) ;;
  *) echo "Unknown transport: $TRANSPORT_ARG" >&2; exit 2 ;;
esac

case "$REPEAT_COUNT" in
  ''|*[!0-9]*) echo "--repeat requires a positive integer" >&2; exit 2 ;;
  0) echo "--repeat requires a positive integer" >&2; exit 2 ;;
esac

case "$STOP_ON_FAIL" in
  0|1) ;;
  *) echo "--stop-on-fail requires 0 or 1" >&2; exit 2 ;;
esac

# shellcheck disable=SC1091
. "$SCRIPT_DIR/tls_cert.sh"
RUN_ENV="$OUT_ROOT/environment-${PBX_ARG}.md"

PBX_CHILDREN=""
PBX_REPORT_READY=0

cleanup() {
  if [ -n "$DIAG_SAMPLE_PID" ]; then
    kill "$DIAG_SAMPLE_PID" 2>/dev/null || true
  fi
  if [ -n "$DIAG_PCAP_PID" ]; then
    kill "$DIAG_PCAP_PID" 2>/dev/null || true
  fi
  for pid in $PBX_CHILDREN; do
    kill "$pid" 2>/dev/null || true
  done
  wait 2>/dev/null || true
}

redacted_env() {
  env | LC_ALL=C sort | awk -F= '
    /^(PBX_|SIP_|TLS_|ASTERISK_|FREESWITCH_|RVOIP_|AUDIO_|IDLE_)/ {
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
  output=$1
  shift
  {
    echo "+ $*"
    "$@"
  } >"$output" 2>&1 || true
}

write_run_environment() {
  mkdir -p "$OUT_ROOT"
  {
    echo "# PBX Interop Environment"
    echo
    echo "- started_at_utc: $RUN_STARTED_UTC"
    echo "- workspace: $WORKSPACE_ROOT"
    echo "- output_root: $OUT_ROOT"
    echo "- pbx_arg: $PBX_ARG"
    echo "- api_arg: $API_ARG"
    echo "- scenario_arg: $SCENARIO_ARG"
    echo "- transport_arg: $TRANSPORT_ARG"
    echo "- repeat_count: $REPEAT_COUNT"
    echo "- stop_on_fail: $STOP_ON_FAIL"
    echo "- pbx_diag: ${PBX_DIAG:-0}"
    echo "- pbx_reuse_tls_cert: $PBX_REUSE_TLS_CERT"
    echo "- pbx_tls_prewarm: $PBX_TLS_PREWARM"
    echo "- pbx_run_with_cargo: $PBX_RUN_WITH_CARGO"
    echo "- example_bin_dir: $EXAMPLE_BIN_DIR"
    echo "- git_rev: $(git -C "$WORKSPACE_ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
    echo "- rustc: $(rustc --version 2>/dev/null || echo unknown)"
    echo "- cargo: $(cargo --version 2>/dev/null || echo unknown)"
    echo "- host: $(uname -a 2>/dev/null || echo unknown)"
    if command -v sipp >/dev/null 2>&1; then
      echo "- sipp: $(sipp -v 2>&1 | head -1)"
    else
      echo "- sipp: not found"
    fi
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

  capture_command "$OUT_ROOT/git-status.txt" git -C "$WORKSPACE_ROOT" status --short
}

init_report() {
  mkdir -p "$OUT_ROOT"
  if [ "${PBX_REPORT_APPEND:-0}" != "1" ] || [ ! -f "$RUN_MATRIX" ]; then
    printf 'status\tprovider\tapi\tscenario\ttransport\trole\tduration_s\texit_code\tstarted_at_utc\tended_at_utc\tlog\tout_dir\n' >"$RUN_MATRIX"
    printf 'provider\tapi\tscenario\ttransport\trole\tduration_s\texit_code\tstarted_at_utc\tended_at_utc\tlog\n' >"$OUT_ROOT/tls-prewarm.tsv"
  fi
  write_run_environment
  PBX_REPORT_READY=1
}

record_matrix() {
  status=$1
  provider=$2
  api=$3
  scenario=$4
  transport=$5
  role=$6
  duration=$7
  exit_code=$8
  started_at=$9
  ended_at=${10}
  log=${11}
  out_dir=${12}
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$status" "$provider" "$api" "$scenario" "$transport" "$role" \
    "$duration" "$exit_code" "$started_at" "$ended_at" "$log" "$out_dir" >>"$RUN_MATRIX"
}

write_run_summary() {
  exit_status=$1
  if [ "$PBX_REPORT_READY" != "1" ]; then
    return
  fi
  ended_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  duration=$(( $(date +%s) - RUN_STARTED_EPOCH ))
  pass_count=$(awk -F '\t' 'NR > 1 && $1 == "PASS" { n++ } END { print n + 0 }' "$RUN_MATRIX" 2>/dev/null || echo 0)
  fail_count=$(awk -F '\t' 'NR > 1 && $1 == "FAIL" { n++ } END { print n + 0 }' "$RUN_MATRIX" 2>/dev/null || echo 0)
  total_count=$(awk 'NR > 1 { n++ } END { print n + 0 }' "$RUN_MATRIX" 2>/dev/null || echo 0)

  {
    echo "# PBX Interop Run Summary"
    echo
    echo "- started_at_utc: $RUN_STARTED_UTC"
    echo "- ended_at_utc: $ended_at"
    echo "- duration_seconds: $duration"
    echo "- exit_status: $exit_status"
    echo "- output_root: $OUT_ROOT"
    echo "- environments: \`environment-*.md\`"
    echo "- matrix: \`matrix.tsv\`"
    echo
    echo "## Result"
    echo
    echo "- total_cells: $total_count"
    echo "- passed_cells: $pass_count"
    echo "- failed_cells: $fail_count"
    echo
    echo "## Matrix"
    echo
    echo "| Status | Provider | API | Scenario | Transport | Role | Duration | Exit | Log |"
    echo "|--------|----------|-----|----------|-----------|------|----------|------|-----|"
    awk -F '\t' 'NR > 1 {
      printf "| %s | %s | %s | %s | %s | %s | %ss | %s | `%s` |\n", $1, $2, $3, $4, $5, $6, $7, $8, $11
    }' "$RUN_MATRIX"
  } >"$RUN_SUMMARY"
}

finish() {
  status=$?
  trap - EXIT INT TERM
  cleanup
  if [ "$status" -eq 0 ] && [ -f "$RUN_MATRIX" ]; then
    failures=$(awk -F '\t' 'NR > 1 && $1 == "FAIL" { n++ } END { print n + 0 }' "$RUN_MATRIX" 2>/dev/null || echo 0)
    if [ "$failures" -gt 0 ]; then
      status=1
    fi
  fi
  write_run_summary "$status"
  exit "$status"
}

trap finish EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

pbx_list() {
  case "$PBX_ARG" in
    both|all) printf '%s\n' asterisk freeswitch ;;
    asterisk|ast) printf '%s\n' asterisk ;;
    freeswitch|free-switch|fs) printf '%s\n' freeswitch ;;
    *) echo "Unknown PBX: $PBX_ARG" >&2; exit 2 ;;
  esac
}

api_examples() {
  case "$API_ARG" in
    all) printf '%s\n' pbx_endpoint pbx_stream_peer pbx_callback_builder ;;
    endpoint) printf '%s\n' pbx_endpoint ;;
    stream_peer|peer|streampeer) printf '%s\n' pbx_stream_peer ;;
    callback|callback_builder) printf '%s\n' pbx_callback_builder ;;
    *) echo "Unknown API: $API_ARG" >&2; exit 2 ;;
  esac
}

scenario_list() {
  case "$SCENARIO_ARG" in
    all) printf '%s\n' registration basic_call hold_resume ring_cancel dtmf reject blind_transfer ;;
    basic|basic_call|call) printf '%s\n' basic_call ;;
    hold|hold_resume) printf '%s\n' hold_resume ;;
    ring|ring_cancel) printf '%s\n' ring_cancel ;;
    blind_transfer|transfer) printf '%s\n' blind_transfer ;;
    registration|dtmf|reject) printf '%s\n' "$SCENARIO_ARG" ;;
    *) echo "Unknown scenario: $SCENARIO_ARG" >&2; exit 2 ;;
  esac
}

load_provider_env() {
  provider=$1
  unset TLS_CERT_PATH TLS_KEY_PATH
  case "$provider" in
    freeswitch)
      unset SIP_SERVER SIP_PORT SIP_TLS_PORT SIP_PASSWORD TLS_CA_PATH
      unset ASTERISK_TLS_CONTACT_MODE ASTERISK_TLS_FLOW_REUSE ASTERISK_TLS_SRTP_REQUIRED
      ;;
    *)
      unset FREESWITCH_UDP_ADDR FREESWITCH_TLS_ADDR FREESWITCH_PASSWORD FREESWITCH_TRANSPORT
      unset FREESWITCH_TLS_CONTACT_MODE FREESWITCH_TLS_FLOW_REUSE FREESWITCH_TLS_SRTP_REQUIRED
      ;;
  esac
  if [ "$provider" = "asterisk" ] && [ -f "$HOME/Developer/asterisk/rvoip-local.env" ]; then
    set -a
    # shellcheck disable=SC1091
    . "$HOME/Developer/asterisk/rvoip-local.env"
    set +a
  fi
  if [ "$provider" = "freeswitch" ] && [ -f "$HOME/Developer/freeswitch/freeswitch-local.env" ]; then
    set -a
    # shellcheck disable=SC1091
    . "$HOME/Developer/freeswitch/freeswitch-local.env"
    set +a
  fi
  if [ -f "$SCRIPT_DIR/env/${provider}.env" ]; then
    set -a
    # shellcheck disable=SC1091
    . "$SCRIPT_DIR/env/${provider}.env"
    set +a
  fi
  if [ -f "$SCRIPT_DIR/.env.local" ]; then
    set -a
    # shellcheck disable=SC1091
    . "$SCRIPT_DIR/.env.local"
    set +a
  fi
}

example_label() {
  case "$1" in
    pbx_endpoint) printf '%s\n' endpoint ;;
    pbx_stream_peer) printf '%s\n' stream_peer ;;
    pbx_callback_builder) printf '%s\n' callback ;;
  esac
}

diag_enabled() {
  [ "${PBX_DIAG:-0}" = "1" ]
}

transport_selected() {
  transport=$1
  case "$TRANSPORT_ARG" in
    all) return 0 ;;
    udp|UDP) [ "$transport" = "UDP" ] ;;
    tls|TLS) [ "$transport" = "TLS" ] ;;
  esac
}

truthy() {
  case "$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')" in
    1|true|yes|on) return 0 ;;
    *) return 1 ;;
  esac
}

example_binary() {
  printf '%s/%s\n' "$EXAMPLE_BIN_DIR" "$1"
}

example_command_label() {
  example=$1
  if truthy "$PBX_RUN_WITH_CARGO"; then
    printf 'cargo run -p rvoip-sip --features dev-insecure-tls --example %s --quiet\n' "$example"
  else
    example_binary "$example"
  fi
}

run_example_command() {
  example=$1
  if truthy "$PBX_RUN_WITH_CARGO"; then
    cargo run -p rvoip-sip --features dev-insecure-tls --example "$example" --quiet
    return $?
  fi
  bin=$(example_binary "$example")
  if [ ! -x "$bin" ]; then
    echo "Built example binary not found or not executable: $bin" >&2
    echo "Set PBX_RUN_WITH_CARGO=1 to use cargo run as a fallback." >&2
    return 127
  fi
  "$bin"
}

resolve_example_bin_dir() {
  if [ -n "$EXAMPLE_BIN_DIR" ]; then
    return
  fi
  metadata=$(cargo metadata --format-version 1 --no-deps 2>/dev/null || true)
  target_dir=$(printf '%s\n' "$metadata" | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p' | head -1)
  if [ -z "$target_dir" ]; then
    target_dir="${CARGO_TARGET_DIR:-$WORKSPACE_ROOT/target}"
  fi
  case "$target_dir" in
    /*) ;;
    *) target_dir="$WORKSPACE_ROOT/$target_dir" ;;
  esac
  EXAMPLE_BIN_DIR="$target_dir/debug/examples"
}

iteration_out_dir() {
  base=$1
  if [ "$REPEAT_COUNT" -gt 1 ]; then
    printf '%s/repeat-%03d\n' "$base" "$PBX_REPEAT_INDEX"
  else
    printf '%s\n' "$base"
  fi
}

pbx_host_for_diag() {
  provider=$1
  transport=$2
  case "$provider:$transport" in
    freeswitch:TLS) printf '%s\n' "${FREESWITCH_TLS_ADDR%%:*}" ;;
    freeswitch:UDP) printf '%s\n' "${FREESWITCH_UDP_ADDR%%:*}" ;;
    *) printf '%s\n' "${SIP_SERVER:-127.0.0.1}" ;;
  esac
}

pbx_port_for_diag() {
  provider=$1
  transport=$2
  case "$provider:$transport" in
    freeswitch:TLS) printf '%s\n' "${FREESWITCH_TLS_ADDR##*:}" ;;
    freeswitch:UDP) printf '%s\n' "${FREESWITCH_UDP_ADDR##*:}" ;;
    *:TLS) printf '%s\n' "${SIP_TLS_PORT:-5061}" ;;
    *) printf '%s\n' "${SIP_PORT:-5060}" ;;
  esac
}

route_interface_for_host() {
  host=$1
  route -n get "$host" 2>/dev/null | awk '/interface:/{print $2; exit}'
}

fs_cli_capture() {
  output=$1
  command=$2
  if ! command -v docker >/dev/null 2>&1; then
    echo "docker not found" >"$output"
    return
  fi
  {
    echo "+ docker exec rvoip-freeswitch fs_cli -x \"$command\""
    docker exec rvoip-freeswitch fs_cli -x "$command"
  } >"$output" 2>&1 || true
}

diag_fs_snapshot() {
  dfs_provider=$1
  dfs_out_dir=$2
  dfs_label=$3
  if ! diag_enabled || [ "$dfs_provider" != "freeswitch" ]; then
    return
  fi
  dfs_snapshot="$dfs_out_dir/fs-cli-$dfs_label.txt"
  {
    echo "# FreeSWITCH fs_cli snapshot: $dfs_label"
    echo
    for command in \
      "status" \
      "show calls" \
      "show channels" \
      "show registrations" \
      "sofia status profile rvoip_tls_srtp"
    do
      echo
      echo "## $command"
      echo
      docker exec rvoip-freeswitch fs_cli -x "$command" 2>&1 || true
    done
  } >"$dfs_snapshot" 2>&1 || true
}

diag_fs_sample_loop() {
  dfsl_provider=$1
  dfsl_out_dir=$2
  if [ "$dfsl_provider" != "freeswitch" ]; then
    return
  fi
  dfsl_sample_dir="$dfsl_out_dir/fs-cli-samples"
  mkdir -p "$dfsl_sample_dir"
  while :; do
    dfsl_stamp=$(date -u +%Y%m%dT%H%M%SZ)
    diag_fs_snapshot "$dfsl_provider" "$dfsl_sample_dir" "$dfsl_stamp"
    sleep "${PBX_DIAG_FS_SAMPLE_SECS:-2}" || break
  done
}

diag_start_pcap() {
  provider=$1
  transport=$2
  out_dir=$3
  host=$(pbx_host_for_diag "$provider" "$transport")
  port=$(pbx_port_for_diag "$provider" "$transport")
  iface=$(route_interface_for_host "$host")
  if [ -z "$iface" ]; then
    iface=${PBX_DIAG_TCPDUMP_IFACE:-any}
  fi
  rtp_start=${FREESWITCH_RTP_START:-${ASTERISK_RTP_START:-16000}}
  rtp_end=${FREESWITCH_RTP_END:-${ASTERISK_RTP_END:-18100}}
  local_rtp_start=${PBX_DIAG_LOCAL_RTP_START:-16000}
  local_rtp_end=${PBX_DIAG_LOCAL_RTP_END:-18100}
  filter="host $host and (tcp port $port or udp port $port or udp portrange $rtp_start-$rtp_end or udp portrange $local_rtp_start-$local_rtp_end)"
  {
    echo "host=$host"
    echo "port=$port"
    echo "interface=$iface"
    echo "filter=$filter"
  } >"$out_dir/pcap-metadata.txt"
  if ! command -v tcpdump >/dev/null 2>&1; then
    echo "tcpdump not found" >"$out_dir/pcap.log"
    return
  fi
  tcpdump -i "$iface" -s 0 -n -w "$out_dir/cell.pcap" "$filter" >"$out_dir/pcap.log" 2>&1 &
  DIAG_PCAP_PID=$!
  sleep 1
}

diag_stop_pcap() {
  out_dir=$1
  if [ -n "$DIAG_PCAP_PID" ]; then
    kill "$DIAG_PCAP_PID" 2>/dev/null || true
    wait "$DIAG_PCAP_PID" 2>/dev/null || true
    DIAG_PCAP_PID=""
  fi
  if command -v tshark >/dev/null 2>&1 && [ -s "$out_dir/cell.pcap" ]; then
    {
      printf 'frame_time_epoch\tip_src\tudp_srcport\ttcp_srcport\tip_dst\tudp_dstport\ttcp_dstport\tprotocol\tinfo\n'
      tshark -r "$out_dir/cell.pcap" -T fields \
        -e frame.time_epoch \
        -e ip.src \
        -e udp.srcport \
        -e tcp.srcport \
        -e ip.dst \
        -e udp.dstport \
        -e tcp.dstport \
        -e _ws.col.Protocol \
        -e _ws.col.Info 2>"$out_dir/packet-timeline.stderr"
    } >"$out_dir/packet-timeline.tsv" || true
  else
    echo "tshark not available or cell.pcap missing/empty" >"$out_dir/packet-timeline.tsv"
  fi
}

diag_begin_cell() {
  provider=$1
  transport=$2
  out_dir=$3
  if ! diag_enabled; then
    return
  fi
  mkdir -p "$out_dir"
  export RUST_LOG="${RUST_LOG:-info,rvoip_sip=debug,rvoip_sip_dialog=debug,rvoip_sip_transport=debug,rvoip_sip_proxy=debug,rvoip_sip_registrar=debug}"
  DIAG_CELL_STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  {
    echo "# PBX Diagnostic Cell"
    echo
    echo "- started_at_utc: $DIAG_CELL_STARTED_AT"
    echo "- provider: $provider"
    echo "- transport: $transport"
    echo "- repeat_index: ${PBX_REPEAT_INDEX:-1}"
    echo "- rust_log: $RUST_LOG"
  } >"$out_dir/diag-metadata.md"
  diag_fs_snapshot "$provider" "$out_dir" before
  diag_fs_sample_loop "$provider" "$out_dir" &
  DIAG_SAMPLE_PID=$!
  diag_start_pcap "$provider" "$transport" "$out_dir"
}

diag_end_cell() {
  provider=$1
  transport=$2
  out_dir=$3
  if ! diag_enabled; then
    return
  fi
  if [ -n "$DIAG_SAMPLE_PID" ]; then
    kill "$DIAG_SAMPLE_PID" 2>/dev/null || true
    wait "$DIAG_SAMPLE_PID" 2>/dev/null || true
    DIAG_SAMPLE_PID=""
  fi
  diag_stop_pcap "$out_dir"
  diag_fs_snapshot "$provider" "$out_dir" after
  if [ "$provider" = "freeswitch" ] && command -v docker >/dev/null 2>&1; then
    docker logs --since "${DIAG_CELL_STARTED_AT:-0}" rvoip-freeswitch >"$out_dir/freeswitch-since-cell.log" 2>&1 || true
  fi
  {
    echo
    echo "- ended_at_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  } >>"$out_dir/diag-metadata.md"
}

run_one() {
  provider=$1
  example=$2
  scenario=$3
  transport=$4
  role=$5
  out_dir=$6
  log=$7
  api_label=$(example_label "$example")
  started_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  start_epoch=$(date +%s)
  status_label=PASS

  mkdir -p "$out_dir"
  {
    echo "# PBX Cell Metadata"
    echo
    echo "- provider: $provider"
    echo "- api: $api_label"
    echo "- scenario: $scenario"
    echo "- transport: $transport"
    echo "- role: $role"
    echo "- started_at_utc: $started_at"
    echo "- output_dir: $out_dir"
    echo "- log: $log"
    echo
    echo "## Command"
    echo
    echo '```sh'
    echo "PBX_PROVIDER=$provider PBX_SCENARIO=$scenario PBX_TRANSPORT=$transport SIP_TRANSPORT=$transport PBX_ROLE=$role AUDIO_OUTPUT_DIR=$out_dir $(example_command_label "$example")"
    echo '```'
    echo
    echo "## Redacted Environment"
    echo
    echo '```text'
    redacted_env
    echo '```'
  } >"$out_dir/${role}_metadata.md"

  {
    echo "provider: $provider"
    echo "api: $api_label"
    echo "scenario: $scenario"
    echo "transport: $transport"
    echo "role: $role"
    echo "started_at_utc: $started_at"
    echo
    echo "+ PBX_PROVIDER=$provider PBX_SCENARIO=$scenario PBX_TRANSPORT=$transport SIP_TRANSPORT=$transport PBX_ROLE=$role AUDIO_OUTPUT_DIR=$out_dir $(example_command_label "$example")"
  } >"$log"

  set +e
  (
    cd "$WORKSPACE_ROOT"
    export PBX_PROVIDER="$provider"
    export PBX_SCENARIO="$scenario"
    export PBX_TRANSPORT="$transport"
    export SIP_TRANSPORT="$transport"
    export PBX_ROLE="$role"
    export AUDIO_OUTPUT_DIR="$out_dir"
    run_example_command "$example"
  ) >>"$log" 2>&1
  rc=$?
  set -e

  ended_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  duration=$(( $(date +%s) - start_epoch ))
  if [ "$rc" -ne 0 ]; then
    status_label=FAIL
  fi
  {
    echo
    echo "ended_at_utc: $ended_at"
    echo "duration_seconds: $duration"
    echo "exit_status: $rc"
  } >>"$log"
  record_matrix "$status_label" "$provider" "$api_label" "$scenario" "$transport" "$role" "$duration" "$rc" "$started_at" "$ended_at" "$log" "$out_dir"
  return "$rc"
}

start_one() {
  provider=$1
  example=$2
  scenario=$3
  transport=$4
  role=$5
  out_dir=$6
  log=$7
  echo "[$provider/$(example_label "$example")/$transport/$scenario/$role] starting"
  run_one "$provider" "$example" "$scenario" "$transport" "$role" "$out_dir" "$log" &
  LAST_PID=$!
  PBX_CHILDREN="$PBX_CHILDREN $LAST_PID"
}

wait_for_log() {
  file=$1
  pattern=$2
  pid=$3
  label=$4
  limit=${5:-45}
  elapsed=0
  while [ "$elapsed" -lt "$limit" ]; do
    if grep -q "$pattern" "$file" 2>/dev/null; then
      return 0
    fi
    if ! kill -0 "$pid" 2>/dev/null; then
      echo "[$label] process exited before '$pattern' appeared"
      sed -n '1,160p' "$file" 2>/dev/null || true
      return 1
    fi
    sleep 1
    elapsed=$((elapsed + 1))
  done
  echo "[$label] timed out waiting for '$pattern'"
  sed -n '1,160p' "$file" 2>/dev/null || true
  return 1
}

wait_child() {
  pid=$1
  label=$2
  log=$3
  set +e
  wait "$pid"
  status=$?
  set -e
  if [ "$status" -ne 0 ]; then
    echo "[$label] failed with exit $status"
    sed -n '1,220p' "$log" 2>/dev/null || true
    return "$status"
  fi
}

prepare_tls() {
  provider=$1
  out_dir=$2
  export PBX_PROVIDER="$provider"
  export PBX_TRANSPORT=TLS
  export SIP_TRANSPORT=TLS
  case "$provider" in
    freeswitch)
      export TLS_INSECURE="${TLS_INSECURE:-1}"
      export FREESWITCH_TLS_CONTACT_MODE="${FREESWITCH_TLS_CONTACT_MODE:-reachable-contact}"
      export FREESWITCH_TLS_SRTP_REQUIRED="${FREESWITCH_TLS_SRTP_REQUIRED:-1}"
      ;;
    *)
      export ASTERISK_TLS_CONTACT_MODE="${ASTERISK_TLS_CONTACT_MODE:-reachable-contact}"
      export ASTERISK_TLS_SRTP_REQUIRED="${ASTERISK_TLS_SRTP_REQUIRED:-1}"
      ;;
  esac
  if truthy "$PBX_REUSE_TLS_CERT"; then
    tls_cert_dir="${PBX_TLS_CERT_ROOT:-$OUT_ROOT/tls}/$provider"
  else
    tls_cert_dir="$out_dir/tls"
  fi
  ensure_pbx_tls_listener_cert "$tls_cert_dir"
}

wait_for_pbx_tls_ready() {
  provider=$1
  out_dir=$2
  host=$(pbx_host_for_diag "$provider" TLS)
  port=$(pbx_port_for_diag "$provider" TLS)
  ready_log="$out_dir/tls-ready.log"
  attempts=${PBX_TLS_READY_ATTEMPTS:-20}
  sleep_secs=${PBX_TLS_READY_SLEEP_SECS:-1}
  mkdir -p "$out_dir"
  {
    echo "# PBX TLS readiness"
    echo
    echo "- provider: $provider"
    echo "- host: $host"
    echo "- port: $port"
    echo "- attempts: $attempts"
    echo "- sleep_secs: $sleep_secs"
  } >"$ready_log"

  i=1
  while [ "$i" -le "$attempts" ]; do
    nc_rc=1
    openssl_rc=1
    {
      echo
      echo "## attempt $i"
      if [ "$provider" = "freeswitch" ] && command -v docker >/dev/null 2>&1; then
        docker exec rvoip-freeswitch fs_cli -x "sofia status profile rvoip_tls_srtp" 2>&1 | sed -n '1,80p'
      fi
      if command -v nc >/dev/null 2>&1; then
        nc -z -w 2 "$host" "$port"
        nc_rc=$?
        echo "nc_rc=$nc_rc"
      else
        nc_rc=0
        echo "nc not found; skipping TCP socket probe"
      fi
      if command -v openssl >/dev/null 2>&1; then
        printf '' | openssl s_client -connect "$host:$port" -servername "$host" -brief 2>&1 | sed -n '1,80p'
        openssl_rc=$?
        echo "openssl_rc=$openssl_rc"
      else
        openssl_rc=0
        echo "openssl not found; skipping TLS handshake probe"
      fi
    } >>"$ready_log" 2>&1
    if [ "$nc_rc" -eq 0 ]; then
      echo "ready_at_attempt=$i" >>"$ready_log"
      return 0
    fi
    sleep "$sleep_secs"
    i=$((i + 1))
  done
  echo "PBX TLS socket was not ready at $host:$port after $attempts attempts; see $ready_log" >&2
  return 1
}

run_prewarm_one() {
  provider=$1
  example=$2
  role=$3
  out_dir=$4
  log="$out_dir/$role.log"
  api_label=$(example_label "$example")
  started_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  start_epoch=$(date +%s)
  mkdir -p "$out_dir"
  {
    echo "provider: $provider"
    echo "api: $api_label"
    echo "scenario: tls_prewarm"
    echo "transport: TLS"
    echo "role: $role"
    echo "started_at_utc: $started_at"
    echo
    echo "+ PBX_PROVIDER=$provider PBX_SCENARIO=registration PBX_TRANSPORT=TLS SIP_TRANSPORT=TLS PBX_ROLE=$role IDLE_SECS=${PBX_TLS_PREWARM_IDLE_SECS:-0} AUDIO_OUTPUT_DIR=$out_dir $(example_command_label "$example")"
  } >"$log"

  set +e
  (
    cd "$WORKSPACE_ROOT"
    export PBX_PROVIDER="$provider"
    export PBX_SCENARIO=registration
    export PBX_TRANSPORT=TLS
    export SIP_TRANSPORT=TLS
    export PBX_ROLE="$role"
    export IDLE_SECS="${PBX_TLS_PREWARM_IDLE_SECS:-0}"
    export AUDIO_OUTPUT_DIR="$out_dir"
    run_example_command "$example"
  ) >>"$log" 2>&1
  rc=$?
  set -e

  ended_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  duration=$(( $(date +%s) - start_epoch ))
  {
    echo
    echo "ended_at_utc: $ended_at"
    echo "duration_seconds: $duration"
    echo "exit_status: $rc"
  } >>"$log"
  printf '%s\t%s\t%s\tTLS\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$provider" "$api_label" tls_prewarm "$role" "$duration" "$rc" "$started_at" "$ended_at" "$log" >>"$OUT_ROOT/tls-prewarm.tsv"
  return "$rc"
}

prewarm_tls() {
  provider=$1
  example=$2
  if ! transport_selected TLS || ! truthy "$PBX_TLS_PREWARM"; then
    return 0
  fi
  api_label=$(example_label "$example")
  out_dir="$OUT_ROOT/_prewarm/$provider/$api_label/TLS"
  rm -rf "$out_dir"
  mkdir -p "$out_dir"
  prepare_tls "$provider" "$out_dir"
  wait_for_pbx_tls_ready "$provider" "$out_dir"
  for role in registration transferee target; do
    run_prewarm_one "$provider" "$example" "$role" "$out_dir" || return $?
  done
}

run_analyze() {
  provider=$1
  scenario=$2
  transport=$3
  out_dir=$4
  log="$out_dir/analyze.log"
  started_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  start_epoch=$(date +%s)
  status_label=PASS
  {
    echo "provider: $provider"
    echo "api: analyzer"
    echo "scenario: $scenario"
    echo "transport: $transport"
    echo "role: analyze"
    echo "started_at_utc: $started_at"
    echo
    echo "+ PBX_PROVIDER=$provider PBX_SCENARIO=$scenario PBX_TRANSPORT=$transport SIP_TRANSPORT=$transport AUDIO_OUTPUT_DIR=$out_dir $(example_command_label pbx_analyze)"
  } >"$log"
  set +e
  (
    cd "$WORKSPACE_ROOT"
    export PBX_PROVIDER="$provider"
    export PBX_SCENARIO="$scenario"
    export PBX_TRANSPORT="$transport"
    export SIP_TRANSPORT="$transport"
    export AUDIO_OUTPUT_DIR="$out_dir"
    run_example_command pbx_analyze
  ) >>"$log" 2>&1
  rc=$?
  set -e
  ended_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  duration=$(( $(date +%s) - start_epoch ))
  if [ "$rc" -ne 0 ]; then
    status_label=FAIL
  fi
  {
    echo
    echo "ended_at_utc: $ended_at"
    echo "duration_seconds: $duration"
    echo "exit_status: $rc"
  } >>"$log"
  record_matrix "$status_label" "$provider" analyzer "$scenario" "$transport" analyze "$duration" "$rc" "$started_at" "$ended_at" "$log" "$out_dir"
  return "$rc"
}

run_registration() {
  provider=$1
  example=$2
  api_label=$(example_label "$example")
  old_idle=${IDLE_SECS-}
  export IDLE_SECS="${REGISTRATION_IDLE_SECS:-2}"
  rc=0
  for transport in TLS UDP; do
    if ! transport_selected "$transport"; then
      continue
    fi
    out_dir="$OUT_ROOT/$provider/$api_label/registration/$transport"
    out_dir=$(iteration_out_dir "$out_dir")
    if [ "$transport" = "TLS" ]; then
      prepare_tls "$provider" "$out_dir"
    fi
    diag_begin_cell "$provider" "$transport" "$out_dir"
    run_one "$provider" "$example" registration "$transport" registration "$out_dir" "$out_dir/registration.log" || {
      rc=$?
      diag_end_cell "$provider" "$transport" "$out_dir"
      break
    }
    diag_end_cell "$provider" "$transport" "$out_dir"
  done
  if [ -n "$old_idle" ]; then
    export IDLE_SECS="$old_idle"
  else
    unset IDLE_SECS
  fi
  return "$rc"
}

run_two_party() {
  provider=$1
  example=$2
  scenario=$3
  transport=$4
  api_label=$(example_label "$example")
  out_dir="$OUT_ROOT/$provider/$api_label/$scenario/$transport"
  out_dir=$(iteration_out_dir "$out_dir")
  rm -rf "$out_dir"
  mkdir -p "$out_dir"
  if [ "$transport" = "TLS" ]; then
    prepare_tls "$provider" "$out_dir"
  fi
  diag_begin_cell "$provider" "$transport" "$out_dir"

  rc=0
  case "$scenario" in
    basic_call|hold_resume|dtmf|reject)
      start_one "$provider" "$example" "$scenario" "$transport" callee "$out_dir" "$out_dir/callee.log"
      pid_a=$LAST_PID
      wait_for_log "$out_dir/callee.log" "Registered." "$pid_a" "$scenario-callee" || rc=$?
      if [ "$rc" -eq 0 ]; then
        run_one "$provider" "$example" "$scenario" "$transport" caller "$out_dir" "$out_dir/caller.log" || rc=$?
      fi
      wait_child "$pid_a" "$scenario-callee" "$out_dir/callee.log" || {
        child_rc=$?
        if [ "$rc" -eq 0 ]; then rc=$child_rc; fi
      }
      ;;
    ring_cancel)
      start_one "$provider" "$example" "$scenario" "$transport" target "$out_dir" "$out_dir/target.log"
      pid_a=$LAST_PID
      wait_for_log "$out_dir/target.log" "Registered." "$pid_a" "$scenario-target" || rc=$?
      if [ "$rc" -eq 0 ]; then
        run_one "$provider" "$example" "$scenario" "$transport" caller "$out_dir" "$out_dir/caller.log" || rc=$?
      fi
      wait_child "$pid_a" "$scenario-target" "$out_dir/target.log" || {
        child_rc=$?
        if [ "$rc" -eq 0 ]; then rc=$child_rc; fi
      }
      ;;
  esac

  case "$scenario" in
    hold_resume|dtmf)
      if [ "$rc" -eq 0 ]; then
        run_analyze "$provider" "$scenario" "$transport" "$out_dir" || rc=$?
      fi
      ;;
  esac
  diag_end_cell "$provider" "$transport" "$out_dir"
  return "$rc"
}

run_transfer() {
  provider=$1
  example=$2
  transport=$3
  api_label=$(example_label "$example")
  scenario=blind_transfer
  out_dir="$OUT_ROOT/$provider/$api_label/$scenario/$transport"
  out_dir=$(iteration_out_dir "$out_dir")
  rm -rf "$out_dir"
  mkdir -p "$out_dir"
  if [ "$transport" = "TLS" ]; then
    prepare_tls "$provider" "$out_dir"
  fi
  diag_begin_cell "$provider" "$transport" "$out_dir"

  rc=0
  start_one "$provider" "$example" "$scenario" "$transport" transferee "$out_dir" "$out_dir/transferee.log"
  pid_a=$LAST_PID
  wait_for_log "$out_dir/transferee.log" "Registered." "$pid_a" transfer-transferee || rc=$?
  if [ "$rc" -eq 0 ]; then
    start_one "$provider" "$example" "$scenario" "$transport" target "$out_dir" "$out_dir/target.log"
    pid_b=$LAST_PID
    wait_for_log "$out_dir/target.log" "Registered." "$pid_b" transfer-target || rc=$?
  else
    pid_b=""
  fi
  if [ "$rc" -eq 0 ]; then
    run_one "$provider" "$example" "$scenario" "$transport" transferor "$out_dir" "$out_dir/transferor.log" || rc=$?
  fi
  wait_child "$pid_a" transfer-transferee "$out_dir/transferee.log" || {
    child_rc=$?
    if [ "$rc" -eq 0 ]; then rc=$child_rc; fi
  }
  if [ -n "$pid_b" ]; then
    wait_child "$pid_b" transfer-target "$out_dir/target.log" || {
      child_rc=$?
      if [ "$rc" -eq 0 ]; then rc=$child_rc; fi
    }
  fi
  if [ "$rc" -eq 0 ]; then
    run_analyze "$provider" "$scenario" "$transport" "$out_dir" || rc=$?
  fi
  diag_end_cell "$provider" "$transport" "$out_dir"
  return "$rc"
}

run_matrix_cell() {
  provider=$1
  example=$2
  scenario=$3
  rc=0
  case "$scenario" in
    registration)
      run_registration "$provider" "$example" || rc=$?
      ;;
    basic_call)
      if transport_selected UDP; then
        run_two_party "$provider" "$example" basic_call UDP || rc=$?
      elif transport_selected TLS; then
        run_two_party "$provider" "$example" basic_call TLS || rc=$?
      fi
      ;;
    hold_resume|ring_cancel|dtmf|reject)
      if transport_selected UDP; then
        run_two_party "$provider" "$example" "$scenario" UDP || rc=$?
        if [ "$rc" -ne 0 ] && [ "$STOP_ON_FAIL" = "1" ]; then return "$rc"; fi
      fi
      if transport_selected TLS; then
        run_two_party "$provider" "$example" "$scenario" TLS || {
          tls_rc=$?
          if [ "$rc" -eq 0 ]; then rc=$tls_rc; fi
        }
      fi
      ;;
    blind_transfer)
      if transport_selected UDP; then
        run_transfer "$provider" "$example" UDP || rc=$?
        if [ "$rc" -ne 0 ] && [ "$STOP_ON_FAIL" = "1" ]; then return "$rc"; fi
      fi
      if transport_selected TLS; then
        run_transfer "$provider" "$example" TLS || {
          tls_rc=$?
          if [ "$rc" -eq 0 ]; then rc=$tls_rc; fi
        }
      fi
      ;;
  esac
  return "$rc"
}

cd "$WORKSPACE_ROOT"
resolve_example_bin_dir
init_report
echo "Building unified PBX examples..."
echo "PBX output root: $OUT_ROOT"
cargo build -p rvoip-sip --features dev-insecure-tls \
  --example pbx_endpoint \
  --example pbx_stream_peer \
  --example pbx_callback_builder \
  --example pbx_analyze

for provider in $(pbx_list); do
  load_provider_env "$provider"
  for example in $(api_examples); do
    prewarm_tls "$provider" "$example"
    for scenario in $(scenario_list); do
      for repeat in $(seq 1 "$REPEAT_COUNT"); do
        export PBX_REPEAT_INDEX="$repeat"
        echo
        echo "========================================================================"
        if [ "$REPEAT_COUNT" -gt 1 ]; then
          echo "== $provider / $(example_label "$example") / $scenario / repeat $repeat/$REPEAT_COUNT"
        else
          echo "== $provider / $(example_label "$example") / $scenario"
        fi
        echo "========================================================================"
        run_matrix_cell "$provider" "$example" "$scenario" || {
          rc=$?
          RUN_FAILURES=1
          if [ "$STOP_ON_FAIL" = "1" ]; then
            exit "$rc"
          fi
        }
      done
    done
  done
done

echo
echo "========================================================================"
echo "== Unified PBX interop sequence complete"
echo "========================================================================"
