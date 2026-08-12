#!/usr/bin/env sh
# Unified PBX interop matrix runner.
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
WORKSPACE_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../../../../.." && pwd)

# Local PBX interop uses Docker through Colima on macOS. Homebrew installs the
# CLIs outside the minimal PATH that some CI/desktop shells provide.
PATH="/opt/homebrew/opt/docker/bin:/opt/homebrew/opt/docker-compose/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
export PATH

OUT_ROOT="${PBX_OUT_ROOT:-$SCRIPT_DIR/output}"
RUN_STARTED_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)
RUN_STARTED_EPOCH=$(date +%s)
RUN_SUMMARY="$OUT_ROOT/summary.md"
RUN_MATRIX="$OUT_ROOT/matrix.tsv"
RUN_ENV="$OUT_ROOT/environment.md"
EXAMPLE_BIN_DIR="${PBX_EXAMPLE_BIN_DIR:-}"
if [ -n "${RVOIP_PBX_LOCAL_ENV_ROOT:-}" ]; then
  LOCAL_ENV_ROOT=$RVOIP_PBX_LOCAL_ENV_ROOT
elif [ -n "${HOME:-}" ]; then
  LOCAL_ENV_ROOT=$HOME/Developer
else
  LOCAL_ENV_ROOT=$WORKSPACE_ROOT/target/release-interop/local-env
fi

PBX_ARG=${PBX_PROVIDER:-asterisk}
API_ARG=${PBX_API:-all}
SCENARIO_ARG=${PBX_SCENARIO:-all}
TRANSPORT_ARG=${PBX_TRANSPORT_FILTER:-all}
REPEAT_COUNT=${PBX_REPEAT:-1}
PBX_REUSE_TLS_CERT=${PBX_REUSE_TLS_CERT:-1}
PBX_RUN_WITH_CARGO=${PBX_RUN_WITH_CARGO:-0}
# `amr` is in the default set so the harness can actually run every
# scenario it advertises: without it an amr_call build compiles the AMR arms
# out and the scenario fails for a reason that has nothing to do with interop.
PBX_CARGO_FEATURES=${PBX_CARGO_FEATURES:-dev-insecure-tls,g729,amr}
PBX_G729_PROFILES="${PBX_G729_PROFILES:-g729a g729ab}"
PBX_TLS_PREWARM=${PBX_TLS_PREWARM:-1}
if [ "${PBX_DIAG:-0}" = "1" ]; then
  STOP_ON_FAIL=${PBX_STOP_ON_FAIL:-0}
else
  STOP_ON_FAIL=${PBX_STOP_ON_FAIL:-1}
fi
RUN_FAILURES=0
RUN_INITIAL_FAILURES=0
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
      echo "Usage: $0 [--pbx asterisk|freeswitch|kamailio|opensips|proxies|both] [--api endpoint|stream_peer|callback|all] [--scenario registration|basic_call|g729_call|amr_call|amr_transcode_call|b2bua_call|hold_resume|ring_cancel|dtmf|reject|blind_transfer|all] [--transport UDP|TLS|all] [--repeat N] [--stop-on-fail 0|1]"
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

# The provider env files are sourced with `set -a` *after* process env, so a
# value in a file would silently override one given on the command line. For
# the AMR probe knobs that inversion is exactly wrong -- a gate pins
# PBX_ASSUME_AMR and a lab file pins PBX_REQUIRE_AMR, and an operator must be
# able to override either for one run. Snapshot what the invocation provided;
# load_provider_env restores it after sourcing.
if [ "${PBX_ASSUME_AMR+x}" = "x" ]; then
  PBX_ASSUME_AMR_INVOKED=$PBX_ASSUME_AMR
  PBX_ASSUME_AMR_INVOKED_SET=1
else
  PBX_ASSUME_AMR_INVOKED=""
  PBX_ASSUME_AMR_INVOKED_SET=0
fi
if [ "${PBX_REQUIRE_AMR+x}" = "x" ]; then
  PBX_REQUIRE_AMR_INVOKED=$PBX_REQUIRE_AMR
  PBX_REQUIRE_AMR_INVOKED_SET=1
else
  PBX_REQUIRE_AMR_INVOKED=""
  PBX_REQUIRE_AMR_INVOKED_SET=0
fi

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
    /^(PBX_|SIP_|TLS_|ASTERISK_|FREESWITCH_|KAMAILIO_|OPENSIPS_|RVOIP_|AUDIO_|IDLE_)/ {
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
    echo "- pbx_cargo_features: $PBX_CARGO_FEATURES"
    echo "- pbx_g729_profiles: $PBX_G729_PROFILES"
    echo "- pbx_assume_amr: ${PBX_ASSUME_AMR:-unset}"
    echo "- pbx_require_amr: ${PBX_REQUIRE_AMR:-unset}"
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
    printf 'status\tprovider\tapi\tscenario\ttransport\trole\tduration_s\texit_code\tstarted_at_utc\tended_at_utc\tlog\tout_dir\tcodec\n' >"$RUN_MATRIX"
    printf 'provider\tapi\tscenario\ttransport\trole\tduration_s\texit_code\tstarted_at_utc\tended_at_utc\tlog\n' >"$OUT_ROOT/tls-prewarm.tsv"
  fi
  RUN_INITIAL_FAILURES=$(awk -F '\t' 'NR > 1 && $1 == "FAIL" { n++ } END { print n + 0 }' "$RUN_MATRIX" 2>/dev/null || echo 0)
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
  # Appended last on purpose: the summary awk reads $11/$12 positionally and
  # archived matrices are parsed by column index.
  codec=${13:-}
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$status" "$provider" "$api" "$scenario" "$transport" "$role" \
    "$duration" "$exit_code" "$started_at" "$ended_at" "$log" "$out_dir" "$codec" >>"$RUN_MATRIX"
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
  skip_count=$(awk -F '\t' 'NR > 1 && $1 == "SKIP" { n++ } END { print n + 0 }' "$RUN_MATRIX" 2>/dev/null || echo 0)
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
    echo "- skipped_cells: $skip_count"
    echo
    echo "## Matrix"
    echo
    echo "| Status | Provider | API | Scenario | Codec | Transport | Role | Duration | Exit | Log |"
    echo "|--------|----------|-----|----------|-------|-----------|------|----------|------|-----|"
    awk -F '\t' 'NR > 1 {
      printf "| %s | %s | %s | %s | %s | %s | %s | %ss | %s | `%s` |\n", $1, $2, $3, $4, $13, $5, $6, $7, $8, $11
    }' "$RUN_MATRIX"
  } >"$RUN_SUMMARY"
}

finish() {
  status=$?
  trap - EXIT INT TERM
  cleanup
  if [ "$status" -eq 0 ] && [ -f "$RUN_MATRIX" ]; then
    failures=$(awk -F '\t' 'NR > 1 && $1 == "FAIL" { n++ } END { print n + 0 }' "$RUN_MATRIX" 2>/dev/null || echo 0)
    if [ "$failures" -gt "$RUN_INITIAL_FAILURES" ]; then
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
    # `both`/`all` deliberately stay asterisk+freeswitch: the release gates
    # and the beta gate's BETA_PBX_PROVIDER default pass them, and growing
    # their matrix silently would change what a release run means. The proxy
    # providers are opt-in by name.
    both|all) printf '%s\n' asterisk freeswitch ;;
    asterisk|ast) printf '%s\n' asterisk ;;
    freeswitch|free-switch|fs) printf '%s\n' freeswitch ;;
    kamailio|kam) printf '%s\n' kamailio ;;
    opensips|open-sips|osips) printf '%s\n' opensips ;;
    proxies) printf '%s\n' kamailio opensips ;;
    *) echo "Unknown PBX: $PBX_ARG" >&2; exit 2 ;;
  esac
}

# What each provider can actually run.
#
# The proxy labs have no TLS listener yet, and a proxy cannot transcode --
# amr_transcode_call's disjoint-codec legs can never negotiate through one.
# The rest of the scenarios are plausible but unproven through a
# Record-Route proxy (re-INVITE hold, REFER transfer), so they stay gated
# until proven; PBX_PROXY_ALL_SCENARIOS=1 lifts the gate for exploration.
provider_supports_tls() {
  case "$1" in
    kamailio|opensips) return 1 ;;
    *) return 0 ;;
  esac
}

provider_scenario_supported() {
  pss_provider=$1
  pss_scenario=$2
  case "$pss_provider" in
    kamailio|opensips)
      case "$pss_scenario" in
        registration|basic_call|amr_call) return 0 ;;
        amr_transcode_call) return 1 ;;
        *)
          case "${PBX_PROXY_ALL_SCENARIOS:-0}" in
            1|true|yes|on) return 0 ;;
            *) return 1 ;;
          esac
          ;;
      esac
      ;;
    *) return 0 ;;
  esac
}

# UDP/TLS cell gate combining the user's --transport with provider ability.
cell_transport_enabled() {
  cte_provider=$1
  cte_transport=$2
  transport_selected "$cte_transport" || return 1
  if [ "$cte_transport" = "TLS" ]; then
    provider_supports_tls "$cte_provider" || return 1
  fi
  return 0
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
    all) printf '%s\n' registration basic_call g729_call amr_call amr_transcode_call b2bua_call hold_resume ring_cancel dtmf reject blind_transfer ;;
    amr|amr_call) printf '%s\n' amr_call ;;
    amr_transcode|amr_transcode_call|transcode) printf '%s\n' amr_transcode_call ;;
    b2bua|b2bua_call) printf '%s\n' b2bua_call ;;
    basic|basic_call|call) printf '%s\n' basic_call ;;
    g729|g729_call|g729ab|g729ab_call) printf '%s\n' g729_call ;;
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
      unset KAMAILIO_UDP_ADDR KAMAILIO_PASSWORD OPENSIPS_UDP_ADDR OPENSIPS_PASSWORD
      ;;
    kamailio)
      unset SIP_SERVER SIP_PORT SIP_TLS_PORT SIP_PASSWORD TLS_CA_PATH
      unset ASTERISK_TLS_CONTACT_MODE ASTERISK_TLS_FLOW_REUSE ASTERISK_TLS_SRTP_REQUIRED
      unset FREESWITCH_UDP_ADDR FREESWITCH_TLS_ADDR FREESWITCH_PASSWORD FREESWITCH_TRANSPORT
      unset FREESWITCH_TLS_CONTACT_MODE FREESWITCH_TLS_FLOW_REUSE FREESWITCH_TLS_SRTP_REQUIRED
      unset OPENSIPS_UDP_ADDR OPENSIPS_PASSWORD
      ;;
    opensips)
      unset SIP_SERVER SIP_PORT SIP_TLS_PORT SIP_PASSWORD TLS_CA_PATH
      unset ASTERISK_TLS_CONTACT_MODE ASTERISK_TLS_FLOW_REUSE ASTERISK_TLS_SRTP_REQUIRED
      unset FREESWITCH_UDP_ADDR FREESWITCH_TLS_ADDR FREESWITCH_PASSWORD FREESWITCH_TRANSPORT
      unset FREESWITCH_TLS_CONTACT_MODE FREESWITCH_TLS_FLOW_REUSE FREESWITCH_TLS_SRTP_REQUIRED
      unset KAMAILIO_UDP_ADDR KAMAILIO_PASSWORD
      ;;
    *)
      unset FREESWITCH_UDP_ADDR FREESWITCH_TLS_ADDR FREESWITCH_PASSWORD FREESWITCH_TRANSPORT
      unset FREESWITCH_TLS_CONTACT_MODE FREESWITCH_TLS_FLOW_REUSE FREESWITCH_TLS_SRTP_REQUIRED
      unset KAMAILIO_UDP_ADDR KAMAILIO_PASSWORD OPENSIPS_UDP_ADDR OPENSIPS_PASSWORD
      ;;
  esac
  if [ "$provider" = "asterisk" ] && [ -f "$LOCAL_ENV_ROOT/asterisk/rvoip-local.env" ]; then
    set -a
    # shellcheck disable=SC1091
    . "$LOCAL_ENV_ROOT/asterisk/rvoip-local.env"
    set +a
  fi
  if [ "$provider" = "freeswitch" ] && [ -f "$LOCAL_ENV_ROOT/freeswitch/freeswitch-local.env" ]; then
    set -a
    # shellcheck disable=SC1091
    . "$LOCAL_ENV_ROOT/freeswitch/freeswitch-local.env"
    set +a
  fi
  if [ "$provider" = "kamailio" ] && [ -f "$LOCAL_ENV_ROOT/kamailio/kamailio-local.env" ]; then
    set -a
    # shellcheck disable=SC1091
    . "$LOCAL_ENV_ROOT/kamailio/kamailio-local.env"
    set +a
  fi
  if [ "$provider" = "opensips" ] && [ -f "$LOCAL_ENV_ROOT/opensips/opensips-local.env" ]; then
    set -a
    # shellcheck disable=SC1091
    . "$LOCAL_ENV_ROOT/opensips/opensips-local.env"
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
  if [ "$PBX_ASSUME_AMR_INVOKED_SET" = "1" ]; then
    export PBX_ASSUME_AMR="$PBX_ASSUME_AMR_INVOKED"
  fi
  if [ "$PBX_REQUIRE_AMR_INVOKED_SET" = "1" ]; then
    export PBX_REQUIRE_AMR="$PBX_REQUIRE_AMR_INVOKED"
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

codec_profile_for_scenario() {
  cpfs_provider=$1
  scenario=$2
  if [ -n "${PBX_CODEC_PROFILE:-}" ]; then
    printf '%s\n' "$PBX_CODEC_PROFILE"
    return
  fi
  case "$scenario" in
    g729_call) printf '%s\n' g729ab ;;
    amr_call)
      # FreeSWITCH's rvoip profiles relay without re-framing and its
      # outbound leg is bandwidth-efficient, so octet-aligned AMR cannot
      # work there end to end; the default must be a profile that can.
      if [ "$cpfs_provider" = "freeswitch" ]; then
        printf '%s\n' amrnb_be
      else
        printf '%s\n' amrnb
      fi
      ;;
    b2bua_call)
      # The exit criterion's AMR-WB, framed per what the PBX can relay.
      if [ "$cpfs_provider" = "freeswitch" ]; then
        printf '%s\n' amrwb_be
      else
        printf '%s\n' amrwb
      fi
      ;;
    *) printf '%s\n' default ;;
  esac
}

# The transcode scenario is labeled by its *pairing*, not by one profile --
# its whole point is that the two legs run different codecs, so a single
# profile name cannot describe a cell. The label feeds the output path and
# the matrix codec column.
codec_label_for_scenario() {
  clfs_provider=$1
  scenario=$2
  if [ "$scenario" = "amr_transcode_call" ]; then
    if [ -n "${PBX_CODEC_PAIRING:-}" ]; then
      printf '%s\n' "$PBX_CODEC_PAIRING"
    elif [ "$clfs_provider" = "freeswitch" ]; then
      printf '%s\n' amrnb_be_pcmu
    else
      printf '%s\n' amrnb_pcmu
    fi
    return
  fi
  codec_profile_for_scenario "$clfs_provider" "$scenario"
}

# --- AMR capability probe -------------------------------------------------
# Whether the PBX *image* carries AMR differs from the provider: the local
# labs do, the committed release-runner images do not. A cell that would fail
# for "no codec in this image" proves nothing, so it records SKIP instead --
# and two guards keep the skip honest: PBX_ASSUME_AMR pins the answer without
# docker (the release gates set 0), and PBX_REQUIRE_AMR=1 (the AMR-capable
# labs) turns any skip into a loud FAIL so a lab regression cannot hide.
AMR_PROBE_STATUS=""
AMR_PROBE_NB=""
AMR_PROBE_WB=""
AMR_PROBE_PROVIDER=""

pbx_amr_probe() {
  pap_provider=$1
  if [ "$AMR_PROBE_PROVIDER" = "$pap_provider" ] && [ -n "$AMR_PROBE_STATUS" ]; then
    return
  fi
  mkdir -p "$OUT_ROOT/$pap_provider"
  pap_transcript="$OUT_ROOT/$pap_provider/amr-probe.txt"
  pap_line=$(sh "$SCRIPT_DIR/amr_probe.sh" detect "$pap_provider" "$pap_transcript" 2>>"$pap_transcript" || true)
  AMR_PROBE_STATUS=$(printf '%s' "$pap_line" | sed -n 's/.*status=\([a-z]*\).*/\1/p')
  AMR_PROBE_NB=$(printf '%s' "$pap_line" | sed -n 's/.*amr=\([a-z]*\).*/\1/p')
  AMR_PROBE_WB=$(printf '%s' "$pap_line" | sed -n 's/.*amrwb=\([a-z]*\).*/\1/p')
  AMR_PROBE_PROVIDER=$pap_provider
  [ -n "$AMR_PROBE_STATUS" ] || AMR_PROBE_STATUS=unknown
  echo "[$pap_provider] AMR probe: status=$AMR_PROBE_STATUS nb=$AMR_PROBE_NB wb=$AMR_PROBE_WB"
}

# Which AMR variants a codec label needs: nb, wb, or both.
amr_variants_for_label() {
  case "$1" in
    amrnb|amrnb_be|amrnb_pcmu|amrnb_be_pcmu) printf 'nb\n' ;;
    amrwb|amrwb_be|amrwb_pcmu|amrwb_be_pcmu) printf 'wb\n' ;;
    amrnb_amrwb) printf 'nb wb\n' ;;
    *) printf 'nb\n' ;;
  esac
}

# 0 = supported, 1 = not. Records the SKIP or the required-but-absent FAIL.
amr_cell_supported() {
  acs_provider=$1
  acs_scenario=$2
  acs_transport=$3
  acs_label=$4
  case "$acs_provider" in
    kamailio|opensips)
      # rtpengine relays payloads without touching them; there is no codec
      # module whose absence could make an AMR cell unrunnable.
      return 0
      ;;
  esac
  pbx_amr_probe "$acs_provider"
  acs_missing=""
  for variant in $(amr_variants_for_label "$acs_label"); do
    case "$variant" in
      nb) [ "$AMR_PROBE_NB" = "yes" ] || acs_missing="$acs_missing nb" ;;
      wb) [ "$AMR_PROBE_WB" = "yes" ] || acs_missing="$acs_missing wb" ;;
    esac
  done
  if [ -z "$acs_missing" ]; then
    return 0
  fi
  acs_now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  acs_transcript="$OUT_ROOT/$acs_provider/amr-probe.txt"
  case "${PBX_REQUIRE_AMR:-0}" in
    1|true|yes|on)
      echo "FAIL: $acs_provider lacks AMR ($acs_missing) but PBX_REQUIRE_AMR is set -- an AMR-capable lab losing its codec must not pass as a skip" >&2
      record_matrix FAIL "$acs_provider" "$(example_label "$example")" "$acs_scenario" "$acs_transport" probe 0 1 "$acs_now" "$acs_now" "$acs_transcript" "$OUT_ROOT/$acs_provider" "$acs_label"
      return 2
      ;;
  esac
  echo "[$acs_provider] skipping $acs_scenario/$acs_transport ($acs_label): image lacks AMR variant(s):$acs_missing (probe: $AMR_PROBE_STATUS)"
  record_matrix SKIP "$acs_provider" "$(example_label "$example")" "$acs_scenario" "$acs_transport" probe 0 0 "$acs_now" "$acs_now" "$acs_transcript" "$OUT_ROOT/$acs_provider" "$acs_label"
  return 1
}

# The pairing sweep, mirroring g729_profile_list: PBX_CODEC_PAIRING pins one,
# PBX_AMR_TRANSCODE_PAIRINGS overrides the list. The default differs per
# provider for a measured reason: FreeSWITCH's mod_amr instantiates its
# bandwidth-efficient decoder even for a leg negotiated octet-aligned
# ("Codec AMR / Bandwidth Efficient decoder error!" on PT 107 input), so on
# FreeSWITCH the bandwidth-efficient pairings are the ones that can work.
# The split also mirrors amr_call's, and is better coverage than either
# framing alone: Asterisk transcodes octet-aligned, FreeSWITCH transcodes
# bandwidth-efficient.
# The amr_call profile sweep. Asterisk and FreeSWITCH run their single
# provider default (one framing each -- see codec_profile_for_scenario); the
# proxy labs sweep all four, because rtpengine relays every framing with one
# config and "both framings" is exactly the evidence the AMR plan asks for.
amr_profile_list() {
  apl_provider=$1
  if [ -n "${PBX_CODEC_PROFILE:-}" ]; then
    printf '%s\n' "$PBX_CODEC_PROFILE"
    return
  fi
  if [ -n "${PBX_AMR_PROFILES:-}" ]; then
    for profile in $PBX_AMR_PROFILES; do
      printf '%s\n' "$profile"
    done
    return
  fi
  case "$apl_provider" in
    kamailio|opensips) printf '%s\n' amrnb amrwb amrnb_be amrwb_be ;;
    *) codec_profile_for_scenario "$apl_provider" amr_call ;;
  esac
}

# The b2bua profile sweep: a PCMU control cell (proving the scenario shape
# itself) plus AMR-WB in the framing the PBX can relay end to end. Override
# with PBX_B2BUA_PROFILES.
b2bua_profile_list() {
  bpl_provider=$1
  if [ -n "${PBX_CODEC_PROFILE:-}" ]; then
    printf '%s\n' "$PBX_CODEC_PROFILE"
    return
  fi
  if [ -n "${PBX_B2BUA_PROFILES:-}" ]; then
    for profile in $PBX_B2BUA_PROFILES; do
      printf '%s\n' "$profile"
    done
    return
  fi
  case "$bpl_provider" in
    freeswitch) printf '%s\n' pcmu amrwb_be ;;
    *) printf '%s\n' pcmu amrwb ;;
  esac
}

amr_transcode_pairing_list() {
  atpl_provider=$1
  if [ -n "${PBX_CODEC_PAIRING:-}" ]; then
    printf '%s\n' "$PBX_CODEC_PAIRING"
    return
  fi
  if [ -n "${PBX_AMR_TRANSCODE_PAIRINGS:-}" ]; then
    for pairing in $PBX_AMR_TRANSCODE_PAIRINGS; do
      printf '%s\n' "$pairing"
    done
    return
  fi
  case "$atpl_provider" in
    freeswitch) printf '%s\n' amrnb_be_pcmu amrwb_be_pcmu ;;
    *) printf '%s\n' amrnb_pcmu amrwb_pcmu ;;
  esac
}

g729_profile_list() {
  if [ -n "${PBX_CODEC_PROFILE:-}" ]; then
    printf '%s\n' "$PBX_CODEC_PROFILE"
    return
  fi
  for profile in $PBX_G729_PROFILES; do
    printf '%s\n' "$profile"
  done
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
    printf 'cargo run -p rvoip-sip --features %s --example %s --quiet\n' "$PBX_CARGO_FEATURES" "$example"
  else
    example_binary "$example"
  fi
}

run_example_command() {
  example=$1
  if truthy "$PBX_RUN_WITH_CARGO"; then
    cargo run -p rvoip-sip --features "$PBX_CARGO_FEATURES" --example "$example" --quiet
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
    kamailio:*) printf '%s\n' "${KAMAILIO_UDP_ADDR%%:*}" ;;
    opensips:*) printf '%s\n' "${OPENSIPS_UDP_ADDR%%:*}" ;;
    *) printf '%s\n' "${SIP_SERVER:-127.0.0.1}" ;;
  esac
}

pbx_port_for_diag() {
  provider=$1
  transport=$2
  case "$provider:$transport" in
    freeswitch:TLS) printf '%s\n' "${FREESWITCH_TLS_ADDR##*:}" ;;
    freeswitch:UDP) printf '%s\n' "${FREESWITCH_UDP_ADDR##*:}" ;;
    kamailio:*) printf '%s\n' "${KAMAILIO_UDP_ADDR##*:}" ;;
    opensips:*) printf '%s\n' "${OPENSIPS_UDP_ADDR##*:}" ;;
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

diag_ast_snapshot() {
  das_provider=$1
  das_out_dir=$2
  das_label=$3
  if ! diag_enabled || [ "$das_provider" != "asterisk" ]; then
    return
  fi
  das_snapshot="$das_out_dir/asterisk-cli-$das_label.txt"
  {
    echo "# Asterisk CLI snapshot: $das_label"
    echo
    # `core show channels verbose` names each channel's format;
    # `core show translation paths` and the codec module's use count are what
    # distinguish a transcoded call (simple_bridge, use count > 0) from a
    # relayed one (native_rtp, use count 0) -- the distinction the transcode
    # scenario exists to force.
    for command in \
      "core show channels verbose" \
      "core show channels concise" \
      "module show like codec_amr" \
      "bridge show all"
    do
      echo
      echo "## $command"
      echo
      docker exec rvoip-asterisk asterisk -rx "$command" 2>&1 || true
    done
  } >"$das_snapshot" 2>&1 || true
}

diag_proxy_snapshot() {
  dps_provider=$1
  dps_out_dir=$2
  dps_label=$3
  if ! diag_enabled; then
    return
  fi
  case "$dps_provider" in
    kamailio)
      dps_snapshot="$dps_out_dir/kamailio-$dps_label.txt"
      {
        echo "# Kamailio snapshot: $dps_label"
        echo
        # ul.dump proves the registrar bindings; rtpengine.show proves the
        # relay node is enabled (a disabled node 503s every call by design).
        for command in "ul.dump" "rtpengine.show all"; do
          echo
          echo "## kamcmd $command"
          echo
          docker exec rvoip-kamailio kamcmd $command 2>&1 || true
        done
        echo
        echo "## rtpengine log tail"
        echo
        docker logs rvoip-rtpengine-kamailio 2>&1 | tail -40 || true
      } >"$dps_snapshot" 2>&1 || true
      ;;
    opensips)
      dps_snapshot="$dps_out_dir/opensips-$dps_label.txt"
      {
        echo "# OpenSIPS snapshot: $dps_label"
        echo
        for command in "ul_dump" "rtpengine_show"; do
          echo
          echo "## opensips-cli -x mi $command"
          echo
          docker exec rvoip-opensips opensips-cli -x mi $command 2>&1 || true
        done
        echo
        echo "## rtpengine log tail"
        echo
        docker logs rvoip-rtpengine-opensips 2>&1 | tail -40 || true
      } >"$dps_snapshot" 2>&1 || true
      ;;
    *)
      return
      ;;
  esac
}

diag_ast_sample_loop() {
  dasl_provider=$1
  dasl_out_dir=$2
  if ! diag_enabled || [ "$dasl_provider" != "asterisk" ]; then
    return
  fi
  dasl_sample_dir="$dasl_out_dir/asterisk-cli-samples"
  mkdir -p "$dasl_sample_dir"
  dasl_count=0
  while [ "$dasl_count" -lt 60 ]; do
    dasl_stamp=$(date -u +%H%M%S)
    diag_ast_snapshot "$dasl_provider" "$dasl_sample_dir" "$dasl_stamp"
    dasl_count=$((dasl_count + 1))
    sleep 2
  done
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
  case "$provider" in
    kamailio)
      rtp_start=${KAMAILIO_RTP_START:-23000}
      rtp_end=${KAMAILIO_RTP_END:-23200}
      ;;
    opensips)
      rtp_start=${OPENSIPS_RTP_START:-23300}
      rtp_end=${OPENSIPS_RTP_END:-23500}
      ;;
    *)
      rtp_start=${FREESWITCH_RTP_START:-${ASTERISK_RTP_START:-16000}}
      rtp_end=${FREESWITCH_RTP_END:-${ASTERISK_RTP_END:-18100}}
      ;;
  esac
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
  diag_ast_snapshot "$provider" "$out_dir" before
  diag_proxy_snapshot "$provider" "$out_dir" before
  if [ "$provider" = "asterisk" ]; then
    diag_ast_sample_loop "$provider" "$out_dir" &
  else
    diag_fs_sample_loop "$provider" "$out_dir" &
  fi
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
  diag_ast_snapshot "$provider" "$out_dir" after
  diag_proxy_snapshot "$provider" "$out_dir" after
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
  codec_label=$(codec_label_for_scenario "$provider" "$scenario")
  if [ "$scenario" = "amr_transcode_call" ]; then
    codec_env="PBX_CODEC_PAIRING=$codec_label"
  else
    codec_env="PBX_CODEC_PROFILE=$codec_label"
  fi
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
    echo "- codec: $codec_label"
    echo "- started_at_utc: $started_at"
    echo "- output_dir: $out_dir"
    echo "- log: $log"
    echo
    echo "## Command"
    echo
    echo '```sh'
    echo "PBX_PROVIDER=$provider PBX_SCENARIO=$scenario PBX_TRANSPORT=$transport SIP_TRANSPORT=$transport PBX_ROLE=$role $codec_env AUDIO_OUTPUT_DIR=$out_dir $(example_command_label "$example")"
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
    echo "codec: $codec_label"
    echo "started_at_utc: $started_at"
    echo
    echo "+ PBX_PROVIDER=$provider PBX_SCENARIO=$scenario PBX_TRANSPORT=$transport SIP_TRANSPORT=$transport PBX_ROLE=$role $codec_env AUDIO_OUTPUT_DIR=$out_dir $(example_command_label "$example")"
  } >"$log"

  set +e
  (
    cd "$WORKSPACE_ROOT"
    export PBX_PROVIDER="$provider"
    export PBX_SCENARIO="$scenario"
    export PBX_TRANSPORT="$transport"
    export SIP_TRANSPORT="$transport"
    export PBX_ROLE="$role"
    if [ "$scenario" = "amr_transcode_call" ] && [ "$provider" = "freeswitch" ]; then
      # FreeSWITCH's rvoip profiles pin disable-transcoding, which REFUSES a
      # call whose legs cannot share a codec -- the exact shape of this
      # scenario. The container also writes *_xcode twins of both profiles
      # with transcoding enabled; register against those instead. The
      # exported value wins over both env files (dotenvy does not override
      # process env).
      if [ -n "${FREESWITCH_XCODE_UDP_ADDR:-}" ]; then
        export FREESWITCH_UDP_ADDR="$FREESWITCH_XCODE_UDP_ADDR"
      fi
      if [ -n "${FREESWITCH_XCODE_TLS_ADDR:-}" ]; then
        export FREESWITCH_TLS_ADDR="$FREESWITCH_XCODE_TLS_ADDR"
      fi
    fi
    if [ "$scenario" = "amr_transcode_call" ]; then
      # One env var cannot name two codecs; the binary resolves its leg's
      # profile from the pairing and its role, and a stray PBX_CODEC_PROFILE
      # is refused by select_codec_profile rather than silently collapsing
      # both legs onto one codec.
      unset PBX_CODEC_PROFILE
      export PBX_CODEC_PAIRING="$codec_label"
    else
      export PBX_CODEC_PROFILE="$codec_label"
    fi
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
  record_matrix "$status_label" "$provider" "$api_label" "$scenario" "$transport" "$role" "$duration" "$rc" "$started_at" "$ended_at" "$log" "$out_dir" "$codec_label"
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
    fs_cli_rc=0
    {
      echo
      echo "## attempt $i"
      if [ "$provider" = "freeswitch" ] && command -v docker >/dev/null 2>&1; then
        fs_cli_rc=1
        if fs_cli_output=$(docker exec rvoip-freeswitch \
          fs_cli -p "${FREESWITCH_EVENT_SOCKET_PASSWORD:-ClueCon}" \
          -x "sofia status" 2>&1); then
          printf '%s\n' "$fs_cli_output" | sed -n '1,80p'
          if printf '%s\n' "$fs_cli_output" \
            | grep -Eq 'rvoip_tls_srtp[[:space:]].*RUNNING'; then
            fs_cli_rc=0
          fi
        else
          printf '%s\n' "$fs_cli_output" | sed -n '1,80p'
        fi
        echo "fs_cli_rc=$fs_cli_rc"
      fi
      if command -v nc >/dev/null 2>&1; then
        if nc -z -w 2 "$host" "$port"; then
          nc_rc=0
        else
          nc_rc=$?
        fi
        echo "nc_rc=$nc_rc"
      else
        echo "nc not found; TLS readiness requires the TCP socket probe"
      fi
      if command -v openssl >/dev/null 2>&1; then
        if openssl_output=$(printf '' \
          | openssl s_client -connect "$host:$port" -servername "$host" -brief 2>&1); then
          openssl_rc=0
        else
          openssl_rc=$?
        fi
        printf '%s\n' "$openssl_output" | sed -n '1,80p'
        echo "openssl_rc=$openssl_rc"
      else
        echo "openssl not found; TLS readiness requires the handshake probe"
      fi
    } >>"$ready_log" 2>&1
    if [ "$nc_rc" -eq 0 ] && [ "$openssl_rc" -eq 0 ] && [ "$fs_cli_rc" -eq 0 ]; then
      echo "ready_at_attempt=$i" >>"$ready_log"
      return 0
    fi
    sleep "$sleep_secs"
    i=$((i + 1))
  done
  if [ "$provider" = "freeswitch" ] && command -v docker >/dev/null 2>&1; then
    capture_command "$out_dir/freeswitch-container.log" \
      docker logs rvoip-freeswitch
  fi
  echo "PBX TLS service was not ready at $host:$port after $attempts attempts; see $ready_log" >&2
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

  if [ "$rc" -ne 0 ] && [ "$provider" = "freeswitch" ] \
    && command -v docker >/dev/null 2>&1; then
    capture_command "$out_dir/freeswitch-container.log" \
      docker logs rvoip-freeswitch
  fi

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
  if ! cell_transport_enabled "$provider" TLS || ! truthy "$PBX_TLS_PREWARM"; then
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
  codec_profile=$(codec_profile_for_scenario "$provider" "$scenario")
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
    echo "codec_profile: $codec_profile"
    echo "started_at_utc: $started_at"
    echo
    echo "+ PBX_PROVIDER=$provider PBX_SCENARIO=$scenario PBX_TRANSPORT=$transport SIP_TRANSPORT=$transport PBX_CODEC_PROFILE=$codec_profile AUDIO_OUTPUT_DIR=$out_dir $(example_command_label pbx_analyze)"
  } >"$log"
  set +e
  (
    cd "$WORKSPACE_ROOT"
    export PBX_PROVIDER="$provider"
    export PBX_SCENARIO="$scenario"
    export PBX_TRANSPORT="$transport"
    export SIP_TRANSPORT="$transport"
    export PBX_CODEC_PROFILE="$codec_profile"
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
  record_matrix "$status_label" "$provider" analyzer "$scenario" "$transport" analyze "$duration" "$rc" "$started_at" "$ended_at" "$log" "$out_dir" "$codec_profile"
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
    if ! cell_transport_enabled "$provider" "$transport"; then
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
  codec_label=$(codec_label_for_scenario "$provider" "$scenario")
  if [ "$scenario" = "g729_call" ] || [ "$scenario" = "amr_call" ] || [ "$scenario" = "amr_transcode_call" ]; then
    out_dir="$OUT_ROOT/$provider/$api_label/$scenario/$codec_label/$transport"
  else
    out_dir="$OUT_ROOT/$provider/$api_label/$scenario/$transport"
  fi
  out_dir=$(iteration_out_dir "$out_dir")
  rm -rf "$out_dir"
  mkdir -p "$out_dir"
  if [ "$transport" = "TLS" ]; then
    prepare_tls "$provider" "$out_dir"
  fi
  diag_begin_cell "$provider" "$transport" "$out_dir"

  rc=0
  case "$scenario" in
    basic_call|g729_call|amr_call|amr_transcode_call|hold_resume|dtmf|reject)
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
    basic_call|g729_call|amr_call|hold_resume|dtmf)
      if [ "$rc" -eq 0 ]; then
        run_analyze "$provider" "$scenario" "$transport" "$out_dir" || rc=$?
      fi
      ;;
  esac
  diag_end_cell "$provider" "$transport" "$out_dir"
  return "$rc"
}

# rvoip as the B2BUA in the middle: caller(2001) -> PBX -> b2bua(2002) -> PBX
# -> target(2003). Three role processes, target and b2bua backgrounded, the
# caller in the foreground driving teardown. codec-labeled output like the
# amr/g729 cells. Endpoint API only.
run_b2bua() {
  provider=$1
  example=$2
  transport=$3
  api_label=$(example_label "$example")
  scenario=b2bua_call
  codec_label=$(codec_label_for_scenario "$provider" b2bua_call)
  out_dir="$OUT_ROOT/$provider/$api_label/$scenario/$codec_label/$transport"
  out_dir=$(iteration_out_dir "$out_dir")
  rm -rf "$out_dir"
  mkdir -p "$out_dir"
  if [ "$transport" = "TLS" ]; then
    prepare_tls "$provider" "$out_dir"
  fi
  diag_begin_cell "$provider" "$transport" "$out_dir"

  rc=0
  start_one "$provider" "$example" "$scenario" "$transport" target "$out_dir" "$out_dir/target.log"
  pid_a=$LAST_PID
  wait_for_log "$out_dir/target.log" "Registered." "$pid_a" b2bua-target || rc=$?
  if [ "$rc" -eq 0 ]; then
    start_one "$provider" "$example" "$scenario" "$transport" b2bua "$out_dir" "$out_dir/b2bua.log"
    pid_b=$LAST_PID
    wait_for_log "$out_dir/b2bua.log" "Registered." "$pid_b" b2bua-bridge || rc=$?
  else
    pid_b=""
  fi
  if [ "$rc" -eq 0 ]; then
    run_one "$provider" "$example" "$scenario" "$transport" caller "$out_dir" "$out_dir/caller.log" || rc=$?
  fi
  wait_child "$pid_a" b2bua-target "$out_dir/target.log" || {
    child_rc=$?
    if [ "$rc" -eq 0 ]; then rc=$child_rc; fi
  }
  if [ -n "$pid_b" ]; then
    wait_child "$pid_b" b2bua-bridge "$out_dir/b2bua.log" || {
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
  if ! provider_scenario_supported "$provider" "$scenario"; then
    echo "[$provider] skipping $scenario: unsupported for this provider (PBX_PROXY_ALL_SCENARIOS=1 to force)"
    return 0
  fi
  case "$scenario" in
    registration)
      run_registration "$provider" "$example" || rc=$?
      ;;
    basic_call)
      if transport_selected UDP; then
        run_two_party "$provider" "$example" basic_call UDP || rc=$?
        if [ "$rc" -ne 0 ] && [ "$STOP_ON_FAIL" = "1" ]; then return "$rc"; fi
      fi
      if cell_transport_enabled "$provider" TLS; then
        run_two_party "$provider" "$example" basic_call TLS || {
          tls_rc=$?
          if [ "$rc" -eq 0 ]; then rc=$tls_rc; fi
        }
      fi
      ;;
    amr_call)
      # Asterisk/FreeSWITCH run their provider default (one framing each);
      # the proxy labs sweep all four framings through one rtpengine config.
      # PBX_CODEC_PROFILE pins one profile, PBX_AMR_PROFILES overrides the
      # sweep list.
      old_amr_profile_set=0
      old_amr_profile=""
      if [ "${PBX_CODEC_PROFILE+x}" = "x" ]; then
        old_amr_profile_set=1
        old_amr_profile=$PBX_CODEC_PROFILE
      fi
      for profile in $(amr_profile_list "$provider"); do
        export PBX_CODEC_PROFILE="$profile"
        profile_rc=0
        if transport_selected UDP; then
          if amr_cell_supported "$provider" amr_call UDP "$profile"; then
            run_two_party "$provider" "$example" amr_call UDP || {
              profile_rc=$?
              if [ "$rc" -eq 0 ]; then rc=$profile_rc; fi
            }
          elif [ "$?" -eq 2 ]; then
            profile_rc=1
            if [ "$rc" -eq 0 ]; then rc=1; fi
          fi
          if [ "$profile_rc" -ne 0 ] && [ "$STOP_ON_FAIL" = "1" ]; then break; fi
        fi
        if cell_transport_enabled "$provider" TLS; then
          if amr_cell_supported "$provider" amr_call TLS "$profile"; then
            run_two_party "$provider" "$example" amr_call TLS || {
              profile_rc=$?
              if [ "$rc" -eq 0 ]; then rc=$profile_rc; fi
            }
          elif [ "$?" -eq 2 ] && [ "$rc" -eq 0 ]; then
            profile_rc=1
            rc=1
          fi
        fi
        if [ "$profile_rc" -ne 0 ] && [ "$STOP_ON_FAIL" = "1" ]; then break; fi
      done
      if [ "$old_amr_profile_set" = "1" ]; then
        export PBX_CODEC_PROFILE="$old_amr_profile"
      else
        unset PBX_CODEC_PROFILE
      fi
      ;;
    amr_transcode_call)
      # Sweeps the pairings the way g729_call sweeps its profiles.
      # PBX_CODEC_PAIRING pins one; PBX_AMR_TRANSCODE_PAIRINGS is the list.
      old_pairing_set=0
      old_pairing=""
      if [ "${PBX_CODEC_PAIRING+x}" = "x" ]; then
        old_pairing_set=1
        old_pairing=$PBX_CODEC_PAIRING
      fi
      for pairing in $(amr_transcode_pairing_list "$provider"); do
        export PBX_CODEC_PAIRING="$pairing"
        pairing_rc=0
        if transport_selected UDP; then
          if amr_cell_supported "$provider" amr_transcode_call UDP "$pairing"; then
            run_two_party "$provider" "$example" amr_transcode_call UDP || {
              pairing_rc=$?
              if [ "$rc" -eq 0 ]; then rc=$pairing_rc; fi
            }
          elif [ "$?" -eq 2 ]; then
            pairing_rc=1
            if [ "$rc" -eq 0 ]; then rc=1; fi
          fi
          if [ "$pairing_rc" -ne 0 ] && [ "$STOP_ON_FAIL" = "1" ]; then
            break
          fi
        fi
        if cell_transport_enabled "$provider" TLS; then
          if amr_cell_supported "$provider" amr_transcode_call TLS "$pairing"; then
            run_two_party "$provider" "$example" amr_transcode_call TLS || {
              pairing_rc=$?
              if [ "$rc" -eq 0 ]; then rc=$pairing_rc; fi
            }
          elif [ "$?" -eq 2 ]; then
            pairing_rc=1
            if [ "$rc" -eq 0 ]; then rc=1; fi
          fi
        fi
        if [ "$pairing_rc" -ne 0 ] && [ "$STOP_ON_FAIL" = "1" ]; then
          break
        fi
      done
      if [ "$old_pairing_set" = "1" ]; then
        export PBX_CODEC_PAIRING="$old_pairing"
      else
        unset PBX_CODEC_PAIRING
      fi
      ;;
    g729_call)
      old_profile_set=0
      old_profile=""
      if [ "${PBX_CODEC_PROFILE+x}" = "x" ]; then
        old_profile_set=1
        old_profile=$PBX_CODEC_PROFILE
      fi
      for profile in $(g729_profile_list); do
        export PBX_CODEC_PROFILE="$profile"
        profile_rc=0
        if transport_selected UDP; then
          run_two_party "$provider" "$example" g729_call UDP || {
            profile_rc=$?
            if [ "$rc" -eq 0 ]; then rc=$profile_rc; fi
          }
          if [ "$profile_rc" -ne 0 ] && [ "$STOP_ON_FAIL" = "1" ]; then
            break
          fi
        fi
        if cell_transport_enabled "$provider" TLS; then
          run_two_party "$provider" "$example" g729_call TLS || {
            profile_rc=$?
            if [ "$rc" -eq 0 ]; then rc=$profile_rc; fi
          }
        fi
        if [ "$profile_rc" -ne 0 ] && [ "$STOP_ON_FAIL" = "1" ]; then
          break
        fi
      done
      if [ "$old_profile_set" = "1" ]; then
        export PBX_CODEC_PROFILE="$old_profile"
      else
        unset PBX_CODEC_PROFILE
      fi
      ;;
    hold_resume|ring_cancel|dtmf|reject)
      if transport_selected UDP; then
        run_two_party "$provider" "$example" "$scenario" UDP || rc=$?
        if [ "$rc" -ne 0 ] && [ "$STOP_ON_FAIL" = "1" ]; then return "$rc"; fi
      fi
      if cell_transport_enabled "$provider" TLS; then
        run_two_party "$provider" "$example" "$scenario" TLS || {
          tls_rc=$?
          if [ "$rc" -eq 0 ]; then rc=$tls_rc; fi
        }
      fi
      ;;
    b2bua_call)
      # rvoip is the B2BUA here, and that role is composed on the unified
      # coordinator the endpoint API wraps; the other two APIs would need
      # their own bridge plumbing, so they skip rather than fail.
      if [ "$example" != "pbx_endpoint" ]; then
        echo "[$provider] skipping b2bua_call for $(example_label "$example"): endpoint API only"
        return 0
      fi
      # A PCMU control cell plus AMR-WB; the AMR cells go through the same
      # capability probe as amr_call.
      old_b2bua_profile_set=0
      old_b2bua_profile=""
      if [ "${PBX_CODEC_PROFILE+x}" = "x" ]; then
        old_b2bua_profile_set=1
        old_b2bua_profile=$PBX_CODEC_PROFILE
      fi
      for profile in $(b2bua_profile_list "$provider"); do
        export PBX_CODEC_PROFILE="$profile"
        profile_rc=0
        case "$profile" in
          amr*)
            probe_ok=1
            if transport_selected UDP; then
              if amr_cell_supported "$provider" b2bua_call UDP "$profile"; then :; else
                [ "$?" -eq 2 ] && { rc=1; profile_rc=1; }
                probe_ok=0
              fi
            fi
            ;;
          *) probe_ok=1 ;;
        esac
        if [ "$probe_ok" = "1" ]; then
          if transport_selected UDP; then
            run_b2bua "$provider" "$example" UDP || {
              profile_rc=$?
              if [ "$rc" -eq 0 ]; then rc=$profile_rc; fi
            }
            if [ "$profile_rc" -ne 0 ] && [ "$STOP_ON_FAIL" = "1" ]; then break; fi
          fi
          if cell_transport_enabled "$provider" TLS; then
            case "$profile" in
              amr*) amr_cell_supported "$provider" b2bua_call TLS "$profile" || {
                      [ "$?" -eq 2 ] && { if [ "$rc" -eq 0 ]; then rc=1; fi; }
                      continue
                    } ;;
            esac
            run_b2bua "$provider" "$example" TLS || {
              profile_rc=$?
              if [ "$rc" -eq 0 ]; then rc=$profile_rc; fi
            }
          fi
        fi
        if [ "$profile_rc" -ne 0 ] && [ "$STOP_ON_FAIL" = "1" ]; then break; fi
      done
      if [ "$old_b2bua_profile_set" = "1" ]; then
        export PBX_CODEC_PROFILE="$old_b2bua_profile"
      else
        unset PBX_CODEC_PROFILE
      fi
      ;;
    blind_transfer)
      if transport_selected UDP; then
        run_transfer "$provider" "$example" UDP || rc=$?
        if [ "$rc" -ne 0 ] && [ "$STOP_ON_FAIL" = "1" ]; then return "$rc"; fi
      fi
      if cell_transport_enabled "$provider" TLS; then
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
cargo build -p rvoip-sip --features "$PBX_CARGO_FEATURES" \
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
