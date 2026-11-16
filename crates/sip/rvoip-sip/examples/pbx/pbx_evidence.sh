#!/usr/bin/env sh
# PBX-side verdicts for the RTCP and session-timer cells (issue #318).
#
# rvoip's roles assert what rvoip observed. This script asserts what the PBX
# observed, from the PBX's own log of the cell (`pbx-log.txt`, captured by
# run.sh with the PBX's RTCP/RTP/SIP debugging switched on for the cell) and
# from rvoip's role logs, which name the local RTP port each role offered.
#
# Usage:
#   pbx_evidence.sh check <asterisk|freeswitch> <scenario> <UDP|TLS> <cell dir>
#   pbx_evidence.sh sip-summary < pbx-log.txt     # TSV of SIP messages
#
# Every check prints what it found; the last line is the verdict, and the
# exit status is non-zero when the PBX's view contradicts the cell's claim.

set -eu

fail() {
  echo "FAIL: $*"
  exit 1
}

# Drop the "[YYYY-MM-DD HH:MM:SS] " prefix Asterisk puts on console lines.
strip_prefix() {
  sed 's/^\[[0-9][0-9][0-9][0-9]-[0-9-]* [0-9:.]*\] //'
}

# The RTP port a role offered or answered with, from its own log.
local_rtp_port() {
  sed -n 's/.*Generated SDP for session .* with local port \([0-9][0-9]*\).*/\1/p' "$1" | head -1
}

# One TSV row per SIP message in an Asterisk `pjsip set logger` or
# FreeSWITCH `sofia global siptrace` log:
#   direction(rx|tx)  start-line  Call-ID  CSeq-number  CSeq-method  Session-Expires
# Headers that carry credentials are never read.
sip_summary() {
  strip_prefix | awk -F'\t' '
    function flush() {
      if (start != "") {
        printf "%s\t%s\t%s\t%s\t%s\t%s\n", dir, start, callid, cseqn, cseqm, se
      }
      start = ""; callid = ""; cseqn = ""; cseqm = ""; se = ""; inmsg = 0
    }
    /^<--- (Received|Transmitting) SIP (request|response)/ {
      flush(); dir = ($0 ~ /Received/) ? "rx" : "tx"; inmsg = 1; next
    }
    /^(recv|send) [0-9]+ bytes (from|to) / {
      flush(); dir = ($0 ~ /^recv/) ? "rx" : "tx"; inmsg = 1; next
    }
    !inmsg { next }
    /^-+$/ { if (start != "") { flush() } ; next }
    {
      line = $0; sub(/\r$/, "", line)
      if (start == "") {
        if (line != "") { start = line }
        next
      }
      if (line == "") { flush(); next }
      split(line, kv, ":")
      name = tolower(kv[1]); value = substr(line, length(kv[1]) + 2)
      sub(/^[ \t]+/, "", value)
      if (name == "call-id" || name == "i") { callid = value }
      else if (name == "cseq") { split(value, cs, " "); cseqn = cs[1]; cseqm = cs[2] }
      else if (name == "session-expires" || name == "x") { se = value }
    }
    END { flush() }
  '
}

# --- Asterisk ------------------------------------------------------------

asterisk_rtcp_check() {
  scenario=$1
  out_dir=$2
  log="$out_dir/pbx-log.txt"
  [ -s "$log" ] || fail "no PBX log captured ($log)"
  caller_port=$(local_rtp_port "$out_dir/caller.log")
  callee_port=$(local_rtp_port "$out_dir/callee.log")
  [ -n "$caller_port" ] && [ -n "$callee_port" ] \
    || fail "could not read the roles' local RTP ports from their logs"
  echo "rvoip RTP ports: caller=$caller_port callee=$callee_port"
  grep -q 'RTCP Debugging Enabled\|RTCP from\|Sent RTCP' "$log" \
    || echo "note: no RTCP debug output at all in the PBX log"

  for port in "$caller_port" "$callee_port"; do
    rtcp_port=$((port + 1))
    # Blocks of `rtcp set debug` output: "RTCP from <ip>:<port>" opens one,
    # "RTT: <secs>(sec)" inside it is what Asterisk computed from our RR.
    stats=$(strip_prefix <"$log" | awk -v rtp="$port" -v rtcp="$rtcp_port" '
      /^RTCP from / { n = split($3, a, ":"); from = a[n]; if (from == rtp) mux++; else if (from == rtcp) sep++; next }
      /^\* Sent RTCP (SR|RR) to / { n = split($NF, a, ":"); to = a[n]; if (to == rtp) to_mux++; else if (to == rtcp) to_sep++; next }
      /RTT: [0-9.]+\(sec\)/ {
        v = $0; sub(/.*RTT: /, "", v); sub(/\(sec\).*/, "", v)
        if (from == rtp || from == rtcp) { if (v + 0 > 0) rtt++ }
      }
      /^Got  RTP packet from / {
        n = split($5, a, ":"); src = a[n]
        if (src == rtp) { rtp_in++; t = $0; sub(/.*\(type /, "", t); sub(/,.*/, "", t); if (t + 0 >= 72 && t + 0 <= 76) rtcp_on_rtp++ }
      }
      END { printf "%d %d %d %d %d %d %d\n", mux, sep, to_mux, to_sep, rtt, rtp_in, rtcp_on_rtp }')
    set -- $stats
    from_rtp=$1 from_rtcp=$2 to_rtp=$3 to_rtcp=$4 rtt=$5 rtp_in=$6 rtcp_on_rtp=$7
    echo "port $port: RTCP from rvoip on RTP port=$from_rtp, on RTP+1=$from_rtcp;" \
      "Asterisk RTCP to RTP port=$to_rtp, to RTP+1=$to_rtcp; RTT>0 computed=$rtt;" \
      "RTP packets seen=$rtp_in, RTCP payload types on the RTP port=$rtcp_on_rtp"
    case "$scenario" in
      rtcp_mux)
        [ "$from_rtp" -gt 0 ] || fail "Asterisk received no RTCP from rvoip's RTP port $port"
        [ "$from_rtcp" -eq 0 ] || fail "rvoip sent RTCP from RTP+1 although mux was negotiated"
        [ "$to_rtp" -gt 0 ] || fail "Asterisk sent no RTCP to rvoip's RTP port $port"
        [ "$rtt" -gt 0 ] || fail "Asterisk computed no RTT from rvoip's reports on port $port"
        ;;
      rtcp_non_mux)
        [ "$from_rtcp" -gt 0 ] || fail "Asterisk received no RTCP from rvoip's RTCP port $rtcp_port"
        [ "$from_rtp" -eq 0 ] || fail "rvoip sent RTCP from its RTP port without mux"
        [ "$to_rtcp" -gt 0 ] || fail "Asterisk sent no RTCP to rvoip's RTCP port $rtcp_port"
        [ "$rtt" -gt 0 ] || fail "Asterisk computed no RTT from rvoip's reports on port $rtcp_port"
        ;;
      rtcp_mux_declined)
        [ "$((from_rtp + from_rtcp))" -eq 0 ] \
          || fail "Asterisk received RTCP from rvoip although rtcp_non_mux was off"
        # The control: debug output for this stream was on and RTP arrived.
        [ "$rtp_in" -gt 0 ] || fail "no RTP debug lines for port $port; the absence below proves nothing"
        [ "$rtcp_on_rtp" -eq 0 ] \
          || fail "RTCP (PT 200-204) arrived on Asterisk's RTP port from rvoip port $port"
        ;;
    esac
  done
  if grep -qi 'unprotect failed' "$log"; then
    fail "Asterisk failed to decrypt SRTP/SRTCP from rvoip"
  fi
  echo "PASS: Asterisk's own RTCP log agrees with $scenario"
}

asterisk_timer_check() {
  out_dir=$1
  log="$out_dir/pbx-log.txt"
  [ -s "$log" ] || fail "no PBX log captured ($log)"
  summary="$out_dir/pbx-sip-summary.tsv"
  sip_summary <"$log" >"$summary"
  [ -s "$summary" ] || fail "no SIP messages in the PBX log (pjsip logger off?)"
  timer_legs_check "$summary" Asterisk
}

# Both legs, from the PBX's side. Leg A is rvoip's call into the PBX (the
# PBX receives the INVITE); leg B is the PBX's call out to rvoip.
timer_legs_check() {
  summary=$1
  pbx=$2
  awk -F'\t' -v pbx="$pbx" '
    $5 == "INVITE" && $2 ~ /^INVITE / && $6 != "" {
      if ($1 == "rx" && a == "") { a = $3 }
      if ($1 == "tx" && b == "") { b = $3 }
    }
    { rows[NR] = $0 }
    END {
      if (a == "") { print "FAIL: " pbx " saw no timer-capable INVITE from rvoip"; exit 1 }
      if (b == "") { print "FAIL: " pbx " sent no timer-capable INVITE to rvoip"; exit 1 }
      for (i = 1; i <= NR; i++) {
        split(rows[i], f, "\t")
        if (f[5] == "INVITE" && f[2] ~ /^SIP\/2.0 2/) {
          if (f[3] == a && f[1] == "tx" && (a2xx == "" || f[4] + 0 < a2xx + 0)) { a2xx = f[4]; ase = f[6] }
          if (f[3] == b && f[1] == "rx" && (b2xx == "" || f[4] + 0 < b2xx + 0)) { b2xx = f[4]; bse = f[6] }
        }
      }
      for (i = 1; i <= NR; i++) {
        split(rows[i], f, "\t")
        if (f[2] !~ /^(UPDATE|INVITE) / || f[6] == "") continue
        for (j = i + 1; j <= NR; j++) {
          split(rows[j], g, "\t")
          if (g[3] == f[3] && g[4] == f[4] && g[5] == f[5] && g[2] ~ /^SIP\/2.0 2/ && g[6] != "") {
            if (f[3] == a && f[1] == "rx" && f[4] + 0 > a2xx + 0) arefresh++
            if (f[3] == b && f[1] == "tx" && f[4] + 0 > b2xx + 0) brefresh++
            break
          }
        }
      }
      printf "leg A (rvoip -> %s): 2xx Session-Expires \"%s\"; refreshes from rvoip accepted by %s: %d\n", pbx, ase, pbx, arefresh
      printf "leg B (%s -> rvoip): 2xx Session-Expires \"%s\"; refreshes by %s accepted by rvoip: %d\n", pbx, bse, pbx, brefresh
      if (tolower(ase) !~ /refresher=uac/) { print "FAIL: leg A did not make rvoip (its UAC) the refresher"; exit 1 }
      if (tolower(bse) !~ /refresher=uac/) { print "FAIL: leg B did not make " pbx " (its UAC) the refresher"; exit 1 }
      if (arefresh < 1) { print "FAIL: " pbx " never received a refresh from rvoip"; exit 1 }
      if (brefresh < 1) { print "FAIL: rvoip never accepted a refresh from " pbx; exit 1 }
      print "PASS: " pbx " saw rvoip refresh leg A and refreshed leg B itself"
    }' "$summary"
}

# --- FreeSWITCH ----------------------------------------------------------

freeswitch_timer_check() {
  out_dir=$1
  log="$out_dir/pbx-log.txt"
  [ -s "$log" ] || fail "no PBX log captured ($log)"
  summary="$out_dir/pbx-sip-summary.tsv"
  sip_summary <"$log" >"$summary"
  [ -s "$summary" ] || fail "no SIP messages in the PBX log (siptrace off?)"
  # The 422 is FreeSWITCH's: Min-SE 120 against rvoip's 90 s offer.
  awk -F'\t' '$1 == "tx" && $2 ~ /^SIP\/2.0 422/ && $5 == "INVITE" { found = 1 }
    END { if (!found) { print "FAIL: FreeSWITCH sent no 422 to the 90 s offer"; exit 1 }
          print "FreeSWITCH answered the 90 s offer with 422" }' "$summary"
  timer_legs_check "$summary" FreeSWITCH
}

# FreeSWITCH's view: `uuid_dump` of both channels while the call was up
# (its own SDP and rvoip's, URL-encoded) and the RECV_RTCP_MESSAGE events it
# raised for every report it accepted. A report whose LSR is non-zero echoes
# one of FreeSWITCH's own SRs, i.e. rvoip received FreeSWITCH's RTCP and
# answered it; Rtt0-Avg, FreeSWITCH's averaged round trip, is reported only.
freeswitch_rtcp_check() {
  out_dir=$1
  snapshot="$out_dir/pbx-snapshot.txt"
  events="$out_dir/pbx-rtcp-events.txt"
  [ -s "$snapshot" ] || fail "no PBX snapshot captured"
  [ -s "$events" ] || fail "no RECV_RTCP_MESSAGE capture"
  channels=$(awk '/^## uuid_dump / { print $3 }' "$snapshot")
  [ "$(printf '%s\n' "$channels" | grep -c .)" -eq 2 ] \
    || fail "expected the call's two channels in the snapshot, got: $(echo $channels)"
  for uuid in $channels; do
    sdp=$(awk -v u="$uuid" '
      /^## uuid_dump / { cur = $3; next }
      cur == u && /^variable_rtp_local_sdp_str: / { if ($0 ~ /a%3Drtcp-mux/) local_mux = 1 }
      cur == u && /^variable_switch_r_sdp: / { if ($0 ~ /a%3Drtcp-mux/) remote_mux = 1 }
      cur == u && /^Call-Direction: / { dir = $2 }
      END { printf "%s %d %d\n", dir, local_mux, remote_mux }' "$snapshot")
    set -- $sdp
    direction=$1 local_mux=$2 remote_mux=$3
    stats=$(awk -F': ' -v u="$uuid" '
      /^Event-Name: / { inev = ($2 == "RECV_RTCP_MESSAGE"); uid = ""; next }
      !inev { next }
      /^Unique-ID: / { uid = $2; if (uid == u) n++; next }
      uid == u && /^Source0-LSR: / { if ($2 + 0 != 0) lsr++ }
      uid == u && /^Rtt0-Avg: / { if ($2 + 0 > 0) rtt++ }
      END { printf "%d %d %d\n", n, lsr, rtt }' "$events")
    set -- $stats
    reports=$1 echoed=$2 rtt=$3
    echo "channel $uuid ($direction): FreeSWITCH SDP a=rtcp-mux=$local_mux, rvoip SDP a=rtcp-mux=$remote_mux;" \
      "RTCP reports from rvoip=$reports, echoing FreeSWITCH SRs=$echoed, RTT>0 computed=$rtt"
    [ "$remote_mux" -eq 1 ] || fail "rvoip's SDP on $direction channel lacks a=rtcp-mux"
    [ "$local_mux" -eq 1 ] || fail "FreeSWITCH did not multiplex RTCP on its $direction channel"
    [ "$reports" -gt 0 ] || fail "FreeSWITCH accepted no RTCP from rvoip on its $direction channel"
    # Not Rtt0-Avg: FreeSWITCH reports 0 until it has averaged a few
    # echoes, which a short call may never reach. A non-zero LSR is the
    # direct proof that rvoip received FreeSWITCH's SR and reported on it.
    [ "$echoed" -gt 0 ] || fail "rvoip never echoed a FreeSWITCH SR on the $direction channel"
  done
  echo "PASS: FreeSWITCH multiplexed RTCP with rvoip on both legs and exchanged reports"
}

check() {
  provider=$1
  scenario=$2
  transport=$3
  out_dir=$4
  echo "provider=$provider scenario=$scenario transport=$transport"
  case "$provider:$scenario" in
    asterisk:rtcp_mux|asterisk:rtcp_non_mux|asterisk:rtcp_mux_declined)
      asterisk_rtcp_check "$scenario" "$out_dir"
      ;;
    asterisk:session_timer) asterisk_timer_check "$out_dir" ;;
    freeswitch:rtcp_mux) freeswitch_rtcp_check "$out_dir" ;;
    freeswitch:session_timer) freeswitch_timer_check "$out_dir" ;;
    *) fail "no PBX-side check for $provider $scenario" ;;
  esac
}

case "${1:-}" in
  check)
    shift
    check "${1:?provider}" "${2:?scenario}" "${3:?transport}" "${4:?cell dir}"
    ;;
  sip-summary)
    sip_summary
    ;;
  *)
    echo "usage: pbx_evidence.sh check <provider> <scenario> <transport> <cell dir> | sip-summary" >&2
    exit 2
    ;;
esac
