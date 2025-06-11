#!/bin/bash

# SIPp to SIPp Audio Streaming Test
# This tests basic SIPp RTP audio functionality without any conference server

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "🧪 Starting SIPp-to-SIPp Audio Streaming Test"
echo "============================================"

# Clean up any existing logs
rm -f scenarios/sipp_to_rust/simple_*.log

echo "📞 Starting UAS (answering side) on port 5080..."
sudo sipp -sf scenarios/sipp_to_rust/simple_uas.xml -p 5080 -i 127.0.0.1 -m 1 \
  -trace_stat -stf scenarios/sipp_to_rust/simple_uas_stats.csv \
  -trace_msg -message_file scenarios/sipp_to_rust/simple_uas_messages.log &
UAS_PID=$!

# Give UAS time to start
sleep 2

echo "📞 Starting UAC (calling side) to 127.0.0.1:5080..."
sudo sipp -sf scenarios/sipp_to_rust/simple_uac.xml 127.0.0.1:5080 -m 1 \
  -trace_stat -stf scenarios/sipp_to_rust/simple_uac_stats.csv \
  -trace_msg -message_file scenarios/sipp_to_rust/simple_uac_messages.log

echo "🔍 Waiting for UAS to complete..."
wait $UAS_PID

echo ""
echo "📊 Test Results:"
echo "================"

# Check statistics files for RTP data
echo "🎵 UAC Statistics:"
if [ -f scenarios/sipp_to_rust/simple_uac_stats.csv ]; then
    echo "  📊 Stats file created ($(wc -l < scenarios/sipp_to_rust/simple_uac_stats.csv) lines)"
    tail -1 scenarios/sipp_to_rust/simple_uac_stats.csv
else
    echo "  ❌ No UAC stats file found"
fi

echo ""
echo "🎵 UAS Statistics:"
if [ -f scenarios/sipp_to_rust/simple_uas_stats.csv ]; then
    echo "  📊 Stats file created ($(wc -l < scenarios/sipp_to_rust/simple_uas_stats.csv) lines)"
    tail -1 scenarios/sipp_to_rust/simple_uas_stats.csv
else
    echo "  ❌ No UAS stats file found"
fi

echo ""
echo "📋 SIP Messages:"
echo "  UAC Messages: $(wc -l < scenarios/sipp_to_rust/simple_uac_messages.log 2>/dev/null || echo 0) lines"
echo "  UAS Messages: $(wc -l < scenarios/sipp_to_rust/simple_uas_messages.log 2>/dev/null || echo 0) lines"

echo ""
echo "✅ SIPp-to-SIPp test complete!" 