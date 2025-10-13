#!/bin/bash

# Blind Transfer Test Runner for session-core-v3
# This script orchestrates running three peers to demonstrate callback-based blind transfer:
# - Alice (Peer1) calls Bob (Peer2) 
# - Bob initiates transfer to Charlie (Peer3)
# - Alice handles REFER via events and creates new session to Charlie
# - Alice ends up talking to Charlie

echo "🔄 Session-Core-V3 Event-Driven Blind Transfer Test"
echo "=============================================="
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
            export RUST_LOG=rvoip_session_core_v3=info,rvoip_dialog_core=info,rvoip_media_core=info
            DEBUG_MODE=1
            echo -e "${YELLOW}🔍 Debug logging enabled${NC}"
            ;;
        --trace)
            export RUST_LOG=rvoip_session_core_v3=trace,rvoip_dialog_core=trace,rvoip_media_core=trace
            DEBUG_MODE=1
            echo -e "${YELLOW}🔍 Trace logging enabled${NC}"
            ;;
        --help)
            echo "Usage: $0 [--debug|--trace|--help]"
            echo ""
            echo "Options:"
            echo "  --debug    Enable debug logging"
            echo "  --trace    Enable trace logging (very verbose)"
            echo "  --help     Show this help message"
            echo ""
            echo "This test demonstrates the new callback-based transfer approach:"
            echo "1. Alice registers a REFER callback"
            echo "2. Alice calls Bob"
            echo "3. Bob initiates transfer (simulated)"
            echo "4. Alice's callback handles the transfer manually"
            echo "5. Alice creates new session to Charlie"
            exit 0
            ;;
    esac
done

# Function to cleanup background processes
cleanup() {
    echo -e "\n${YELLOW}🧹 Cleaning up background processes...${NC}"
    if [[ -n $CHARLIE_PID ]]; then
        kill $CHARLIE_PID 2>/dev/null
        echo -e "${BLUE}[CHARLIE]${NC} Process terminated"
    fi
    if [[ -n $BOB_PID ]]; then
        kill $BOB_PID 2>/dev/null
        echo -e "${GREEN}[BOB]${NC} Process terminated"
    fi
    if [[ -n $ALICE_PID ]]; then
        kill $ALICE_PID 2>/dev/null
        echo -e "${RED}[ALICE]${NC} Process terminated"
    fi
    
    # Wait a moment for processes to clean up
    sleep 1
    
    echo -e "${GREEN}✅ Cleanup complete${NC}"
}

# Set up signal handlers
trap cleanup EXIT
trap cleanup INT
trap cleanup TERM

# Check if we're in the right directory
if [[ ! -f "../../Cargo.toml" ]]; then
    echo -e "${RED}❌ Error: Please run this script from the examples/blind_transfer directory${NC}"
    echo "   cd crates/session-core-v3/examples/blind_transfer"
    echo "   ./run_blind_transfer.sh"
    exit 1
fi

# Clean up any existing output
echo -e "${BLUE}🧹 Cleaning up previous run...${NC}"
rm -rf output/
mkdir -p logs/

# Generate timestamp for log files
TIMESTAMP=$(date +"%Y%m%d_%H%M%S")

echo -e "${BLUE}📋 Test Overview:${NC}"
echo "  • Alice (port 5060) - Caller with event-driven REFER handling"
echo "  • Bob (port 5061) - Transferor" 
echo "  • Charlie (port 5062) - Transfer target"
echo ""
echo -e "${YELLOW}🔄 New Event-Driven Approach:${NC}"
echo "  • Alice uses simple next_event() loop"
echo "  • When REFER received, Alice handles it directly"
echo "  • Clean event-driven transfer logic"
echo "  • Main functions under 50 lines each"
echo ""

# Start Charlie (Transfer Target) first
echo -e "${BLUE}🚀 Starting Charlie (Transfer Target)...${NC}"
if [[ $DEBUG_MODE -eq 1 ]]; then
    cargo run --example blind_transfer_peer3_target > logs/charlie_${TIMESTAMP}.log 2>&1 &
else
    cargo run --example blind_transfer_peer3_target > logs/charlie_${TIMESTAMP}.log 2>&1 &
fi
CHARLIE_PID=$!
echo -e "${BLUE}[CHARLIE]${NC} Started with PID $CHARLIE_PID (port 5062)"

# Give Charlie time to start
sleep 2

# Start Bob (Transferor)
echo -e "${GREEN}🚀 Starting Bob (Transferor)...${NC}"
if [[ $DEBUG_MODE -eq 1 ]]; then
    cargo run --example blind_transfer_peer2_transferor > logs/bob_${TIMESTAMP}.log 2>&1 &
else
    cargo run --example blind_transfer_peer2_transferor > logs/bob_${TIMESTAMP}.log 2>&1 &
fi
BOB_PID=$!
echo -e "${GREEN}[BOB]${NC} Started with PID $BOB_PID (port 5061)"

# Give Bob time to start
sleep 2

# Start Alice (Event-driven caller)
echo -e "${RED}🚀 Starting Alice (Event-driven caller)...${NC}"
echo -e "${YELLOW}📞 Alice will:${NC}"
echo "  1. Call Bob using simple API"
echo "  2. Handle events in next_event() loop"
echo "  3. Process REFER event and create new session to Charlie"
echo "  4. Demonstrate clean event-driven transfer"
echo ""

if [[ $DEBUG_MODE -eq 1 ]]; then
    cargo run --example blind_transfer_peer1_caller 2>&1 | tee logs/alice_${TIMESTAMP}.log
else
    cargo run --example blind_transfer_peer1_caller > logs/alice_${TIMESTAMP}.log 2>&1
fi
ALICE_EXIT_CODE=$?

# Alice has finished, wait a moment for other processes to complete
echo -e "\n${YELLOW}⏳ Waiting for other processes to complete...${NC}"
sleep 3

# Check results
echo -e "\n${BLUE}📊 Test Results:${NC}"
echo "=================="

if [[ $ALICE_EXIT_CODE -eq 0 ]]; then
    echo -e "${GREEN}✅ Alice completed successfully${NC}"
else
    echo -e "${RED}❌ Alice failed with exit code $ALICE_EXIT_CODE${NC}"
fi

# Check if audio files were created
if [[ -f "output/alice_sent.wav" ]]; then
    echo -e "${GREEN}✅ Alice sent audio saved${NC}"
else
    echo -e "${RED}❌ Alice sent audio missing${NC}"
fi

if [[ -f "output/alice_received.wav" ]]; then
    echo -e "${GREEN}✅ Alice received audio saved${NC}"
else
    echo -e "${YELLOW}⚠️  Alice received audio missing (may be normal)${NC}"
fi

if [[ -f "output/bob_sent.wav" ]]; then
    echo -e "${GREEN}✅ Bob sent audio saved${NC}"
else
    echo -e "${RED}❌ Bob sent audio missing${NC}"
fi

if [[ -f "output/charlie_sent.wav" ]]; then
    echo -e "${GREEN}✅ Charlie sent audio saved${NC}"
else
    echo -e "${YELLOW}⚠️  Charlie sent audio missing (transfer may not have completed)${NC}"
fi

echo ""
echo -e "${BLUE}📁 Generated Files:${NC}"
echo "  • Logs: logs/*_${TIMESTAMP}.log"
if [[ -d "output" ]]; then
    echo "  • Audio: output/*.wav"
    ls -la output/ 2>/dev/null | grep -E "\\.wav$" | sed 's/^/    /'
fi

echo ""
echo -e "${BLUE}🔍 Log Analysis:${NC}"
if grep -q "Simple event-driven" logs/alice_${TIMESTAMP}.log 2>/dev/null; then
    echo -e "${GREEN}✅ Event-driven SimplePeer started${NC}"
else
    echo -e "${RED}❌ Event-driven SimplePeer not detected${NC}"
fi

if grep -q "Received REFER" logs/alice_${TIMESTAMP}.log 2>/dev/null; then
    echo -e "${GREEN}✅ REFER request was received and handled${NC}"
else
    echo -e "${YELLOW}⚠️  REFER request handling not detected${NC}"
fi

if grep -q "Transfer complete" logs/alice_${TIMESTAMP}.log 2>/dev/null; then
    echo -e "${GREEN}✅ Transfer was completed successfully${NC}"
else
    echo -e "${YELLOW}⚠️  Transfer completion not detected${NC}"
fi

echo ""
echo -e "${BLUE}💡 Key Features of Event-Driven API:${NC}"
echo "  • Simple next_event() loop for all events"
echo "  • Direct method calls (alice.call(), alice.hangup())"
echo "  • CallHandle for clean audio operations"
echo "  • Main functions under 50 lines"
echo "  • No complex state management in user code"

if [[ $DEBUG_MODE -eq 1 ]]; then
    echo ""
    echo -e "${YELLOW}🔍 Debug logs available in logs/ directory${NC}"
    echo "  View with: tail -f logs/alice_${TIMESTAMP}.log"
fi

echo ""
echo -e "${GREEN}🎉 Event-driven blind transfer test completed!${NC}"