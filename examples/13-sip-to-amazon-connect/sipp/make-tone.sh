#!/usr/bin/env bash
# Generate tone_pcmu.pcap: a 440 Hz tone as G.711 mu-law (PCMU/PT 0) RTP,
# replayable by SIPp's play_pcap_audio. No sudo/ffmpeg/capture needed.
#   ./make-tone.sh [seconds]   (default 12)
set -euo pipefail
cd "$(dirname "$0")"
DUR="${1:-12}"
python3 make_tone.py "$DUR" tone.txt
text2pcap -q -u 5004,5004 -t "%H:%M:%S." tone.txt tone_pcmu.pcap
rm -f tone.txt
echo "wrote $(pwd)/tone_pcmu.pcap"
