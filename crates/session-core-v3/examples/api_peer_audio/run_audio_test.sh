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

# Set recording as default
export RECORD_AUDIO=1
RECORD_MODE=1

# Parse arguments
DEBUG_MODE=0
for arg in "$@"; do
    case $arg in
        --no-record)
            unset RECORD_AUDIO
            RECORD_MODE=0
            echo -e "${YELLOW}📼 Recording disabled${NC}"
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
            echo "Usage: $0 [--no-record] [--debug|--trace]"
            echo "  --no-record  Disable audio recording (enabled by default)"
            echo "  --debug      Enable info-level logging"
            echo "  --trace      Enable debug-level logging"
            exit 0
            ;;
    esac
done

# Show recording status
if [ $RECORD_MODE -eq 1 ]; then
    echo -e "${BLUE}📼 Recording enabled - Audio files will be saved to output/${NC}"
fi

# Create log directory (in the examples/api_peer_audio directory)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOG_DIR="$SCRIPT_DIR/logs"
mkdir -p $LOG_DIR
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
ALICE_LOG="$LOG_DIR/alice_${TIMESTAMP}.log"
BOB_LOG="$LOG_DIR/bob_${TIMESTAMP}.log"

# Clean up any previous output
OUTPUT_DIR="$SCRIPT_DIR/output"
if [ -d "$OUTPUT_DIR" ]; then
    rm -rf "$OUTPUT_DIR"
fi
mkdir -p "$OUTPUT_DIR"

# Navigate to the session-core-v2 directory to run cargo commands
cd "$(dirname "$0")/../.."

# Start Bob (peer2) in the background
echo -e "${GREEN}▶️  Starting Bob (peer2) on port 5061...${NC}"
RECORD_AUDIO=$RECORD_AUDIO RVOIP_STATE_TABLE="$SCRIPT_DIR/peer_audio_states.yaml" cargo run --example api_peer_audio_peer2 -p rvoip-session-core-v2 > >(tee "$BOB_LOG" | sed 's/^/[BOB] /') 2>&1 &
BOB_PID=$!

# Give Bob time to start listening
sleep 3

# Start Alice (peer1)
echo -e "${GREEN}▶️  Starting Alice (peer1) on port 5060...${NC}"
RECORD_AUDIO=$RECORD_AUDIO RVOIP_STATE_TABLE="$SCRIPT_DIR/peer_audio_states.yaml" cargo run --example api_peer_audio_peer1 -p rvoip-session-core-v2 > >(tee "$ALICE_LOG" | sed 's/^/[ALICE] /') 2>&1 &
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
TIMEOUT=30
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
        echo "📁 Audio files saved to: $OUTPUT_DIR/"
        ls -la "$OUTPUT_DIR"/*.wav 2>/dev/null || echo "   (No audio files found - audio exchange may not be implemented yet)"
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