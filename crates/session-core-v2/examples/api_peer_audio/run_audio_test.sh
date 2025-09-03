#!/bin/bash

# Audio exchange test runner for session-core-v2
# This script orchestrates running both peers

echo "🎵 Session-Core-V2 Audio Exchange Test"
echo "======================================"
echo ""

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[0;33m'
NC='\033[0m' # No Color

# Parse arguments
DEBUG_MODE=0
RECORD_MODE=0
for arg in "$@"; do
    case $arg in
        --record)
            export RECORD_AUDIO=1
            RECORD_MODE=1
            echo -e "${BLUE}📼 Recording enabled - Audio files will be saved${NC}"
            ;;
        --debug)
            export RUST_LOG=rvoip_session_core_v2=info
            DEBUG_MODE=1
            echo -e "${YELLOW}🔍 Debug logging enabled${NC}"
            ;;
        --trace)
            export RUST_LOG=rvoip_session_core_v2=debug
            DEBUG_MODE=1
            echo -e "${YELLOW}🔍 Trace logging enabled${NC}"
            ;;
        --help)
            echo "Usage: $0 [--record] [--debug|--trace]"
            echo "  --record  Save audio to files"
            echo "  --debug   Enable info-level logging"
            echo "  --trace   Enable debug-level logging"
            exit 0
            ;;
    esac
done

# Create log directory
LOG_DIR="logs"
mkdir -p $LOG_DIR
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
ALICE_LOG="$LOG_DIR/alice_${TIMESTAMP}.log"
BOB_LOG="$LOG_DIR/bob_${TIMESTAMP}.log"

# Build the examples first
echo -e "${BLUE}🔨 Building examples...${NC}"
cargo build --example api_peer_audio_peer1 --example api_peer_audio_peer2 -p rvoip-session-core-v2 2>&1 | grep -E "(Compiling|Finished)" || {
    echo -e "${RED}❌ Build failed${NC}"
    exit 1
}

# Clean up any previous output
if [ -d "output" ]; then
    rm -rf output
fi
mkdir -p output

# Get the binary path
# We're in crates/session-core-v2/examples/api_peer_audio, need to go up to project root
PROJECT_ROOT="$(cd ../../../.. && pwd)"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PROJECT_ROOT/target}"
PEER1_BIN="$CARGO_TARGET_DIR/debug/examples/api_peer_audio_peer1"
PEER2_BIN="$CARGO_TARGET_DIR/debug/examples/api_peer_audio_peer2"

# Check if binaries exist
if [ ! -f "$PEER1_BIN" ] || [ ! -f "$PEER2_BIN" ]; then
    echo -e "${RED}❌ Example binaries not found. Build may have failed.${NC}"
    echo "Looking for:"
    echo "  - $PEER1_BIN"
    echo "  - $PEER2_BIN"
    exit 1
fi

# Start Bob (peer2) in the background
echo -e "${GREEN}▶️  Starting Bob (peer2) on port 5061...${NC}"
$PEER2_BIN > >(tee "$BOB_LOG" | sed 's/^/[BOB] /') 2>&1 &
BOB_PID=$!

# Give Bob time to start listening
sleep 2

# Start Alice (peer1)
echo -e "${GREEN}▶️  Starting Alice (peer1) on port 5060...${NC}"
$PEER1_BIN > >(tee "$ALICE_LOG" | sed 's/^/[ALICE] /') 2>&1 &
ALICE_PID=$!

# Function to check if process is still running
check_timeout() {
    local pid=$1
    local name=$2
    local timeout=$3
    local elapsed=0
    
    while [ $elapsed -lt $timeout ]; do
        if ! kill -0 $pid 2>/dev/null; then
            # Process finished
            return 0
        fi
        sleep 1
        elapsed=$((elapsed + 1))
        
        # Show progress
        if [ $((elapsed % 5)) -eq 0 ]; then
            echo -e "${BLUE}⏱️  Waiting... ($elapsed/$timeout seconds)${NC}"
        fi
    done
    
    # Timeout reached
    echo -e "${YELLOW}⚠️  $name timed out after $timeout seconds${NC}"
    return 1
}

# Wait for both to complete with timeout
TIMEOUT=20
echo -e "${BLUE}⏳ Waiting for test to complete (max ${TIMEOUT}s)...${NC}"

# Monitor both processes
check_timeout $ALICE_PID "Alice" $TIMEOUT &
ALICE_MONITOR=$!

check_timeout $BOB_PID "Bob" $TIMEOUT &
BOB_MONITOR=$!

# Wait for monitors to complete
wait $ALICE_MONITOR
ALICE_TIMEOUT=$?

wait $BOB_MONITOR
BOB_TIMEOUT=$?

# Get actual exit codes if processes finished
ALICE_EXIT=0
BOB_EXIT=0

if [ $ALICE_TIMEOUT -eq 0 ]; then
    wait $ALICE_PID 2>/dev/null
    ALICE_EXIT=$?
else
    echo -e "${YELLOW}Terminating Alice...${NC}"
    kill -TERM $ALICE_PID 2>/dev/null
    ALICE_EXIT=124  # Timeout exit code
fi

if [ $BOB_TIMEOUT -eq 0 ]; then
    wait $BOB_PID 2>/dev/null
    BOB_EXIT=$?
else
    echo -e "${YELLOW}Terminating Bob...${NC}"
    kill -TERM $BOB_PID 2>/dev/null
    BOB_EXIT=124  # Timeout exit code
fi

echo ""
echo "======================================"

# Check results
if [ $ALICE_EXIT -eq 0 ] && [ $BOB_EXIT -eq 0 ]; then
    echo -e "${GREEN}✅ Test completed successfully!${NC}"
    
    if [ $RECORD_MODE -eq 1 ]; then
        echo ""
        echo "📁 Audio files saved to: output/"
        ls -la output/*.raw 2>/dev/null || echo "   (No audio files found - audio exchange may not be implemented yet)"
    fi
else
    echo -e "${RED}❌ Test failed or timed out${NC}"
    [ $ALICE_EXIT -ne 0 ] && echo "   Alice exit code: $ALICE_EXIT"
    [ $BOB_EXIT -ne 0 ] && echo "   Bob exit code: $BOB_EXIT"
    
    echo ""
    echo -e "${YELLOW}📋 Logs saved to:${NC}"
    echo "   - $ALICE_LOG"
    echo "   - $BOB_LOG"
    echo ""
    echo "Check logs for details. Common issues:"
    echo "  • API differences - session-core-v2 has different API"
    echo "  • Missing features - Audio channels may not be directly exposed"
    echo "  • Connection issues - Ensure dialog-core and media-core are working"
    
    exit 1
fi

# Show log location even on success if debug mode
if [ $DEBUG_MODE -eq 1 ]; then
    echo ""
    echo -e "${YELLOW}📋 Debug logs saved to:${NC}"
    echo "   - $ALICE_LOG"
    echo "   - $BOB_LOG"
fi