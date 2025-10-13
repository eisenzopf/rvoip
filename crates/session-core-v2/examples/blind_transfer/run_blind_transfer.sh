#!/bin/bash

# Blind Transfer Test Runner for session-core-v2
# This script orchestrates running three peers to demonstrate blind transfer:
# - Alice (Peer1) calls Bob (Peer2)
# - Bob transfers Alice to Charlie (Peer3)
# - Alice ends up talking to Charlie

echo "🔄 Session-Core-V2 Blind Transfer Test"
echo "======================================"
echo ""

# Force use of default state table (unset any custom table)
unset RVOIP_STATE_TABLE

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[0;33m'
NC='\033[0m' # No Color

# Parse arguments
DEBUG_MODE=0
for arg in "$@"; do
    case $arg in
        --debug)
            export RUST_LOG=rvoip_session_core_v2=info,rvoip_dialog_core=info,rvoip_media_core=info
            DEBUG_MODE=1
            echo -e "${YELLOW}🔍 Debug logging enabled${NC}"
            ;;
        --trace)
            export RUST_LOG=rvoip_session_core_v2=debug,rvoip_dialog_core=debug,rvoip_media_core=debug
            DEBUG_MODE=1
            echo -e "${YELLOW}🔍 Trace logging enabled${NC}"
            ;;
        --help)
            echo "Usage: $0 [--debug|--trace]"
            echo "  --debug      Enable info-level logging"
            echo "  --trace      Enable debug-level logging"
            exit 0
            ;;
    esac
done

# Create log directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOG_DIR="$SCRIPT_DIR/logs"
mkdir -p $LOG_DIR
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
CHARLIE_LOG="$LOG_DIR/charlie_${TIMESTAMP}.log"
BOB_LOG="$LOG_DIR/bob_${TIMESTAMP}.log"
ALICE_LOG="$LOG_DIR/alice_${TIMESTAMP}.log"

# Navigate to the session-core-v2 directory to run cargo commands
cd "$(dirname "$0")/../.."

echo "Building examples..."
cargo build --example blind_transfer_peer1_caller -p rvoip-session-core-v2 --quiet
cargo build --example blind_transfer_peer2_transferor -p rvoip-session-core-v2 --quiet
cargo build --example blind_transfer_peer3_target -p rvoip-session-core-v2 --quiet

echo ""

# Start Charlie (Peer3 - transfer target) first
echo -e "${GREEN}▶️  Starting Charlie (peer3 - transfer target) on port 5062...${NC}"
cargo run --example blind_transfer_peer3_target -p rvoip-session-core-v2 --quiet > >(tee "$CHARLIE_LOG" | sed 's/^/[CHARLIE] /') 2>&1 &
CHARLIE_PID=$!

# Give Charlie time to start listening
sleep 2

# Start Bob (Peer2 - transferor)
echo -e "${GREEN}▶️  Starting Bob (peer2 - transferor) on port 5061...${NC}"
cargo run --example blind_transfer_peer2_transferor -p rvoip-session-core-v2 --quiet > >(tee "$BOB_LOG" | sed 's/^/[BOB] /') 2>&1 &
BOB_PID=$!

# Give Bob time to start listening
sleep 2

# Start Alice (Peer1 - caller)
echo -e "${GREEN}▶️  Starting Alice (peer1 - caller) on port 5060...${NC}"
cargo run --example blind_transfer_peer1_caller -p rvoip-session-core-v2 --quiet > >(tee "$ALICE_LOG" | sed 's/^/[ALICE] /') 2>&1 &
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

# Wait for all to complete with timeout
TIMEOUT=45  # Increased for blind transfer timing
echo -e "${BLUE}⏳ Waiting for test to complete (max ${TIMEOUT}s)...${NC}"
echo ""

# Monitor all processes
check_timeout $ALICE_PID "Alice" $TIMEOUT &
ALICE_MONITOR=$!

check_timeout $BOB_PID "Bob" $TIMEOUT &
BOB_MONITOR=$!

check_timeout $CHARLIE_PID "Charlie" $TIMEOUT &
CHARLIE_MONITOR=$!

# Wait for monitors to complete
wait $ALICE_MONITOR
ALICE_TIMEOUT=$?

wait $BOB_MONITOR
BOB_TIMEOUT=$?

wait $CHARLIE_MONITOR
CHARLIE_TIMEOUT=$?

# Get actual exit codes if processes finished
ALICE_EXIT=0
BOB_EXIT=0
CHARLIE_EXIT=0

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

if [ $CHARLIE_TIMEOUT -eq 0 ]; then
    wait $CHARLIE_PID 2>/dev/null
    CHARLIE_EXIT=$?
else
    echo -e "${YELLOW}Terminating Charlie...${NC}"
    kill -TERM $CHARLIE_PID 2>/dev/null
    CHARLIE_EXIT=124  # Timeout exit code
fi

echo ""
echo "======================================"

# Check results
if [ $ALICE_EXIT -eq 0 ] && [ $BOB_EXIT -eq 0 ] && [ $CHARLIE_EXIT -eq 0 ]; then
    echo -e "${GREEN}✅ Blind transfer test completed successfully!${NC}"
    echo ""
    echo "Test flow:"
    echo "  1. Alice called Bob ✓"
    echo "  2. Bob received call from Alice ✓"
    echo "  3. Bob initiated blind transfer to Charlie ✓"
    echo "  4. Charlie received transferred call ✓"
    echo "  5. Alice and Charlie connected ✓"
else
    echo -e "${RED}❌ Test failed or timed out${NC}"
    [ $ALICE_EXIT -ne 0 ] && echo "   Alice exit code: $ALICE_EXIT"
    [ $BOB_EXIT -ne 0 ] && echo "   Bob exit code: $BOB_EXIT"
    [ $CHARLIE_EXIT -ne 0 ] && echo "   Charlie exit code: $CHARLIE_EXIT"

    echo ""
    echo -e "${YELLOW}📋 Logs saved to:${NC}"
    echo "   - $ALICE_LOG"
    echo "   - $BOB_LOG"
    echo "   - $CHARLIE_LOG"
    echo ""
    echo "Check logs for details. Common issues:"
    echo "  • Blind transfer may not be fully implemented"
    echo "  • REFER handling might need work"
    echo "  • Connection issues between peers"

    exit 1
fi

# Show log location even on success if debug mode
if [ $DEBUG_MODE -eq 1 ]; then
    echo ""
    echo -e "${YELLOW}📋 Debug logs saved to:${NC}"
    echo "   - $ALICE_LOG"
    echo "   - $BOB_LOG"
    echo "   - $CHARLIE_LOG"
fi
