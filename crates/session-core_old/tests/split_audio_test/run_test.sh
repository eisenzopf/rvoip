#!/bin/bash

# Clean up any existing output
rm -rf tests/split_audio_test

echo "Starting split audio test..."
echo "==============================="

# Start peer_b (UAS) in background
echo "Starting Peer B (UAS) on port 5061..."
RUST_LOG=info cargo run --bin peer_b &
PEER_B_PID=$!

# Give peer_b time to start and be ready
sleep 3

# Start peer_a (UAC)
echo "Starting Peer A (UAC) on port 5060..."
RUST_LOG=info cargo run --bin peer_a

# Wait for peer_a to complete
PEER_A_EXIT=$?

# Give a moment for peer_b to finish
sleep 2

# Kill peer_b if still running
kill $PEER_B_PID 2>/dev/null

echo ""
echo "Test completed!"
echo "Check audio files in: tests/split_audio_test/"

exit $PEER_A_EXIT