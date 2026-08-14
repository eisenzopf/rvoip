#!/usr/bin/env bash
#
# Per-rate AMR interop sweep: one live PBX cell per codec mode.
#
# Every ordinary AMR cell runs at whichever mode the encoder opens at, which is
# the highest the negotiation permits. That leaves the other rates with
# conformance-vector evidence and no third-party evidence at all. This pins one
# rate per cell the standard way -- `Config::amr_mode_set` puts an RFC 4867
# `mode-set` in the INVITE naming exactly that mode, reached from here through
# `PBX_AMR_MODE_SET` -- and records what came back.
#
# Two facts are captured per cell, because the interesting failure passes the
# obvious check: a cell pinned to one rate and a cell that ignored the pin both
# carry clean audio, so "the call passed" attests to nothing about the rate.
#
#   built_mode_set  the mode the codec was *actually constructed with*, read
#                   from media-core's `codec generation built` log line rather
#                   than from the environment variable meant to cause it.
#   tone            the analyser's far-tone-over-near-tone dominance for the
#                   cell, so a pinned rate that produced silence cannot pass.
#
# A row is only meaningful when `built_mode_set` equals `mode`. The script
# fails if any row disagrees, so a sweep cannot quietly attest to the wrong
# rate.
#
# Usage:
#   rate-sweep.sh [--profile amrnb|amrwb] [--transport UDP|TLS]
#                 [--pbx asterisk|freeswitch] [--modes 0,1,2] [--out FILE]
#
# Defaults sweep every mode the variant has: 0-7 narrowband, 0-8 wideband.
set -uo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
WORKSPACE_ROOT=$(cd "$SCRIPT_DIR/../../../../.." && pwd)
OUT_ROOT="$SCRIPT_DIR/output"

PROFILE=amrnb
TRANSPORT=UDP
PBX=asterisk
MODES=""
REPORT=""

while [ $# -gt 0 ]; do
  case "$1" in
    --profile) PROFILE=$2; shift 2 ;;
    --transport) TRANSPORT=$2; shift 2 ;;
    --pbx) PBX=$2; shift 2 ;;
    --modes) MODES=$2; shift 2 ;;
    --out) REPORT=$2; shift 2 ;;
    -h|--help)
      sed -n '2,30p' "$0"
      exit 0
      ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

case "$PROFILE" in
  amrnb) DEFAULT_MODES="0 1 2 3 4 5 6 7" ;;
  amrwb) DEFAULT_MODES="0 1 2 3 4 5 6 7 8" ;;
  *) echo "profile must be amrnb or amrwb, got '$PROFILE'" >&2; exit 2 ;;
esac
if [ -n "$MODES" ]; then
  SWEEP=$(printf '%s' "$MODES" | tr ',' ' ')
else
  SWEEP=$DEFAULT_MODES
fi
if [ -z "$REPORT" ]; then
  REPORT="$OUT_ROOT/rate-sweep-$PBX-$PROFILE-$TRANSPORT.tsv"
fi
mkdir -p "$(dirname "$REPORT")"

printf 'mode\tstatus\tbuilt_mode_set\ttone\n' >"$REPORT"
echo "AMR per-rate sweep: $PBX / $PROFILE / $TRANSPORT / modes: $SWEEP"

failures=0
for mode in $SWEEP; do
  cell="$OUT_ROOT/$PBX/endpoint/amr_call/$PROFILE/$TRANSPORT"
  # Clear the cell before the run. A cell that does not execute -- a transport
  # gated off, a lab that failed to come up -- leaves the previous run's logs
  # and WAVs in place, and reading those reports a rate as attested when
  # nothing ran. This script did exactly that on its first TLS attempt: it
  # read a tone from a cell four hours old and called it a pass.
  rm -rf "$cell"

  PBX_AMR_MODE_SET="$mode" PBX_CODEC_PROFILE="$PROFILE" \
    bash "$SCRIPT_DIR/run.sh" \
      --pbx "$PBX" --api endpoint --scenario amr_call \
      --transport "$TRANSPORT" --repeat 1 >/dev/null 2>&1

  status=$(awk -F'\t' 'NR>1 && $6=="caller" {print $1}' "$OUT_ROOT/matrix.tsv" 2>/dev/null | tail -1)
  # Strip the tracing crate's ANSI styling before reading the field.
  built=$(grep -a "codec generation built" "$cell/caller.log" 2>/dev/null \
    | sed 's/\x1b\[[0-9;]*m//g' | grep -a "codec=AMR" | tail -1 \
    | sed -n 's/.*mode_set="\([^"]*\)".*/\1/p')
  tone=$(grep -a "dominant over" "$cell/analyze.log" 2>/dev/null | head -1 \
    | sed -n 's/.*dominant over [0-9]*Hz by \([0-9.]*\)x.*/\1x/p')

  printf '%s\t%s\t%s\t%s\n' "$mode" "${status:-NO-ROW}" "${built:-none}" "${tone:-none}" >>"$REPORT"
  echo "  mode $mode: ${status:-NO-ROW} built=${built:-none} tone=${tone:-none}"

  if [ ! -f "$cell/caller.log" ]; then
    # No log at all means the cell never ran -- most often a transport this
    # provider or lab cannot do. Said plainly rather than counted as a
    # codec failure.
    echo "    FAIL: no cell ran for $TRANSPORT (nothing was executed to attest)" >&2
    failures=$((failures + 1))
  elif [ "${status:-}" != "PASS" ]; then
    echo "    FAIL: cell did not pass" >&2
    failures=$((failures + 1))
  elif [ "${built:-none}" != "$mode" ]; then
    # The check that makes this a per-rate claim rather than a pile of
    # passing calls: the codec must have been built at the mode we pinned.
    echo "    FAIL: pinned mode $mode but the codec was built with '${built:-none}'" >&2
    failures=$((failures + 1))
  elif [ -z "${tone:-}" ]; then
    echo "    FAIL: no tone analysis, so the audio at this rate is unverified" >&2
    failures=$((failures + 1))
  fi
done

echo
echo "report: $REPORT"
if [ "$failures" -ne 0 ]; then
  echo "$failures cell(s) failed" >&2
  exit 1
fi
echo "all $(printf '%s' "$SWEEP" | wc -w | tr -d ' ') modes passed at their pinned rate"
