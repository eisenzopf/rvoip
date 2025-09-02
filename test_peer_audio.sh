#!/bin/bash

# Clean up any existing output
rm -rf crates/session-core/examples/api_peer_audio/output
mkdir -p crates/session-core/examples/api_peer_audio/output

echo "Starting Bob (peer2) on port 5061..."
RECORD_AUDIO=1 ./target/debug/examples/api_peer_audio_peer2 > /tmp/peer2.log 2>&1 &
PEER2_PID=$!

sleep 2

echo "Starting Alice (peer1) on port 5060..."
RECORD_AUDIO=1 ./target/debug/examples/api_peer_audio_peer1 > /tmp/peer1.log 2>&1 &
PEER1_PID=$!

echo "Waiting for calls to complete..."
sleep 5

# Kill any remaining processes
kill $PEER1_PID $PEER2_PID 2>/dev/null

echo "Checking results..."
echo "=== Peer1 (Alice) Log ==="
tail -20 /tmp/peer1.log

echo ""
echo "=== Peer2 (Bob) Log ==="
tail -20 /tmp/peer2.log

echo ""
echo "=== WAV Files Created ==="
ls -la crates/session-core/examples/api_peer_audio/output/ 2>/dev/null || echo "No output directory found"