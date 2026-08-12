#!/usr/bin/env sh
# Does this PBX support AMR? Asked before every AMR interop cell, because the
# answer differs by *image*, not by provider: the local lab containers carry
# AMR (Asterisk source-built with the traud patches, FreeSWITCH with
# mod_amr/mod_amrwb) while the committed release-runner images do not — the
# packaged Alpine Asterisk cannot take the patches at all.
#
# A cell that would fail for "this image has no AMR" is not interop evidence
# of anything, so the runner records it as SKIP instead. Two guards keep the
# skip from becoming rot:
#   - PBX_ASSUME_AMR=0|1 short-circuits the probe (the release gates pin 0,
#     so gate behaviour never depends on a docker exec);
#   - PBX_REQUIRE_AMR=1 (set in the AMR-capable labs' env files) turns any
#     skip into a loud failure, so a lab regression cannot hide as a skip.
#
# Split into `parse` (pure, stdin -> answer, unit-tested against captured
# fixtures in probe-fixtures/) and `detect` (environment + docker), so the
# part that can silently rot is the part CI pins.
#
# Usage:
#   amr_probe.sh parse  <asterisk|freeswitch>              # stdin: CLI output
#   amr_probe.sh detect <asterisk|freeswitch> <transcript> # env + docker
#
# Both print one line: `status=<probed|assumed|unknown> amr=<yes|no> amrwb=<yes|no>`.

set -eu

parse_asterisk() {
  # `core show codecs` rows: `      ID TYPE  NAME  FORMAT  (DESCRIPTION)`,
  # e.g. `       1 audio amr          amr              (AMR)`. Match the NAME
  # column exactly; substring matching would let `amrwb` satisfy `amr`.
  amr=no
  amrwb=no
  while IFS= read -r line; do
    set -- $line
    [ "$#" -ge 3 ] || continue
    if [ "$2" = "audio" ]; then
      case "$3" in
        amr) amr=yes ;;
        amrwb) amrwb=yes ;;
      esac
    fi
  done
  printf 'amr=%s amrwb=%s\n' "$amr" "$amrwb"
}

parse_freeswitch() {
  # `show codec` emits CSV rows `codec,<name>,<module>`; anchor on the module
  # column, which names the implementation rather than the SDP string:
  #   codec,AMR / Octet Aligned,mod_amr
  #   codec,AMR-WB / Bandwidth Efficient,mod_amrwb
  amr=no
  amrwb=no
  while IFS= read -r line; do
    case "$line" in
      codec,*,mod_amr) amr=yes ;;
      codec,*,mod_amrwb) amrwb=yes ;;
    esac
  done
  printf 'amr=%s amrwb=%s\n' "$amr" "$amrwb"
}

parse() {
  case "$1" in
    asterisk) parse_asterisk ;;
    freeswitch) parse_freeswitch ;;
    *) echo "amr_probe.sh: unknown provider '$1'" >&2; exit 2 ;;
  esac
}

first_running_container() {
  for candidate in "$@"; do
    if docker inspect --format '{{.State.Running}}' "$candidate" 2>/dev/null | grep -q true; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  return 1
}

detect() {
  provider=$1
  transcript=$2

  if [ -n "${PBX_ASSUME_AMR:-}" ]; then
    case "$PBX_ASSUME_AMR" in
      1|true|yes|on) answer=yes ;;
      *) answer=no ;;
    esac
    {
      echo "decision: assumed from PBX_ASSUME_AMR=$PBX_ASSUME_AMR"
    } >"$transcript"
    printf 'status=assumed amr=%s amrwb=%s\n' "$answer" "$answer"
    return 0
  fi

  if ! command -v docker >/dev/null 2>&1; then
    echo "decision: unknown (docker not on PATH)" >"$transcript"
    printf 'status=unknown amr=no amrwb=no\n'
    return 0
  fi

  case "$provider" in
    asterisk)
      candidates="${PBX_ASTERISK_CONTAINER:-rvoip-asterisk rvoip-release-asterisk}"
      # shellcheck disable=SC2086
      if ! container=$(first_running_container $candidates); then
        echo "decision: unknown (no running container among: $candidates)" >"$transcript"
        printf 'status=unknown amr=no amrwb=no\n'
        return 0
      fi
      output=$(docker exec "$container" asterisk -rx "core show codecs" 2>&1) || {
        {
          echo "decision: unknown (CLI failed in $container)"
          printf '%s\n' "$output"
        } >"$transcript"
        printf 'status=unknown amr=no amrwb=no\n'
        return 0
      }
      ;;
    freeswitch)
      candidates="${PBX_FREESWITCH_CONTAINER:-rvoip-freeswitch}"
      # shellcheck disable=SC2086
      if ! container=$(first_running_container $candidates); then
        echo "decision: unknown (no running container among: $candidates)" >"$transcript"
        printf 'status=unknown amr=no amrwb=no\n'
        return 0
      fi
      output=$(docker exec "$container" fs_cli -p "${FREESWITCH_EVENT_SOCKET_PASSWORD:-ClueCon}" -x "show codec" 2>&1) || {
        {
          echo "decision: unknown (CLI failed in $container)"
          printf '%s\n' "$output"
        } >"$transcript"
        printf 'status=unknown amr=no amrwb=no\n'
        return 0
      }
      ;;
    *)
      echo "amr_probe.sh: unknown provider '$provider'" >&2
      exit 2
      ;;
  esac

  answer=$(printf '%s\n' "$output" | parse "$provider")
  {
    echo "decision: probed via $container"
    echo "parsed: $answer"
    echo "--- raw CLI output ---"
    printf '%s\n' "$output"
  } >"$transcript"
  printf 'status=probed %s\n' "$answer"
}

case "${1:-}" in
  parse)
    shift
    parse "${1:?provider}"
    ;;
  detect)
    shift
    detect "${1:?provider}" "${2:?transcript path}"
    ;;
  *)
    echo "usage: amr_probe.sh parse <provider> | detect <provider> <transcript>" >&2
    exit 2
    ;;
esac
