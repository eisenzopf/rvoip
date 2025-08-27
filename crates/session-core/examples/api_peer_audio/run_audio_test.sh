#!/bin/bash

# Audio exchange test runner
# This script orchestrates running both peers and optionally records audio

echo "🎵 SimplePeer Audio Exchange Test"
echo "================================="
echo ""

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Check if we should record audio
if [ "$1" == "--record" ]; then
    export RECORD_AUDIO=1
    echo -e "${BLUE}📼 Recording enabled - WAV files will be saved${NC}"
fi

# Build the examples first
echo -e "${BLUE}🔨 Building examples...${NC}"
cargo build --example api_peer_audio_peer1 --example api_peer_audio_peer2 -p rvoip-session-core 2>&1 | grep -E "(Compiling|Finished)" || {
    echo -e "${RED}❌ Build failed${NC}"
    exit 1
}

# Clean up any previous output
if [ -d "output" ]; then
    rm -rf output
fi

# Start Bob (peer2) in the background
echo -e "${GREEN}▶️  Starting Bob (peer2) on port 5061...${NC}"
cargo run --example api_peer_audio_peer2 -p rvoip-session-core 2>&1 | sed 's/^/[BOB] /' &
BOB_PID=$!

# Give Bob time to start listening
sleep 2

# Start Alice (peer1)
echo -e "${GREEN}▶️  Starting Alice (peer1) on port 5060...${NC}"
cargo run --example api_peer_audio_peer1 -p rvoip-session-core 2>&1 | sed 's/^/[ALICE] /' &
ALICE_PID=$!

# Wait for both to complete
wait $ALICE_PID
ALICE_EXIT=$?

wait $BOB_PID
BOB_EXIT=$?

echo ""
echo "================================="

# Check results
if [ $ALICE_EXIT -eq 0 ] && [ $BOB_EXIT -eq 0 ]; then
    echo -e "${GREEN}✅ Test completed successfully!${NC}"
    
    if [ "$RECORD_AUDIO" == "1" ]; then
        echo ""
        echo "📁 Audio files saved to: examples/api_peer_audio/output/"
        echo "   - alice_sent.wav     (440Hz tone)"
        echo "   - alice_received.wav (should be 880Hz from Bob)"
        echo "   - bob_sent.wav       (880Hz tone)"
        echo "   - bob_received.wav   (should be 440Hz from Alice)"
        echo ""
        echo "🎧 You can verify the audio with:"
        echo "   ffplay output/alice_sent.wav"
        echo "   ffplay output/bob_received.wav"
    fi
else
    echo -e "${RED}❌ Test failed${NC}"
    [ $ALICE_EXIT -ne 0 ] && echo "   Alice exit code: $ALICE_EXIT"
    [ $BOB_EXIT -ne 0 ] && echo "   Bob exit code: $BOB_EXIT"
    exit 1
fi