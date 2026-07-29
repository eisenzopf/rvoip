#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
INTEROP_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
WORKSPACE_ROOT=$(git -C "$INTEROP_DIR" rev-parse --show-toplevel)
# shellcheck source=common.sh
. "$SCRIPT_DIR/common.sh"
# shellcheck source=tls.sh
. "$SCRIPT_DIR/tls.sh"

SIPP_BIN=${SIPP_BIN:-sipp}
TCPDUMP_BIN=${TCPDUMP_BIN:-tcpdump}
TSHARK_BIN=${TSHARK_BIN:-tshark}
PEERS=${PROXY_INTEROP_PEERS:-"kamailio opensips"}
TRANSPORTS=${PROXY_INTEROP_TRANSPORTS:-"udp tcp tls"}
ORDERS=${PROXY_INTEROP_ORDERS:-"rvoip-first peer-first"}
RVOIP_PORT=${PROXY_INTEROP_RVOIP_PORT:-25060}
UAS_PORT=${PROXY_INTEROP_UAS_PORT:-25080}
UAC_PORT=${PROXY_INTEROP_UAC_PORT:-25090}
PEER_TCP_EGRESS_PORT=${PROXY_INTEROP_PEER_TCP_EGRESS_PORT:-$((PROXY_INTEROP_PEER_PORT + 1))}
RAW_UAC_PORT_MATCHED_BEFORE=${PROXY_INTEROP_RAW_UAC_PORT_MATCHED_BEFORE:-$((UAC_PORT + 1))}
RAW_UAC_PORT_MATCHED_AFTER=${PROXY_INTEROP_RAW_UAC_PORT_MATCHED_AFTER:-$((UAC_PORT + 2))}
RAW_UAC_PORT_CANCEL_RETRANSMISSION=${PROXY_INTEROP_RAW_UAC_PORT_CANCEL_RETRANSMISSION:-$((UAC_PORT + 3))}
RAW_UAC_PORT_UNMATCHED_CANCEL=${PROXY_INTEROP_RAW_UAC_PORT_UNMATCHED_CANCEL:-$((UAC_PORT + 4))}
AUX_UAS_PORT_1=${PROXY_INTEROP_AUX_UAS_PORT_1:-25081}
AUX_UAS_PORT_2=${PROXY_INTEROP_AUX_UAS_PORT_2:-25082}
AUX_UAS_PORT_3=${PROXY_INTEROP_AUX_UAS_PORT_3:-25083}
DEAD_TARGET_PORT=${PROXY_INTEROP_DEAD_TARGET_PORT:-25999}
RFC3263_DNS_PORT=${PROXY_INTEROP_RFC3263_DNS_PORT:-25353}
TLS_PEER_BOUNDARY_PORT=${PROXY_INTEROP_TLS_PEER_BOUNDARY_PORT:-25170}
TLS_UAS_BOUNDARY_PORT=${PROXY_INTEROP_TLS_UAS_BOUNDARY_PORT:-25180}
TLS_UAC_BOUNDARY_PORT=${PROXY_INTEROP_TLS_UAC_BOUNDARY_PORT:-25190}
RETENTION_DRAIN_SECONDS=${PROXY_INTEROP_RETENTION_DRAIN_SECONDS:-130}
FAIL_FAST=${PROXY_INTEROP_FAIL_FAST:-1}
REQUIRE_CLEAN_SOURCE=${PROXY_INTEROP_REQUIRE_CLEAN_SOURCE:-0}
REQUIRE_UNCHANGED_SOURCE=${PROXY_INTEROP_REQUIRE_UNCHANGED_SOURCE:-1}
REQUIRE_RUNTIME_STATE=${PROXY_INTEROP_REQUIRE_RUNTIME_STATE:-1}
REQUIRE_RETENTION_CONVERGENCE=${PROXY_INTEROP_REQUIRE_RETENTION_CONVERGENCE:-1}
RUN_ADVANCED_SCENARIOS=${PROXY_INTEROP_RUN_ADVANCED_SCENARIOS:-1}
RUNTIME_STATE_SETTLE_ATTEMPTS=${PROXY_INTEROP_RUNTIME_STATE_SETTLE_ATTEMPTS:-100}
timestamp=$(date -u +%Y%m%dT%H%M%SZ)
STARTED_AT_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)
ARTIFACT_DIR=${PROXY_INTEROP_ARTIFACT_DIR:-"$WORKSPACE_ROOT/target/proxy-interop/$timestamp"}
RUN_DIR="$ARTIFACT_DIR/runtime"
MATRIX="$ARTIFACT_DIR/matrix.tsv"
PROXY_BINARY="$WORKSPACE_ROOT/target/release/examples/stateful_proxy_interop"
DOCKER_SNAPSHOT="$WORKSPACE_ROOT/crates/sip/rvoip-sip/scripts/docker_peer_snapshot.py"
STATE_SNAPSHOT="$SCRIPT_DIR/state_snapshot.py"
SCENARIO_EVIDENCE="$SCRIPT_DIR/scenario_evidence.py"
PACKET_EVIDENCE="$SCRIPT_DIR/packet_evidence.py"
TLS_BOUNDARY="$SCRIPT_DIR/tls_boundary.py"
RAW_UNMATCHED_CANCEL="$SCRIPT_DIR/raw_unmatched_cancel.py"
RAW_CANCEL_RETRANSMISSION="$SCRIPT_DIR/raw_cancel_retransmission.py"
RAW_MATCHED_CANCEL="$SCRIPT_DIR/raw_matched_cancel.py"
RAW_ADVANCED_UDP="$SCRIPT_DIR/raw_advanced_udp.py"
RAW_TCP_TIMER_FAILURE="$SCRIPT_DIR/raw_tcp_timer_failure.py"
RAW_TCP_ROUTING_AUTH="$SCRIPT_DIR/raw_tcp_routing_auth.py"
RFC3263_DNS="$SCRIPT_DIR/rfc3263_dns.py"
MAX_SCENARIO_TRACE_BYTES=${PROXY_INTEROP_MAX_SCENARIO_TRACE_BYTES:-8388608}

CORE_SCENARIOS=(
  options-readiness
  invite-success
  matched-cancel-before-provisional
  matched-cancel-after-provisional
  cancel-retransmission
  unmatched-cancel
  ack-non2xx
  via-response-destination
  message-body-content-length
)

if [[ -e "$ARTIFACT_DIR" ]]; then
  if [[ ! -d "$ARTIFACT_DIR" ]] ||
    [[ -n "$(find "$ARTIFACT_DIR" -mindepth 1 -maxdepth 1 -print -quit)" ]]; then
    echo "artifact directory must be a fresh, empty directory: $ARTIFACT_DIR" >&2
    exit 1
  fi
fi
mkdir -p "$RUN_DIR"
printf 'peer\torder\ttransport\tstatus\tduration_seconds\tartifact\tscenarios\n' >"$MATRIX"

for tool in docker "$SIPP_BIN" "$TCPDUMP_BIN" "$TSHARK_BIN" python3 cargo git sed; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "required tool not found: $tool" >&2
    exit 1
  fi
done
if ! docker info >/dev/null 2>&1; then
  echo "Docker is not reachable" >&2
  exit 1
fi
if [[ ! -x "$STATE_SNAPSHOT" && ! -f "$STATE_SNAPSHOT" ]]; then
  echo "runtime-state helper is missing: $STATE_SNAPSHOT" >&2
  exit 1
fi
if [[ ! -f "$PACKET_EVIDENCE" ]]; then
  echo "structured packet analyzer is missing: $PACKET_EVIDENCE" >&2
  exit 1
fi
if [[ ! -f "$TLS_BOUNDARY" ]]; then
  echo "TLS hostname-verifying boundary is missing: $TLS_BOUNDARY" >&2
  exit 1
fi
if [[ ! -f "$RAW_UNMATCHED_CANCEL" ]]; then
  echo "raw unmatched-CANCEL driver is missing: $RAW_UNMATCHED_CANCEL" >&2
  exit 1
fi
if [[ ! -f "$RAW_CANCEL_RETRANSMISSION" ]]; then
  echo "raw CANCEL retransmission driver is missing: $RAW_CANCEL_RETRANSMISSION" >&2
  exit 1
fi
if [[ ! -f "$RAW_MATCHED_CANCEL" ]]; then
  echo "raw matched-CANCEL driver is missing: $RAW_MATCHED_CANCEL" >&2
  exit 1
fi
if [[ ! -f "$RAW_ADVANCED_UDP" ]]; then
  echo "raw advanced UDP driver is missing: $RAW_ADVANCED_UDP" >&2
  exit 1
fi
if [[ ! -f "$RAW_TCP_TIMER_FAILURE" ]]; then
  echo "raw advanced Timer/failure driver is missing: $RAW_TCP_TIMER_FAILURE" >&2
  exit 1
fi
if [[ ! -f "$RAW_TCP_ROUTING_AUTH" ]]; then
  echo "raw advanced routing/auth driver is missing: $RAW_TCP_ROUTING_AUTH" >&2
  exit 1
fi
if [[ ! -f "$RFC3263_DNS" ]]; then
  echo "RFC 3263 DNS authority is missing: $RFC3263_DNS" >&2
  exit 1
fi

resolved_tool_path() {
  local tool=$1
  local resolved
  resolved=$(command -v "$tool")
  if [[ "$resolved" != /* ]]; then
    resolved=$(CDPATH= cd -- "$(dirname -- "$resolved")" && pwd)/$(basename -- "$resolved")
  fi
  printf '%s\n' "$resolved"
}

first_nonempty_line() {
  awk 'NF { print; exit }'
}

sha256_stream() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 | awk '{print $1}'
  else
    openssl dgst -sha256 | awk '{print $NF}'
  fi
}

sha256_file() {
  local path=$1
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$path"
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$path"
  else
    openssl dgst -sha256 "$path"
  fi
}

source_fingerprint_value() {
  {
    git -C "$WORKSPACE_ROOT" diff --binary
    git -C "$WORKSPACE_ROOT" ls-files --others --exclude-standard -z |
      while IFS= read -r -d '' file; do
        printf 'untracked:%s\n' "$file"
        sha256_file "$WORKSPACE_ROOT/$file"
      done
  } | sha256_stream
}

host_os=$(uname -s)
docker_context=$(docker context show 2>/dev/null || true)
network_topology=""
peer_interface=""
case "$host_os" in
  Darwin)
    default_interface=$(route -n get default 2>/dev/null | awk '/interface:/{print $2; exit}')
    HOST_ADDRESS=${PROXY_INTEROP_HOST_ADDRESS:-$(ipconfig getifaddr "$default_interface" 2>/dev/null || true)}
    if command -v colima >/dev/null 2>&1 && [[ "$docker_context" == "colima" ]]; then
      PEER_ADDRESS=${PROXY_INTEROP_PEER_ADDRESS:-$(colima list 2>/dev/null |
        awk '$1 == "default" && $NF ~ /^[0-9]+\./ {print $NF; exit}')}
      peer_interface=$(route -n get "$PEER_ADDRESS" 2>/dev/null |
        awk '/interface:/{print $2; exit}')
      if [[ -z "${PROXY_INTEROP_HOST_ADDRESS:-}" && -n "$peer_interface" ]]; then
        HOST_ADDRESS=$(ifconfig "$peer_interface" |
          awk '$1 == "inet" {print $2; exit}')
      fi
      CAPTURE_INTERFACES=${PROXY_INTEROP_CAPTURE_INTERFACES:-"lo0 ${peer_interface:-bridge100}"}
      network_topology=darwin-colima-host-network
    else
      echo "Darwin interoperability currently requires the reviewed Colima host-network topology" >&2
      exit 1
    fi
    ;;
  Linux)
    if [[ "$(docker info --format '{{.OperatingSystem}}' 2>/dev/null)" == *"Docker Desktop"* ]]; then
      echo "native Linux Docker host networking is required; Docker Desktop Linux is unsupported" >&2
      exit 1
    fi
    HOST_ADDRESS=${PROXY_INTEROP_HOST_ADDRESS:-127.0.0.1}
    PEER_ADDRESS=${PROXY_INTEROP_PEER_ADDRESS:-127.0.0.1}
    CAPTURE_INTERFACES=${PROXY_INTEROP_CAPTURE_INTERFACES:-"any lo"}
    network_topology=linux-native-host-network
    ;;
  *)
    echo "unsupported interoperability host OS: $host_os" >&2
    exit 1
    ;;
esac
if [[ -z "${HOST_ADDRESS:-}" || -z "${PEER_ADDRESS:-}" ]]; then
  echo "failed to derive host/peer addresses for topology $network_topology" >&2
  exit 1
fi
export PROXY_INTEROP_NETWORK_TOPOLOGY="$network_topology"

BASE_COMPOSE_FILE_ARGS=("${COMPOSE_FILE_ARGS[@]}")
if [[ "$host_os" == Linux ]]; then
  # The same no-NAT host-network override used for Colima is also the only
  # accepted native-Linux topology.
  if [[ ! " ${COMPOSE_FILE_ARGS[*]} " =~ " $COLIMA_HOST_COMPOSE_FILE " ]]; then
    BASE_COMPOSE_FILE_ARGS+=(--file "$COLIMA_HOST_COMPOSE_FILE")
  fi
fi

source_sha=$(git -C "$WORKSPACE_ROOT" rev-parse HEAD)
source_status=$(git -C "$WORKSPACE_ROOT" status --short)
source_fingerprint=$(source_fingerprint_value)
source_dirty=false
[[ -z "$source_status" ]] || source_dirty=true
printf '%s\n' "$source_status" >"$ARTIFACT_DIR/source-status.txt"
if [[ "$REQUIRE_CLEAN_SOURCE" == 1 && "$source_dirty" == true ]]; then
  echo "release evidence requires a clean source tree" >&2
  exit 1
fi

{
  echo "started_at_utc: $STARTED_AT_UTC"
  echo "source_sha: $source_sha"
  echo "source_fingerprint: $source_fingerprint"
  echo "source_dirty: $source_dirty"
  echo "host_os: $host_os"
  echo "docker_context: $docker_context"
  echo "network_topology: $network_topology"
  echo "host_address: $HOST_ADDRESS"
  echo "peer_address: $PEER_ADDRESS"
  echo "capture_interfaces: $CAPTURE_INTERFACES"
  echo "peer_port: $PROXY_INTEROP_PEER_PORT"
  echo "peer_tcp_egress_port: $PEER_TCP_EGRESS_PORT"
  echo "rvoip_port: $RVOIP_PORT"
  echo "uas_port: $UAS_PORT"
  echo "uac_port: $UAC_PORT"
  echo "raw_uac_port_matched_before: $RAW_UAC_PORT_MATCHED_BEFORE"
  echo "raw_uac_port_matched_after: $RAW_UAC_PORT_MATCHED_AFTER"
  echo "raw_uac_port_cancel_retransmission: $RAW_UAC_PORT_CANCEL_RETRANSMISSION"
  echo "raw_uac_port_unmatched_cancel: $RAW_UAC_PORT_UNMATCHED_CANCEL"
  echo "rfc3263_dns_port: $RFC3263_DNS_PORT"
  echo "tls_peer_boundary_port: $TLS_PEER_BOUNDARY_PORT"
  echo "tls_uas_boundary_port: $TLS_UAS_BOUNDARY_PORT"
  echo "tls_uac_boundary_port: $TLS_UAC_BOUNDARY_PORT"
  echo "retention_drain_seconds: $RETENTION_DRAIN_SECONDS"
  echo "runtime_state_settle_attempts: $RUNTIME_STATE_SETTLE_ATTEMPTS"
  echo "peers: $PEERS"
  echo "orders: $ORDERS"
  echo "transports: $TRANSPORTS"
  echo "kamailio_image: $KAMAILIO_IMAGE"
  echo "kamailio_platform: $KAMAILIO_PLATFORM"
  echo "opensips_image: $OPENSIPS_IMAGE"
  echo "opensips_platform: $OPENSIPS_PLATFORM"
  echo "cargo_path: $(resolved_tool_path cargo)"
  echo "cargo: $(cargo --version)"
  echo "rustc_path: $(resolved_tool_path rustc)"
  echo "rustc: $(rustc --version)"
  echo "sipp_path: $(resolved_tool_path "$SIPP_BIN")"
  echo "sipp: $($SIPP_BIN -v 2>&1 | first_nonempty_line)"
  echo "tcpdump_path: $(resolved_tool_path "$TCPDUMP_BIN")"
  echo "tcpdump: $($TCPDUMP_BIN --version 2>&1 | first_nonempty_line)"
  echo "tshark_path: $(resolved_tool_path "$TSHARK_BIN")"
  echo "tshark: $($TSHARK_BIN --version 2>&1 | first_nonempty_line)"
  echo "docker_path: $(resolved_tool_path docker)"
  echo "docker: $(docker version --format '{{.Server.Version}} {{.Server.Os}}/{{.Server.Arch}}')"
} >"$ARTIFACT_DIR/environment.txt"

state_ports=(
  "$RVOIP_PORT" "$PROXY_INTEROP_PEER_PORT" "$PEER_TCP_EGRESS_PORT" "$UAS_PORT" "$UAC_PORT"
  "$RAW_UAC_PORT_MATCHED_BEFORE" "$RAW_UAC_PORT_MATCHED_AFTER"
  "$RAW_UAC_PORT_CANCEL_RETRANSMISSION" "$RAW_UAC_PORT_UNMATCHED_CANCEL"
  "$AUX_UAS_PORT_1" "$AUX_UAS_PORT_2" "$AUX_UAS_PORT_3" "$DEAD_TARGET_PORT"
  "$RFC3263_DNS_PORT"
  "$TLS_PEER_BOUNDARY_PORT" "$TLS_UAS_BOUNDARY_PORT" "$TLS_UAC_BOUNDARY_PORT"
)
state_snapshot_args=()
for port in "${state_ports[@]}"; do
  state_snapshot_args+=(--port "$port")
done
python3 -B "$STATE_SNAPSHOT" snapshot \
  --output "$ARTIFACT_DIR/runtime-state-start.json" \
  "${state_snapshot_args[@]}"

echo "Building exact release proxy executable..."
declare -a cargo_build_command=(
  "$(resolved_tool_path cargo)"
  build
  --manifest-path "$WORKSPACE_ROOT/Cargo.toml"
  --release
  --package rvoip-sip-proxy
  --example stateful_proxy_interop
)
{
  printf '%q ' "${cargo_build_command[@]}"
  printf '\n'
} >"$ARTIFACT_DIR/cargo-build-command.txt"
"${cargo_build_command[@]}" \
  2>&1 | tee "$ARTIFACT_DIR/cargo-build.log"
PROXY_BINARY_SHA256=$(sha256_file "$PROXY_BINARY" | awk '{print $1}')
printf '%s\n' "$PROXY_BINARY_SHA256" >"$ARTIFACT_DIR/proxy-binary.sha256"
printf '%s\n' "${PROXY_BINARY#"$WORKSPACE_ROOT"/}" >"$ARTIFACT_DIR/proxy-binary.path"

active_peer=""
proxy_pid=""
dns_pid=""
uas_pid=""
aux_uas_pids=()
capture_pids=()
capture_paths=()
tls_boundary_pids=()
row_dir=""
current_transport=""

stop_pid() {
  local pid=$1
  local signal_name=${2:-TERM}
  if [[ -n "$pid" ]] && kill -0 "$pid" >/dev/null 2>&1; then
    kill "-$signal_name" "$pid" >/dev/null 2>&1 || true
    for _ in $(seq 1 100); do
      kill -0 "$pid" >/dev/null 2>&1 || break
      sleep 0.02
    done
    if kill -0 "$pid" >/dev/null 2>&1; then
      kill -KILL "$pid" >/dev/null 2>&1 || true
    fi
    wait "$pid" >/dev/null 2>&1 || true
  fi
}

stop_captures() {
  local pid
  for pid in "${capture_pids[@]:-}"; do
    stop_pid "$pid" TERM
  done
  capture_pids=()
}

stop_tls_boundaries() {
  local pid
  for pid in "${tls_boundary_pids[@]:-}"; do
    stop_pid "$pid" TERM
  done
  tls_boundary_pids=()
}

cleanup_row() {
  local status=$?
  stop_captures
  stop_pid "${uas_pid:-}" TERM
  uas_pid=""
  local pid
  for pid in "${aux_uas_pids[@]:-}"; do
    stop_pid "$pid" TERM
  done
  aux_uas_pids=()
  stop_tls_boundaries
  stop_pid "${proxy_pid:-}" INT
  proxy_pid=""
  stop_rfc3263_dns || true
  if [[ -n "$active_peer" ]]; then
    if [[ -n "$row_dir" ]]; then
      compose logs --no-color "$active_peer" >"$row_dir/peer.log" 2>&1 || true
    fi
    if "$SCRIPT_DIR/down.sh" >/dev/null 2>&1; then
      active_peer=""
    else
      echo "failed to stop owned interoperability peer: $active_peer" >&2
      status=1
    fi
  fi
  return "$status"
}

cleanup_all() {
  local status=${1:-0}
  cleanup_row || true
  tls_cleanup_pki
  return "$status"
}

cleanup_signal() {
  local signal_name=$1
  local status
  case "$signal_name" in
    INT) status=130 ;;
    TERM) status=143 ;;
    *) status=1 ;;
  esac
  trap - EXIT INT TERM
  cleanup_row || true
  tls_cleanup_pki
  exit "$status"
}

trap 'status=$?; trap - EXIT; cleanup_all "$status"; exit "$status"' EXIT
trap 'cleanup_signal INT' INT
trap 'cleanup_signal TERM' TERM

if [[ " $TRANSPORTS " == *" tls "* ]]; then
  tls_prepare_pki "$WORKSPACE_ROOT"
fi

render_peer_config() {
  local peer=$1
  local transport=$2
  local order=$3
  local next_hop_host=$4
  local next_hop_port=$5
  local output=$6
  COMPOSE_FILE_ARGS=("${BASE_COMPOSE_FILE_ARGS[@]}")
  if [[ "$transport" == tls ]]; then
    tls_enable_compose_overlay "$INTEROP_DIR"
    tls_render_peer_config \
      "$INTEROP_DIR" "$peer" "$order" "$PEER_ADDRESS" "$PROXY_INTEROP_PEER_PORT" \
      "$next_hop_host" "$next_hop_port" "$output" "$row_dir/kamailio-tls-domain.cfg" \
      "$PEER_TCP_EGRESS_PORT"
  else
    # `up.sh` sources common.sh in a subprocess, so clear the exported overlay
    # flag as well as restoring this process's Compose argument array.
    tls_disable_compose_overlay
    sed \
      -e "s|__PUBLIC_HOST__|$PEER_ADDRESS|g" \
      -e "s|__PUBLIC_PORT__|$PROXY_INTEROP_PEER_PORT|g" \
      -e "s|__NEXT_HOP_HOST__|$next_hop_host|g" \
      -e "s|__NEXT_HOP_PORT__|$next_hop_port|g" \
      -e "s|__TRANSPORT__|$transport|g" \
      "$INTEROP_DIR/config/$peer.cfg.in" >"$output"
  fi
}

wait_for_proxy_line() {
  local pattern=$1
  local expected=${2:-1}
  for _ in $(seq 1 200); do
    local observed
    observed=$(grep -c "^$pattern" "$row_dir/rvoip.log" 2>/dev/null || true)
    if [[ "$observed" -ge "$expected" ]]; then
      return 0
    fi
    if ! kill -0 "$proxy_pid" >/dev/null 2>&1; then
      echo "rvoip proxy exited while waiting for $pattern" >&2
      return 1
    fi
    sleep 0.05
  done
  echo "rvoip proxy log timed out waiting for $pattern" >&2
  return 1
}

stop_rfc3263_dns() {
  local pid=${dns_pid:-}
  if [[ -z "$pid" ]]; then
    return 0
  fi
  if kill -0 "$pid" >/dev/null 2>&1; then
    stop_pid "$pid" TERM
  else
    wait "$pid" >/dev/null 2>&1 || true
  fi
  if kill -0 "$pid" >/dev/null 2>&1; then
    echo "RFC 3263 DNS authority did not stop: pid=$pid" >&2
    return 1
  fi
  dns_pid=""
}

wait_for_rfc3263_dns() {
  local log=$1
  for _ in $(seq 1 200); do
    if grep -q '^RFC3263_DNS_READY ' "$log" 2>/dev/null; then
      return 0
    fi
    if ! kill -0 "$dns_pid" >/dev/null 2>&1; then
      wait "$dns_pid" >/dev/null 2>&1 || true
      dns_pid=""
      echo "RFC 3263 DNS authority exited before readiness: $log" >&2
      return 1
    fi
    sleep 0.025
  done
  echo "RFC 3263 DNS authority readiness timed out: $log" >&2
  return 1
}

start_rfc3263_dns() {
  local scenario_dir="$row_dir/scenarios/rfc3263-failover"
  local log="$row_dir/rfc3263-dns.stdout.log"
  if [[ -n "$dns_pid" ]]; then
    echo "RFC 3263 DNS authority is already owned: pid=$dns_pid" >&2
    return 1
  fi
  mkdir -p "$scenario_dir"
  python3 -B "$RFC3263_DNS" \
    --listen "$HOST_ADDRESS:$RFC3263_DNS_PORT" \
    --zone failover.interop.test \
    --address "$HOST_ADDRESS" \
    --dead-port "$DEAD_TARGET_PORT" \
    --live-port "$AUX_UAS_PORT_1" \
    --log "$scenario_dir/dns-queries.jsonl" \
    >"$log" 2>&1 &
  dns_pid=$!
  if ! wait_for_rfc3263_dns "$log"; then
    stop_rfc3263_dns || true
    return 1
  fi
}

wait_for_tls_boundary() {
  local pid=$1
  local log=$2
  for _ in $(seq 1 200); do
    if grep -q '^TLS_BOUNDARY_READY ' "$log" 2>/dev/null; then
      return 0
    fi
    if ! kill -0 "$pid" >/dev/null 2>&1; then
      wait "$pid" >/dev/null 2>&1 || true
      echo "TLS boundary exited before readiness: $log" >&2
      return 1
    fi
    sleep 0.025
  done
  echo "TLS boundary readiness timed out: $log" >&2
  return 1
}

start_tls_boundary() {
  local log=$1
  shift
  python3 -B "$TLS_BOUNDARY" "$@" >"$log" 2>&1 &
  local pid=$!
  tls_boundary_pids+=("$pid")
  wait_for_tls_boundary "$pid" "$log"
}

start_tls_boundaries() {
  local peer=$1
  local order=$2
  local first_destination first_identity
  local peer_destination peer_destination_identity last_identity

  tls_select_peer "$peer"
  case "$order" in
    rvoip-first)
      first_destination="$HOST_ADDRESS:$RVOIP_PORT"
      first_identity="$PROXY_INTEROP_TLS_RVOIP_IDENTITY"
      peer_destination="$HOST_ADDRESS:$TLS_UAS_BOUNDARY_PORT"
      peer_destination_identity="$PROXY_INTEROP_TLS_SIPP_IDENTITY"
      last_identity="$PROXY_INTEROP_TLS_PEER_IDENTITY"
      ;;
    peer-first)
      first_destination="$PEER_ADDRESS:$PROXY_INTEROP_PEER_PORT"
      first_identity="$PROXY_INTEROP_TLS_PEER_IDENTITY"
      peer_destination="$HOST_ADDRESS:$RVOIP_PORT"
      peer_destination_identity="$PROXY_INTEROP_TLS_RVOIP_IDENTITY"
      last_identity="$PROXY_INTEROP_TLS_RVOIP_IDENTITY"
      ;;
    *)
      echo "unknown TLS adjacency order: $order" >&2
      return 2
      ;;
  esac

  # Start the terminal server first, followed by both outbound client
  # boundaries. No readiness probe opens a TLS connection, so every accepted
  # connection in these logs corresponds to actual scenario traffic.
  start_tls_boundary "$row_dir/tls-boundary-server.log" \
    --mode server \
    --listen "$HOST_ADDRESS:$TLS_UAS_BOUNDARY_PORT" \
    --destination "$HOST_ADDRESS:$UAS_PORT" \
    --ca "$PROXY_INTEROP_TLS_CA" \
    --certificate "$PROXY_INTEROP_TLS_SIPP_CERT" \
    --private-key "$PROXY_INTEROP_TLS_SIPP_KEY" \
    --expected-peer-dns "$last_identity" \
    --server-identity "$PROXY_INTEROP_TLS_SIPP_IDENTITY"

  start_tls_boundary "$row_dir/tls-${peer}-outbound-boundary.log" \
    --mode client \
    --listen "$HOST_ADDRESS:$TLS_PEER_BOUNDARY_PORT" \
    --destination "$peer_destination" \
    --ca "$PROXY_INTEROP_TLS_CA" \
    --certificate "$PROXY_INTEROP_TLS_PEER_CERT" \
    --private-key "$PROXY_INTEROP_TLS_PEER_KEY" \
    --expected-peer-dns "$peer_destination_identity"

  start_tls_boundary "$row_dir/tls-boundary-client.log" \
    --mode client \
    --listen "$HOST_ADDRESS:$TLS_UAC_BOUNDARY_PORT" \
    --destination "$first_destination" \
    --ca "$PROXY_INTEROP_TLS_CA" \
    --certificate "$PROXY_INTEROP_TLS_SIPP_CERT" \
    --private-key "$PROXY_INTEROP_TLS_SIPP_KEY" \
    --expected-peer-dns "$first_identity"
}

capture_interface_available() {
  local interface=$1
  "$TCPDUMP_BIN" -D 2>/dev/null |
    sed -E 's/^[0-9]+\.//' |
    awk '{print $1}' |
    grep -Fxq "$interface"
}

start_captures() {
  local scenario=$1
  if [[ ${#capture_pids[@]} -ne 0 ]]; then
    echo "capture processes are still active before scenario $scenario" >&2
    return 1
  fi
  capture_paths=()
  local filter="port $PROXY_INTEROP_PEER_PORT or port $RVOIP_PORT or port $UAS_PORT or port $UAC_PORT or port $AUX_UAS_PORT_1 or port $AUX_UAS_PORT_2 or port $AUX_UAS_PORT_3 or port $DEAD_TARGET_PORT or port $RFC3263_DNS_PORT or port $TLS_PEER_BOUNDARY_PORT or port $TLS_UAS_BOUNDARY_PORT or port $TLS_UAC_BOUNDARY_PORT"
  local tcpdump_args=("$TCPDUMP_BIN")
  if "$TCPDUMP_BIN" --help 2>&1 | grep -q -- "--immediate-mode"; then
    tcpdump_args+=(--immediate-mode)
  fi
  local interface safe_name pid
  for interface in $CAPTURE_INTERFACES; do
    capture_interface_available "$interface" || continue
    safe_name=${interface//[^a-zA-Z0-9_.-]/_}
    local capture_path="$row_dir/${scenario}--$safe_name.pcap"
    "${tcpdump_args[@]}" -i "$interface" -U -s 0 -n \
      -w "$capture_path" "$filter" \
      >"$row_dir/scenarios/$scenario/tcpdump-$safe_name.log" 2>&1 &
    pid=$!
    sleep 0.1
    if kill -0 "$pid" >/dev/null 2>&1; then
      capture_pids+=("$pid")
      capture_paths+=("$capture_path")
    else
      wait "$pid" || true
    fi
  done
  if [[ ${#capture_pids[@]} -eq 0 ]]; then
    echo "no configured capture interface could start" >&2
    return 1
  fi
}

validate_scenario_packet_evidence() {
  local scenario=$1
  local scenario_dir="$row_dir/scenarios/$scenario"
  stop_captures

  local pcap
  for pcap in "${capture_paths[@]:-}"; do
    if [[ ! -f "$pcap" || $(wc -c <"$pcap") -le 24 ]]; then
      echo "scenario capture did not flush packet records: $pcap" >&2
      return 1
    fi
    if [[ $(basename "$pcap") != "$scenario"--*.pcap ]]; then
      echo "scenario capture is not scenario-bound: $pcap" >&2
      return 1
    fi
  done
  if [[ ${#capture_paths[@]} -eq 0 ]]; then
    echo "scenario has no packet captures: $scenario" >&2
    return 1
  fi

  local packet_command=(
    python3 -B "$PACKET_EVIDENCE"
    --scenario "$scenario"
    --transport "$current_transport"
    --peer "$active_peer"
    --order "$current_order"
    --rvoip-address "$HOST_ADDRESS" --rvoip-port "$RVOIP_PORT"
    --peer-address "$PEER_ADDRESS" --peer-port "$PROXY_INTEROP_PEER_PORT"
    --uac-port "$UAC_PORT"
    --uas-port "$UAS_PORT"
    --tls-uac-boundary-port "$TLS_UAC_BOUNDARY_PORT"
    --tls-uas-boundary-port "$TLS_UAS_BOUNDARY_PORT"
    --tls-peer-boundary-port "$TLS_PEER_BOUNDARY_PORT"
    --dead-target-port "$DEAD_TARGET_PORT"
    --normal-target-port "$UAS_PORT"
    --dns-port "$RFC3263_DNS_PORT"
    --live-target-port "$AUX_UAS_PORT_1"
    --original-target-uri "sip:agent@destination.invalid;transport=tcp"
    --next-hop-route-uri \
      "sip:next-hop@$HOST_ADDRESS:$AUX_UAS_PORT_1;transport=tcp;lr"
    --record-route-uri "sip:$HOST_ADDRESS:$RVOIP_PORT;transport=tcp;lr"
    --local-route-uri "sip:$HOST_ADDRESS:$RVOIP_PORT;transport=tcp;lr"
  )
  if [[ "$current_transport" == tls ]]; then
    packet_command+=(
      --expected-tls-sni "$PROXY_INTEROP_TLS_RVOIP_IDENTITY"
      --expected-tls-sni "$PROXY_INTEROP_TLS_PEER_IDENTITY"
      --expected-tls-sni "$PROXY_INTEROP_TLS_SIPP_IDENTITY"
    )
  fi
  packet_command+=(
    --output "$scenario_dir/packet-evidence.json"
    "${capture_paths[@]}"
  )
  "${packet_command[@]}" || return
  if [[ "$scenario" == sips-routing ]]; then
    # The live SIPp and packet evidence is finalized with the independent
    # positive/negative TLS verifier after all row TLS logs are complete.
    return
  fi
  local evidence_mode=sipp
  case "$scenario" in
    sequential-fork|parallel-fork|multiple-2xx|late-2xx|sixxx-cancel|stray-response-drop|\
timer-c-calling|timer-c-proceeding|transport-failure|rfc3263-failover|\
route-strict|route-loose-record-route|auth-aggregation|capacity-overload)
      evidence_mode=raw
      ;;
  esac
  python3 -B "$SCENARIO_EVIDENCE" "$evidence_mode" \
    --scenario "$scenario" --directory "$scenario_dir" \
    --peer "$active_peer" --order "$current_order" --transport "$current_transport" \
    --expected-rvoip-sent-by "$HOST_ADDRESS:$RVOIP_PORT" \
    --expected-peer-sent-by "$PEER_ADDRESS:$PROXY_INTEROP_PEER_PORT"
}

run_captured_external_scenario() {
  local scenario=$1
  shift
  mkdir -p "$row_dir/scenarios/$scenario"
  start_captures "$scenario" || return
  local result=0
  "$@" || result=$?
  stop_captures
  if [[ "$result" -ne 0 ]]; then
    return "$result"
  fi
  validate_scenario_packet_evidence "$scenario"
}

sipp_mode() {
  case "$1" in
    udp) printf '%s\n' u1 ;;
    tcp) printf '%s\n' t1 ;;
    # SIPp 3.7.7 cannot prove RFC 6125 hostname identity. TLS rows use local
    # TCP behind the gate-owned mTLS boundaries.
    tls) printf '%s\n' t1 ;;
    *) return 2 ;;
  esac
}

sipp_uac_mode() {
  case "$1" in
    udp) printf '%s\n' u1 ;;
    # SIPp 3.7.7 on macOS opens both its listening and connected UAC sockets
    # on the configured local port in t1 mode, so the second bind fails with
    # EADDRINUSE even when no external process owns the port. Per-call mode
    # uses one connected socket for this one-call harness and preserves the
    # configured Via/Contact identity.
    tcp|tls) printf '%s\n' tn ;;
    *) return 2 ;;
  esac
}

wait_for_background_sipp() {
  local pid=$1
  for _ in $(seq 1 160); do
    kill -0 "$pid" >/dev/null 2>&1 || break
    sleep 0.05
  done
  if kill -0 "$pid" >/dev/null 2>&1; then
    return 1
  fi
  wait "$pid"
}

validate_scenario_trace_bounds() {
  local scenario_dir=$1
  local path bytes
  for path in "$scenario_dir"/*-messages.log "$scenario_dir"/*.stdout.log \
    "$scenario_dir"/*-errors.log; do
    [[ -f "$path" ]] || continue
    bytes=$(wc -c <"$path")
    if [[ "$bytes" -gt "$MAX_SCENARIO_TRACE_BYTES" ]]; then
      echo "scenario trace exceeded $MAX_SCENARIO_TRACE_BYTES bytes: $path" >&2
      return 1
    fi
  done
}

start_uas() {
  local scenario_file=$1
  local scenario_dir=$2
  local listen_port=${3:-$UAS_PORT}
  local mode
  mode=$(sipp_mode "$current_transport")
  local sipp_args=(
    -sf "$scenario_file"
    -i "$HOST_ADDRESS" -p "$listen_port" -t "$mode" -m 1
    -nostdin -timeout 30s -timeout_error -recv_timeout 20000
    -trace_err -error_file "$scenario_dir/uas-errors.log"
    -trace_msg -message_file "$scenario_dir/uas-messages.log"
    -trace_stat -stf "$scenario_dir/uas-stats.csv"
  )
  "$SIPP_BIN" "${sipp_args[@]}" \
    >"$scenario_dir/uas.stdout.log" 2>&1 &
  uas_pid=$!
  sleep 0.25
  if ! kill -0 "$uas_pid" >/dev/null 2>&1; then
    wait "$uas_pid" || true
    uas_pid=""
    return 1
  fi
}

run_uac() {
  local scenario_file=$1
  local scenario_dir=$2
  local target_host=$3
  local target_port=$4
  local mode
  mode=$(sipp_uac_mode "$current_transport")
  local sipp_args=(
    "$target_host:$target_port"
    -sf "$scenario_file"
    -i "$HOST_ADDRESS" -p "$UAC_PORT" -t "$mode" -m 1 -l 1 -r 1
    -nostdin -timeout 30s -timeout_error -recv_timeout 20000
    -trace_err -error_file "$scenario_dir/uac-errors.log"
    -trace_msg -message_file "$scenario_dir/uac-messages.log"
    -trace_stat -stf "$scenario_dir/uac-stats.csv"
  )
  "$SIPP_BIN" "${sipp_args[@]}" \
    >"$scenario_dir/uac.stdout.log" 2>&1
}

run_sipp_scenario() {
  local scenario=$1
  local uac_xml=$2
  local uas_xml=$3
  local target_host=$4
  local target_port=$5
  local scenario_dir="$row_dir/scenarios/$scenario"
  mkdir -p "$scenario_dir"
  if ! start_uas "$INTEROP_DIR/scenarios/$uas_xml" "$scenario_dir"; then
    echo "failed to start SIPp UAS for $scenario" >"$scenario_dir/harness-error.txt"
    return 1
  fi
  local result=0
  run_uac "$INTEROP_DIR/scenarios/$uac_xml" "$scenario_dir" \
    "$target_host" "$target_port" || result=$?
  if wait_for_background_sipp "$uas_pid"; then
    uas_pid=""
  else
    result=1
    # Preserve ownership when the bounded wait timed out so the row cleanup
    # can terminate the still-running process. A completed non-zero SIPp
    # process has already been reaped and no longer needs cleanup.
    if ! kill -0 "$uas_pid" >/dev/null 2>&1; then
      uas_pid=""
    fi
  fi
  validate_scenario_trace_bounds "$scenario_dir" || result=1
  if [[ "$result" -ne 0 ]]; then
    echo "SIPp scenario failed with exit $result" >"$scenario_dir/harness-error.txt"
    return 1
  fi
}

run_unmatched_cancel_scenario() {
  local target_host=$1
  local target_port=$2
  local uac_port=$UAC_PORT
  if [[ "$current_transport" != udp ]]; then
    uac_port=$RAW_UAC_PORT_UNMATCHED_CANCEL
  fi
  local scenario_dir="$row_dir/scenarios/unmatched-cancel"
  mkdir -p "$scenario_dir"
  python3 -B "$RAW_UNMATCHED_CANCEL" \
    --transport "$current_transport" \
    --uas-listen "$HOST_ADDRESS:$UAS_PORT" \
    --uac-bind "$HOST_ADDRESS:$uac_port" \
    --target "$target_host:$target_port" \
    --advertised-host "$HOST_ADDRESS" \
    --output-dir "$scenario_dir" || {
    echo "raw unmatched-CANCEL scenario failed" >"$scenario_dir/harness-error.txt"
    return 1
  }
  validate_scenario_trace_bounds "$scenario_dir"
}

run_cancel_retransmission_scenario() {
  local target_host=$1
  local target_port=$2
  local uac_port=$UAC_PORT
  if [[ "$current_transport" != udp ]]; then
    uac_port=$RAW_UAC_PORT_CANCEL_RETRANSMISSION
  fi
  local scenario_dir="$row_dir/scenarios/cancel-retransmission"
  mkdir -p "$scenario_dir"
  python3 -B "$RAW_CANCEL_RETRANSMISSION" \
    --transport "$current_transport" \
    --uas-listen "$HOST_ADDRESS:$UAS_PORT" \
    --uac-bind "$HOST_ADDRESS:$uac_port" \
    --target "$target_host:$target_port" \
    --advertised-host "$HOST_ADDRESS" \
    --output-dir "$scenario_dir" || {
    echo "raw CANCEL retransmission scenario failed" \
      >"$scenario_dir/harness-error.txt"
    return 1
  }
  validate_scenario_trace_bounds "$scenario_dir"
}

run_matched_cancel_scenario() {
  local ordering=$1
  local target_host=$2
  local target_port=$3
  local uac_port=$UAC_PORT
  if [[ "$current_transport" != udp ]]; then
    uac_port=$RAW_UAC_PORT_MATCHED_BEFORE
    if [[ "$ordering" == after ]]; then
      uac_port=$RAW_UAC_PORT_MATCHED_AFTER
    fi
  fi
  local scenario_dir="$row_dir/scenarios/matched-cancel-${ordering}-provisional"
  mkdir -p "$scenario_dir"
  python3 -B "$RAW_MATCHED_CANCEL" \
    --ordering "$ordering" \
    --transport "$current_transport" \
    --uas-listen "$HOST_ADDRESS:$UAS_PORT" \
    --uac-bind "$HOST_ADDRESS:$uac_port" \
    --target "$target_host:$target_port" \
    --advertised-host "$HOST_ADDRESS" \
    --output-dir "$scenario_dir" || {
    echo "raw matched-CANCEL $ordering-provisional scenario failed" \
      >"$scenario_dir/harness-error.txt"
    return 1
  }
  validate_scenario_trace_bounds "$scenario_dir"
}

run_via_response_scenario() {
  local target_host=$1
  local target_port=$2
  local scenario=via-response-destination
  local scenario_dir="$row_dir/scenarios/$scenario"
  mkdir -p "$scenario_dir"
  if [[ "$current_transport" != udp ]]; then
    run_sipp_scenario "$scenario" via_response_uac.xml options_uas.xml \
      "$target_host" "$target_port"
    return
  fi
  if ! start_uas "$INTEROP_DIR/scenarios/options_uas.xml" "$scenario_dir"; then
    echo "failed to start UDP Via probe UAS" >"$scenario_dir/harness-error.txt"
    return 1
  fi
  local result=0
  python3 -B "$SCRIPT_DIR/udp_via_probe.py" \
    --local-address "$HOST_ADDRESS" \
    --sent-by-port "$UAC_PORT" \
    --target-host "$target_host" \
    --target-port "$target_port" \
    --output "$scenario_dir/udp-via-probe.json" || result=$?
  if wait_for_background_sipp "$uas_pid"; then
    uas_pid=""
  else
    result=1
    if ! kill -0 "$uas_pid" >/dev/null 2>&1; then
      uas_pid=""
    fi
  fi
  [[ "$result" -eq 0 ]] || return "$result"
}

run_core_scenarios() {
  local target_host=$1
  local target_port=$2
  run_captured_external_scenario options-readiness \
    run_sipp_scenario options-readiness options_uac.xml options_uas.xml \
    "$target_host" "$target_port" || return
  run_captured_external_scenario invite-success \
    run_sipp_scenario invite-success invite_success_uac.xml invite_success_uas.xml \
    "$target_host" "$target_port" || return
  run_captured_external_scenario matched-cancel-before-provisional \
    run_matched_cancel_scenario before "$target_host" "$target_port" || return
  run_captured_external_scenario matched-cancel-after-provisional \
    run_matched_cancel_scenario after "$target_host" "$target_port" || return
  run_captured_external_scenario cancel-retransmission \
    run_cancel_retransmission_scenario "$target_host" "$target_port" || return
  run_captured_external_scenario unmatched-cancel \
    run_unmatched_cancel_scenario "$target_host" "$target_port" || return
  run_captured_external_scenario ack-non2xx \
    run_sipp_scenario ack-non2xx ack_non2xx_uac.xml ack_non2xx_uas.xml \
    "$target_host" "$target_port" || return
  run_captured_external_scenario via-response-destination \
    run_via_response_scenario "$target_host" "$target_port" || return
  run_captured_external_scenario message-body-content-length \
    run_sipp_scenario message-body-content-length \
    message_body_uac.xml message_body_uas.xml "$target_host" "$target_port"
}

advanced_scenarios_for_row() {
  if [[ "$RUN_ADVANCED_SCENARIOS" != 1 || "$current_order" != peer-first ]]; then
    return
  fi
  case "$current_transport" in
    udp)
      printf '%s\n' \
        sequential-fork parallel-fork multiple-2xx late-2xx sixxx-cancel stray-response-drop
      ;;
    tcp)
      printf '%s\n' \
        timer-c-calling timer-c-proceeding transport-failure rfc3263-failover \
        route-strict route-loose-record-route auth-aggregation capacity-overload
      ;;
    tls) return ;;
  esac
}

run_advanced_scenarios() {
  local target_host=$1
  local target_port=$2
  local scenario
  while IFS= read -r scenario; do
    [[ -n "$scenario" ]] || continue
    case "$current_transport" in
      udp)
        run_captured_external_scenario "$scenario" \
          python3 -B "$RAW_ADVANCED_UDP" \
          --scenario "$scenario" \
          --uac-bind "$HOST_ADDRESS:$UAC_PORT" \
          --proxy-target "$target_host:$target_port" \
          --rvoip-target "$HOST_ADDRESS:$RVOIP_PORT" \
          --uas-listen "primary=$HOST_ADDRESS:$UAS_PORT" \
          --uas-listen "aux1=$HOST_ADDRESS:$AUX_UAS_PORT_1" \
          --uas-listen "aux2=$HOST_ADDRESS:$AUX_UAS_PORT_2" \
          --advertised-host "$HOST_ADDRESS" \
          --expected-rvoip-sent-by "$HOST_ADDRESS:$RVOIP_PORT" \
          --expected-peer-sent-by "$PEER_ADDRESS:$PROXY_INTEROP_PEER_PORT" \
          --output-dir "$row_dir/scenarios/$scenario" || return
        ;;
      tcp)
        local common_tcp_args=(
          --scenario "$scenario"
          --primary-listen "$HOST_ADDRESS:$UAS_PORT"
          --uac-bind "$HOST_ADDRESS:0"
          --target "$target_host:$target_port"
          --advertised-host "$HOST_ADDRESS"
          --expected-rvoip-sent-by "$HOST_ADDRESS:$RVOIP_PORT"
          --expected-peer-sent-by "$PEER_ADDRESS:$PROXY_INTEROP_PEER_PORT"
          --order "$current_order"
          --output-dir "$row_dir/scenarios/$scenario"
        )
        case "$scenario" in
          timer-c-calling|timer-c-proceeding|transport-failure)
            run_captured_external_scenario "$scenario" \
              python3 -B "$RAW_TCP_TIMER_FAILURE" \
              "${common_tcp_args[@]}" --timer-c-ms 500 || return
            ;;
          rfc3263-failover)
            run_captured_external_scenario "$scenario" \
              python3 -B "$RAW_TCP_TIMER_FAILURE" \
              "${common_tcp_args[@]}" \
              --aux-listen "$HOST_ADDRESS:$AUX_UAS_PORT_1" || return
            ;;
          capacity-overload)
            run_captured_external_scenario "$scenario" \
              python3 -B "$RAW_TCP_TIMER_FAILURE" \
              "${common_tcp_args[@]}" \
              --capacity-fill-limit 72 \
              --quiet-window-ms 50 || return
            ;;
          route-strict|route-loose-record-route)
            run_captured_external_scenario "$scenario" \
              python3 -B "$RAW_TCP_ROUTING_AUTH" \
              "${common_tcp_args[@]}" \
              --aux-listen "$HOST_ADDRESS:$AUX_UAS_PORT_1" \
              --rvoip-local-uri \
                "sip:$HOST_ADDRESS:$RVOIP_PORT;transport=tcp;lr" \
              --next-hop-route-uri \
                "sip:next-hop@$HOST_ADDRESS:$AUX_UAS_PORT_1;transport=tcp;lr" \
              --original-target-uri \
                "sip:agent@destination.invalid;transport=tcp" \
              --expected-record-route-uri \
                "sip:$HOST_ADDRESS:$RVOIP_PORT;transport=tcp;lr" || return
            ;;
          auth-aggregation)
            run_captured_external_scenario "$scenario" \
              python3 -B "$RAW_TCP_ROUTING_AUTH" \
              "${common_tcp_args[@]}" \
              --aux-listen "$HOST_ADDRESS:$AUX_UAS_PORT_1" \
              --aux-listen "$HOST_ADDRESS:$AUX_UAS_PORT_2" || return
            ;;
          *)
            echo "unknown advanced TCP scenario: $scenario" >&2
            return 1
            ;;
        esac
        ;;
      *)
        echo "advanced external scenario driver not implemented: $scenario" >&2
        mkdir -p "$row_dir/scenarios/$scenario"
        printf '%s\n' "OPEN: real external-peer driver not yet implemented" \
          >"$row_dir/scenarios/$scenario/open-gate.txt"
        return 1
        ;;
    esac
  done < <(advanced_scenarios_for_row)
}

decode_and_validate_pcaps() {
  stop_captures
  local scenario scenario_dir
  for scenario in "${CORE_SCENARIOS[@]}"; do
    scenario_dir="$row_dir/scenarios/$scenario"
    [[ -s "$scenario_dir/packet-evidence.json" ]] || {
      echo "scenario packet evidence is missing: $scenario" >&2
      return 1
    }
    [[ -s "$scenario_dir/result.json" ]] || {
      echo "scenario result is missing: $scenario" >&2
      return 1
    }
    compgen -G "$row_dir/${scenario}--*.pcap" >/dev/null || {
      echo "scenario-bound packet capture is missing: $scenario" >&2
      return 1
    }
  done
}

parse_retention() {
  local require_zero=$1
  python3 -B - "$row_dir" "$active_peer" "$current_order" "$current_transport" "$require_zero" <<'PY'
import hashlib
import json
import pathlib
import sys

row = pathlib.Path(sys.argv[1])
peer, order, transport = sys.argv[2:5]
require_zero = sys.argv[5] == "1"
log_path = row / "rvoip.log"
expected = (
    "pre_zero",
    "activity",
    "cooldown",
    "post_retention",
    "pre_shutdown",
    "post_shutdown",
)
phases = {}
for raw in log_path.read_text().splitlines():
    if not raw.startswith("RVOIP_PROXY_RETENTION "):
        continue
    fields = {}
    for field in raw.split()[1:]:
        key, value = field.split("=", 1)
        fields[key] = value
    phase = fields.pop("phase")
    if phase in phases:
        raise SystemExit(f"duplicate retention phase {phase}")
    counters = {key: int(value) for key, value in fields.items()}
    phases[phase] = {
        "counters": counters,
        "nonzero": {key: value for key, value in counters.items() if value},
    }
phase_set_ok = tuple(phases) == expected
checks = [
    {"name": "exact-phase-sequence", "passed": phase_set_ok, "observed": list(phases)},
    {
        "name": "pre-zero-empty",
        "passed": not phases.get("pre_zero", {}).get("nonzero"),
        "observed": phases.get("pre_zero", {}).get("nonzero"),
    },
    {
        "name": "activity-observed",
        "passed": bool(phases.get("activity", {}).get("nonzero")),
        "observed": phases.get("activity", {}).get("nonzero"),
    },
]
for phase in ("post_retention", "pre_shutdown", "post_shutdown"):
    checks.append(
        {
            "name": f"{phase.replace('_', '-')}-empty",
            "passed": not phases.get(phase, {}).get("nonzero"),
            "observed": phases.get(phase, {}).get("nonzero"),
        }
    )
if not require_zero:
    for check in checks:
        if check["name"].endswith("-empty") and check["name"] != "pre-zero-empty":
            check["passed"] = True
            check["diagnostic_only"] = True
status = "PASS" if checks and all(item["passed"] for item in checks) else "FAIL"
scenario_dir = row / "scenarios" / "retention-cleanup"
scenario_dir.mkdir(parents=True, exist_ok=True)
scenario_log = scenario_dir / "rvoip.log"
scenario_log.write_bytes(log_path.read_bytes())
digest = hashlib.sha256(scenario_log.read_bytes()).hexdigest()
payload = {
    "schema": "rvoip-sip-proxy-interop-scenario-v1",
    "scenario": "retention-cleanup",
    "status": status,
    "evidence_kind": "retention-phase-snapshots",
    "external_peer_exercised": True,
    "peer": peer,
    "order": order,
    "transport": transport,
    "assertions": checks,
    "phases": phases,
    "inputs": {
        "rvoip.log": {"sha256": digest, "bytes": scenario_log.stat().st_size}
    },
}
(scenario_dir / "result.json").write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
post = phases.get("post_retention", {}).get("nonzero", {})
(row / "retention-check.txt").write_text(
    f"phase_count={len(phases)}\nnonzero={post}\n"
)
if status != "PASS":
    raise SystemExit(1)
PY
}

tls_proxy_target_authority() {
  local peer=$1
  local order=$2
  case "$order" in
    rvoip-first) tls_peer_identity "$peer" ;;
    peer-first) printf '%s\n' "$PROXY_INTEROP_TLS_SIPP_IDENTITY" ;;
    *) return 2 ;;
  esac
}

assert_peer_version() {
  local peer=$1
  local version_file=$2
  case "$peer" in
    kamailio)
      grep -Eq '^version: kamailio 6\.1\.3 \(x86_64/Linux\)[[:space:]]*$' \
        "$version_file"
      ;;
    opensips)
      grep -Eq '^version: opensips 3\.6\.7 \(x86_64/linux\)[[:space:]]*$' \
        "$version_file"
      ;;
    *) return 2 ;;
  esac
}

current_order=""
LAST_ROW_STATUS=FAIL
run_row() {
  local peer=$1
  local order=$2
  local transport=$3
  local next_hop next_port proxy_target uac_target
  local target_host target_port
  local started ended
  local row_status=PASS
  local scenarios=("${CORE_SCENARIOS[@]}")
  scenarios+=(retention-cleanup)
  if [[ "$transport" == tls ]]; then
    scenarios+=(sips-routing)
  fi

  current_transport=$transport
  current_order=$order
  row_dir="$ARTIFACT_DIR/$peer/$order/$transport"
  if ! mkdir -p "$row_dir/scenarios" "$row_dir/supplemental"; then
    echo "failed to create row artifact directory: $row_dir" >&2
    LAST_ROW_STATUS=FAIL
    return 0
  fi
  started=$(date +%s)
  active_peer=$peer
  export PROXY_INTEROP_PROJECT="rvoip-proxy-${peer//[^a-zA-Z0-9]/}-${order//[^a-zA-Z0-9]/}-$transport-$$"

  case "$order" in
    rvoip-first)
      next_hop=$HOST_ADDRESS
      next_port=$UAS_PORT
      proxy_target="$PEER_ADDRESS:$PROXY_INTEROP_PEER_PORT"
      uac_target="$HOST_ADDRESS:$RVOIP_PORT"
      ;;
    peer-first)
      next_hop=$HOST_ADDRESS
      next_port=$RVOIP_PORT
      proxy_target="$HOST_ADDRESS:$UAS_PORT"
      uac_target="$PEER_ADDRESS:$PROXY_INTEROP_PEER_PORT"
      ;;
    *)
      echo "unknown adjacency order: $order" >&2
      LAST_ROW_STATUS=FAIL
      return 0
      ;;
  esac
  if [[ "$transport" == tls ]]; then
    # SIPp stays on local TCP. The UAC boundary owns the first external mTLS
    # hop, the peer boundary owns the peer's outbound mTLS hop, and the UAS
    # boundary owns the final mTLS server hop.
    next_hop=$HOST_ADDRESS
    next_port=$TLS_PEER_BOUNDARY_PORT
    uac_target="$HOST_ADDRESS:$TLS_UAC_BOUNDARY_PORT"
    if [[ "$order" == peer-first ]]; then
      proxy_target="$HOST_ADDRESS:$TLS_UAS_BOUNDARY_PORT"
    fi
  fi
  target_host=${uac_target%:*}
  target_port=${uac_target##*:}

  if ! render_peer_config "$peer" "$transport" "$order" "$next_hop" "$next_port" \
    "$row_dir/$peer.cfg"; then
    echo "failed to render $peer configuration" >"$row_dir/harness-error.txt"
    row_status=FAIL
  fi
  export PROXY_INTEROP_PEER_CONFIG="$row_dir/$peer.cfg"
  if [[ "$row_status" == PASS ]] &&
    ! compose config >"$row_dir/compose-effective.yml"; then
    echo "failed to render effective Compose configuration" \
      >"$row_dir/harness-error.txt"
    row_status=FAIL
  fi

  local proxy_args=(
    --listen "0.0.0.0:$RVOIP_PORT"
    --advertised "$HOST_ADDRESS:$RVOIP_PORT"
    --target "$proxy_target"
    --transport "$transport"
  )
  if [[ "$order" == peer-first && "$transport" == udp &&
    "$RUN_ADVANCED_SCENARIOS" == 1 ]]; then
    proxy_args+=(
      --interop-test-mode
      --aux-target "$HOST_ADDRESS:$AUX_UAS_PORT_1"
      --aux-target "$HOST_ADDRESS:$AUX_UAS_PORT_2"
      --local-uri "sip:$HOST_ADDRESS:$RVOIP_PORT;transport=udp;lr"
      --record-route-sip "sip:$HOST_ADDRESS:$RVOIP_PORT;transport=udp;lr"
      --record-route-sips "sips:$HOST_ADDRESS:$RVOIP_PORT;transport=tls;lr"
    )
  fi
  if [[ "$order" == peer-first && "$transport" == tcp &&
    "$RUN_ADVANCED_SCENARIOS" == 1 ]]; then
    proxy_args+=(
      --interop-test-mode
      --aux-target "$HOST_ADDRESS:$AUX_UAS_PORT_1"
      --aux-target "$HOST_ADDRESS:$AUX_UAS_PORT_2"
      --failover-target "$HOST_ADDRESS:$DEAD_TARGET_PORT"
      --failover-target "$HOST_ADDRESS:$AUX_UAS_PORT_1"
      --dns-server "$HOST_ADDRESS:$RFC3263_DNS_PORT"
      --rfc3263-uri "sip:agent@failover.interop.test;transport=tcp"
      --timer-c-ms 500
      --max-response-contexts 64
      --local-uri "sip:$HOST_ADDRESS:$RVOIP_PORT;transport=tcp;lr"
      --record-route-sip "sip:$HOST_ADDRESS:$RVOIP_PORT;transport=tcp;lr"
      --record-route-sips "sips:$HOST_ADDRESS:$RVOIP_PORT;transport=tls;lr"
    )
  fi
  if [[ "$transport" == tls && "$row_status" == PASS ]]; then
    local target_authority
    if ! tls_proxy_args; then
      echo "failed to build rvoip TLS arguments" >"$row_dir/harness-error.txt"
      row_status=FAIL
    elif ! target_authority=$(tls_proxy_target_authority "$peer" "$order"); then
      echo "failed to select rvoip TLS target authority" \
        >"$row_dir/harness-error.txt"
      row_status=FAIL
    fi
  fi
  if [[ "$transport" == tls && "$row_status" == PASS ]]; then
    proxy_args+=(
      --target-authority "$target_authority"
      "${TLS_PROXY_ARGS[@]}"
    )
  fi

  if [[ "$row_status" == PASS && "$order" == peer-first &&
    "$transport" == tcp && "$RUN_ADVANCED_SCENARIOS" == 1 ]] &&
    ! start_rfc3263_dns; then
    row_status=FAIL
  fi

  if [[ "$row_status" == PASS ]]; then
    "$PROXY_BINARY" "${proxy_args[@]}" >"$row_dir/rvoip.log" 2>&1 &
    proxy_pid=$!
    wait_for_proxy_line RVOIP_PROXY_READY || row_status=FAIL
  fi
  if [[ "$row_status" == PASS ]]; then
    wait_for_proxy_line 'RVOIP_PROXY_RETENTION phase=pre_zero ' || row_status=FAIL
  fi

  if [[ "$row_status" == PASS ]] &&
    ! "$SCRIPT_DIR/up.sh" "$peer" "$row_dir/$peer.cfg" >"$row_dir/peer-start.log" 2>&1; then
    row_status=FAIL
  fi
  if [[ "$row_status" == PASS ]]; then
    local container_id
    if ! container_id=$(compose ps --quiet "$peer") || [[ -z "$container_id" ]]; then
      echo "peer container did not start" >"$row_dir/harness-error.txt"
      row_status=FAIL
    fi
    if [[ "$row_status" == PASS ]] &&
      ! docker inspect "$container_id" |
        python3 -B "$DOCKER_SNAPSHOT" --product "$peer" \
          >"$row_dir/peer-runtime.json"; then
      echo "failed to record peer runtime identity" >"$row_dir/harness-error.txt"
      row_status=FAIL
    fi
    if [[ "$row_status" == PASS && "$transport" == tls && "$peer" == opensips ]] &&
      ! tls_capture_opensips_image_provenance \
        "$INTEROP_DIR" "$container_id" "$row_dir"; then
      echo "failed to record OpenSIPS image provenance" >"$row_dir/harness-error.txt"
      row_status=FAIL
    fi
    if [[ "$row_status" == PASS ]] &&
      ! compose exec --no-TTY "$peer" sh -c "$(peer_version_command "$peer")" \
        >"$row_dir/peer-version.txt" 2>&1; then
      echo "failed to record peer version" >"$row_dir/harness-error.txt"
      row_status=FAIL
    fi
    if [[ "$row_status" == PASS ]] &&
      ! assert_peer_version "$peer" "$row_dir/peer-version.txt"; then
      echo "independent peer version does not match the reviewed release" \
        >"$row_dir/harness-error.txt"
      row_status=FAIL
    fi
  fi
  if [[ "$row_status" == PASS && "$transport" == tls ]] &&
    ! start_tls_boundaries "$peer" "$order"; then
    row_status=FAIL
  fi
  if [[ "$row_status" == PASS && "$transport" == tls ]]; then
    tls_verify_live_endpoints \
      "$row_dir" "$peer" "$order" "$PEER_ADDRESS:$PROXY_INTEROP_PEER_PORT" \
      "$HOST_ADDRESS:$RVOIP_PORT" || row_status=FAIL
  fi
  if [[ "$row_status" == PASS ]] &&
    ! run_core_scenarios "$target_host" "$target_port"; then
    row_status=FAIL
  fi
  if [[ "$row_status" == PASS && "$transport" == tls ]] &&
    ! run_captured_external_scenario sips-routing \
      run_sipp_scenario sips-routing \
      sips_routing_uac.xml sips_routing_uas.xml \
      "$target_host" "$target_port"; then
    row_status=FAIL
  fi
  if [[ "$row_status" == PASS && "$RUN_ADVANCED_SCENARIOS" == 1 ]]; then
    row_advanced=()
    while IFS= read -r scenario; do
      [[ -n "$scenario" ]] && row_advanced+=("$scenario")
    done < <(advanced_scenarios_for_row)
    if [[ ${#row_advanced[@]} -gt 0 ]]; then
      scenarios+=("${row_advanced[@]}")
    fi
    run_advanced_scenarios "$target_host" "$target_port" || row_status=FAIL
  fi

  if [[ "$row_status" == PASS ]]; then
    kill -USR1 "$proxy_pid" || row_status=FAIL
  fi
  if [[ "$row_status" == PASS ]]; then
    wait_for_proxy_line 'RVOIP_PROXY_RETENTION phase=activity ' || row_status=FAIL
  fi
  if [[ "$row_status" == PASS ]]; then
    decode_and_validate_pcaps || row_status=FAIL
  else
    stop_captures
  fi
  if [[ "$row_status" == PASS ]]; then
    sleep 1 || row_status=FAIL
  fi
  if [[ "$row_status" == PASS ]]; then
    kill -USR1 "$proxy_pid" || row_status=FAIL
  fi
  if [[ "$row_status" == PASS ]]; then
    wait_for_proxy_line 'RVOIP_PROXY_RETENTION phase=cooldown ' || row_status=FAIL
  fi
  if [[ "$row_status" == PASS ]]; then
    sleep "$RETENTION_DRAIN_SECONDS" || row_status=FAIL
  fi
  if [[ "$row_status" == PASS ]]; then
    kill -USR1 "$proxy_pid" || row_status=FAIL
  fi
  if [[ "$row_status" == PASS ]]; then
    wait_for_proxy_line 'RVOIP_PROXY_RETENTION phase=post_retention ' ||
      row_status=FAIL
  fi

  if [[ -n "$proxy_pid" ]] && kill -0 "$proxy_pid" >/dev/null 2>&1; then
    kill -INT "$proxy_pid" || row_status=FAIL
    wait "$proxy_pid" || row_status=FAIL
    proxy_pid=""
  else
    row_status=FAIL
  fi
  if [[ "$row_status" == PASS ]]; then
    parse_retention "$REQUIRE_RETENTION_CONVERGENCE" || row_status=FAIL
  fi

  stop_rfc3263_dns || row_status=FAIL
  stop_captures
  stop_pid "${uas_pid:-}" TERM
  uas_pid=""
  local aux_pid
  for aux_pid in "${aux_uas_pids[@]:-}"; do
    stop_pid "$aux_pid" TERM
  done
  aux_uas_pids=()
  stop_tls_boundaries
  compose logs --no-color "$peer" >"$row_dir/peer.log" 2>&1 || true
  if [[ "$transport" == tls && "$row_status" == PASS ]]; then
    tls_derive_verification_result "$row_dir" "$peer" "$order" || row_status=FAIL
  fi
  if [[ "$transport" == tls && "$row_status" == PASS ]]; then
    mkdir -p "$row_dir/scenarios/sips-routing"
    python3 -B "$SCENARIO_EVIDENCE" tls \
      --directory "$row_dir/scenarios/sips-routing" \
      --row-directory "$row_dir" \
      --peer "$peer" --order "$order" \
      --expected-rvoip-sent-by "$HOST_ADDRESS:$RVOIP_PORT" \
      --expected-peer-sent-by "$PEER_ADDRESS:$PROXY_INTEROP_PEER_PORT" ||
      row_status=FAIL
  fi
  if "$SCRIPT_DIR/down.sh" >"$row_dir/peer-stop.log" 2>&1; then
    active_peer=""
  else
    row_status=FAIL
  fi

  ended=$(date +%s)
  local scenario_csv
  scenario_csv=$(IFS=,; echo "${scenarios[*]}")
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$peer" "$order" "$transport" "$row_status" "$((ended - started))" \
    "${row_dir#"$ARTIFACT_DIR"/}" "$scenario_csv" >>"$MATRIX"
  row_dir=""
  LAST_ROW_STATUS=$row_status
  return 0
}

failures=0
for peer in $PEERS; do
  require_peer "$peer"
  for order in $ORDERS; do
    for transport in $TRANSPORTS; do
      echo "==> $peer / $order / $transport"
      LAST_ROW_STATUS=FAIL
      run_row "$peer" "$order" "$transport"
      if [[ "$LAST_ROW_STATUS" != PASS ]]; then
        failures=$((failures + 1))
        cleanup_row || true
        if [[ "$FAIL_FAST" == 1 ]]; then
          break 3
        fi
      fi
    done
  done
done

cleanup_row || true
tls_cleanup_pki

END_PROXY_BINARY_SHA256=$(sha256_file "$PROXY_BINARY" | awk '{print $1}')
proxy_binary_unchanged=false
if [[ "$PROXY_BINARY_SHA256" == "$END_PROXY_BINARY_SHA256" ]]; then
  proxy_binary_unchanged=true
fi
{
  echo "start_sha256=$PROXY_BINARY_SHA256"
  echo "end_sha256=$END_PROXY_BINARY_SHA256"
  echo "unchanged=$proxy_binary_unchanged"
} >"$ARTIFACT_DIR/proxy-binary-check.txt"
if [[ "$proxy_binary_unchanged" != true ]]; then
  failures=$((failures + 1))
fi

runtime_state_clean=false
: >"$ARTIFACT_DIR/runtime-state-settle.log"
for _ in $(seq 1 "$RUNTIME_STATE_SETTLE_ATTEMPTS"); do
  python3 -B "$STATE_SNAPSHOT" snapshot \
    --output "$ARTIFACT_DIR/runtime-state-end.json" \
    "${state_snapshot_args[@]}"
  if python3 -B "$STATE_SNAPSHOT" compare \
    --before "$ARTIFACT_DIR/runtime-state-start.json" \
    --after "$ARTIFACT_DIR/runtime-state-end.json" \
    --output "$ARTIFACT_DIR/runtime-state-check.json" \
    >>"$ARTIFACT_DIR/runtime-state-settle.log" 2>&1; then
    runtime_state_clean=true
    break
  fi
  sleep 0.1
done
if [[ "$runtime_state_clean" != true && "$REQUIRE_RUNTIME_STATE" == 1 ]]; then
  failures=$((failures + 1))
fi

end_source_sha=$(git -C "$WORKSPACE_ROOT" rev-parse HEAD)
end_source_status=$(git -C "$WORKSPACE_ROOT" status --short)
end_source_fingerprint=$(source_fingerprint_value)
source_unchanged=false
if [[ "$source_sha" == "$end_source_sha" &&
  "$source_fingerprint" == "$end_source_fingerprint" &&
  "$source_status" == "$end_source_status" ]]; then
  source_unchanged=true
fi
printf '%s\n' "$end_source_status" >"$ARTIFACT_DIR/source-status-end.txt"
{
  echo "start_sha=$source_sha"
  echo "end_sha=$end_source_sha"
  echo "start_fingerprint=$source_fingerprint"
  echo "end_fingerprint=$end_source_fingerprint"
  echo "unchanged=$source_unchanged"
} >"$ARTIFACT_DIR/source-check.txt"
if [[ "$REQUIRE_UNCHANGED_SOURCE" == 1 && "$source_unchanged" != true ]]; then
  failures=$((failures + 1))
fi

ENDED_AT_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)
python3 -B - \
  "$MATRIX" "$ARTIFACT_DIR/summary.md" "$ARTIFACT_DIR/summary.json" \
  "$STARTED_AT_UTC" "$ENDED_AT_UTC" \
  "$source_sha" "$end_source_sha" "$source_fingerprint" "$end_source_fingerprint" \
  "$source_dirty" "$source_unchanged" \
  "$KAMAILIO_IMAGE" "$OPENSIPS_IMAGE" "$KAMAILIO_PLATFORM" "$OPENSIPS_PLATFORM" \
  "$RETENTION_DRAIN_SECONDS" "$failures" <<'PY'
import csv
import json
import pathlib
import sys

matrix = pathlib.Path(sys.argv[1])
summary = pathlib.Path(sys.argv[2])
machine_summary = pathlib.Path(sys.argv[3])
rows = list(csv.DictReader(matrix.open(), delimiter="\t"))
for row in rows:
    row["scenarios"] = [item for item in row["scenarios"].split(",") if item]
passed = sum(row["status"] == "PASS" for row in rows)
failed = len(rows) - passed
lines = [
    "# Stateful proxy external interoperability matrix",
    "",
    f"- passed: {passed}",
    f"- failed: {failed}",
    "",
    "| Peer | Order | Transport | Status | Seconds | Scenarios | Evidence |",
    "|---|---|---|---:|---:|---:|---|",
]
for row in rows:
    lines.append(
        f"| {row['peer']} | {row['order']} | {row['transport']} | "
        f"{row['status']} | {row['duration_seconds']} | {len(row['scenarios'])} | "
        f"`{row['artifact']}` |"
    )
summary.write_text("\n".join(lines) + "\n")
payload = {
    "schema": "rvoip-sip-proxy-interop-v1",
    "started_at_utc": sys.argv[4],
    "ended_at_utc": sys.argv[5],
    "result": {
        "status": "PASS" if int(sys.argv[17]) == 0 else "FAIL",
        "passed_rows": passed,
        "failed_rows": failed,
        "gate_failures": int(sys.argv[17]),
    },
    "source": {
        "start_sha": sys.argv[6],
        "end_sha": sys.argv[7],
        "start_fingerprint_sha256": sys.argv[8],
        "end_fingerprint_sha256": sys.argv[9],
        "dirty_at_start": sys.argv[10] == "true",
        "unchanged": sys.argv[11] == "true",
    },
    "configuration": {
        "kamailio_image": sys.argv[12],
        "opensips_image": sys.argv[13],
        "kamailio_platform": sys.argv[14],
        "opensips_platform": sys.argv[15],
        "retention_drain_seconds": int(sys.argv[16]),
    },
    "rows": rows,
}
machine_summary.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
PY

trap - EXIT INT TERM
cat "$ARTIFACT_DIR/summary.md"
echo "Machine summary: $ARTIFACT_DIR/summary.json"
echo "Artifacts: $ARTIFACT_DIR"
if [[ "$failures" -ne 0 ]]; then
  exit 1
fi
