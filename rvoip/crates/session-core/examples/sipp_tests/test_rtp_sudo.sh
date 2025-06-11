#!/bin/bash

# SIPp RTP Test with Root Privileges
# Tests if sudo actually enables RTP packet generation

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "🧪 Testing SIPp RTP with Root Privileges"
echo "========================================"

# Clean up any existing processes
pkill -f "sipp.*simple" 2>/dev/null || true
sleep 1

echo "📞 Starting UAS with sudo on port 5080..."
sudo sipp -sf scenarios/sipp_to_rust/simple_uas.xml -p 5080 -i 127.0.0.1 -m 1 \
  -trace_stat -stf /tmp/uas_rtp_stats.csv &
UAS_PID=$!

# Give UAS time to start
sleep 3

echo "📞 Starting UAC with sudo to 127.0.0.1:5080..."
sudo sipp -sf scenarios/sipp_to_rust/simple_uac.xml 127.0.0.1:5080 -m 1 \
  -trace_stat -stf /tmp/uac_rtp_stats.csv

echo "🔍 Waiting for UAS to complete..."
wait $UAS_PID

echo ""
echo "📊 RTP Results:"
echo "==============="

# Check for RTP statistics in the output
echo "🎵 UAC RTP Results:"
if [ -f /tmp/uac_rtp_stats.csv ]; then
    # Get the last line which has final stats
    LAST_LINE=$(tail -1 /tmp/uac_rtp_stats.csv)
    echo "  Stats captured: $(wc -l < /tmp/uac_rtp_stats.csv) lines"
    echo "  Final stats: $LAST_LINE"
else
    echo "  ❌ No UAC stats file found"
fi

echo ""
echo "🎵 UAS RTP Results:"
if [ -f /tmp/uas_rtp_stats.csv ]; then
    LAST_LINE=$(tail -1 /tmp/uas_rtp_stats.csv)
    echo "  Stats captured: $(wc -l < /tmp/uas_rtp_stats.csv) lines"
    echo "  Final stats: $LAST_LINE"
else
    echo "  ❌ No UAS stats file found"
fi

echo ""
echo "🔍 Key Question: Do we see non-zero RTP packet counts with sudo?"
echo "✅ Test complete!"

# Clean up temp files
rm -f /tmp/uac_rtp_stats.csv /tmp/uas_rtp_stats.csv 