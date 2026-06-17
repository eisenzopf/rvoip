#!/usr/bin/env bash
# Drive the rvoip→Amazon Connect gateway with SIPp: place a call, play a G.711
# tone into it (you should hear it on the agent CCP), capture the agent's return
# audio, and decode it to return.wav (proves the Opus→G.711 transcode).
#
#   ./run-sipp-test.sh           # call 127.0.0.1:5060, media port 6000
#   DEST=1.2.3.4:5060 ./run-sipp-test.sh
#
# Needs sudo for tcpdump (loopback capture). Start the gateway first in another
# terminal: ./run-gateway.sh
set -euo pipefail
cd "$(dirname "$0")"

DEST="${DEST:-127.0.0.1:5060}"
MP="${MP:-6000}"
IFACE="${IFACE:-lo0}"
CAP="sipp_capture.pcap"

[ -f tone_pcmu.pcap ] || ./make-tone.sh 12

echo "→ capturing return RTP on $IFACE udp/$MP (sudo tcpdump)…"
sudo tcpdump -i "$IFACE" -w "$CAP" "udp port $MP" 2>/dev/null &
TPID=$!
sleep 1

echo "→ placing 1 call to $DEST (media port $MP)."
echo "  *** ANSWER the agent CCP when it rings, and TALK so we capture return audio. ***"
# sudo: SIPp's play_pcap_audio sends RTP via raw sockets, which needs root.
sudo sipp "$DEST" -sf uac_tone.xml -m 1 -mp "$MP" -l 1 -nostdin -timeout 60 -trace_err || true

sleep 1
sudo kill "$TPID" 2>/dev/null || true
wait "$TPID" 2>/dev/null || true

echo
echo "→ RTP streams seen in capture:"
tshark -r "$CAP" -o rtp.heuristic_rtp:TRUE -q -z rtp,streams 2>/dev/null || true

echo
echo "→ decoding gateway→sipp audio (PCMU PT0 toward udp/$MP) to return.wav…"
tshark -r "$CAP" -o rtp.heuristic_rtp:TRUE \
  -Y "rtp && udp.dstport==$MP && rtp.p_type==0" -T fields -e rtp.payload 2>/dev/null \
  | tr -d ':\n' | xxd -r -p > return.ulaw || true

if [ -s return.ulaw ]; then
  sox -t ul -r 8000 -c 1 return.ulaw return.wav
  echo "✅ wrote return.wav ($(wc -c < return.ulaw | tr -d ' ') µ-law bytes ≈ $(( $(wc -c < return.ulaw) / 8000 ))s)"
  echo "   listen:  afplay return.wav"
else
  echo "⚠ no PCMU RTP captured toward sipp."
  echo "  Likely the agent didn't answer/talk, or media didn't bridge. Check the gateway logs."
fi
