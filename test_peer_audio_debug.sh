#!/bin/bash

# Clean up any existing output
rm -rf crates/session-core/examples/api_peer_audio/output
mkdir -p crates/session-core/examples/api_peer_audio/output

echo "Building examples..."
cargo build --example api_peer_audio_peer1 --example api_peer_audio_peer2

echo "Starting Bob (peer2) on port 5061 with debug logging..."
RUST_LOG=rvoip_session_core=info,debug RECORD_AUDIO=1 ./target/debug/examples/api_peer_audio_peer2 > /tmp/peer2_debug.log 2>&1 &
PEER2_PID=$!

sleep 2

echo "Starting Alice (peer1) on port 5060 with debug logging..."
RUST_LOG=rvoip_session_core=info,debug RECORD_AUDIO=1 ./target/debug/examples/api_peer_audio_peer1 > /tmp/peer1_debug.log 2>&1 &
PEER1_PID=$!

echo "Waiting for calls to complete (15 seconds)..."
sleep 15

# Kill any remaining processes
kill $PEER1_PID $PEER2_PID 2>/dev/null

echo "=== Key events from Peer1 (Alice) Log ==="
grep -E "audio_channels|Active|MediaFlow|media ready|media flow event" /tmp/peer1_debug.log | head -30

echo ""
echo "=== Key events from Peer2 (Bob) Log ==="
grep -E "audio_channels|Active|MediaFlow|media ready|media flow event" /tmp/peer2_debug.log | head -30

echo ""
echo "=== WAV Files Created ==="
ls -la crates/session-core/examples/api_peer_audio/output/ 2>/dev/null || echo "No output directory found"