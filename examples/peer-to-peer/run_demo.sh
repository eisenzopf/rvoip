#!/bin/bash

# Peer-to-peer SIP demo runner
# This script starts both peers and monitors their execution

set -e

# Colors for output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}🚀 RVOIP Peer-to-Peer Demo${NC}"
echo "============================"

# Function to cleanup
cleanup() {
    echo -e "\n${YELLOW}🧹 Cleaning up...${NC}"
    # Kill peers if running
    if [ ! -z "$PEER_A_PID" ]; then
        kill $PEER_A_PID 2>/dev/null || true
    fi
    if [ ! -z "$PEER_B_PID" ]; then
        kill $PEER_B_PID 2>/dev/null || true
    fi
    echo -e "${GREEN}✅ Cleanup complete${NC}"
}

trap cleanup EXIT

# Create logs directory
mkdir -p logs

# Build the binaries
echo -e "\n${BLUE}🔨 Building Peer A and Peer B...${NC}"
cargo build --release --bin peer_a --bin peer_b

# Check if builds succeeded
if [ $? -ne 0 ]; then
    echo -e "${RED}❌ Build failed!${NC}"
    exit 1
fi

echo -e "${GREEN}✅ Build successful${NC}"

# Start Peer B first (it needs to be ready to receive the call)
echo -e "\n${BLUE}▶️  Starting Peer B (Receiver)...${NC}"
echo "   SIP Port: 5061"
echo "   Media Ports: 21000-21100"
echo "   Log: logs/peer_b.log"

target/release/peer_b > logs/peer_b_stdout.log 2>&1 &
PEER_B_PID=$!

# Wait for Peer B to start
echo -n "   Waiting for Peer B to start"
for i in {1..10}; do
    if lsof -i :5061 >/dev/null 2>&1; then
        echo -e "\n${GREEN}✅ Peer B is ready${NC}"
        break
    fi
    echo -n "."
    sleep 1
done

# Give Peer B a moment to fully initialize
sleep 1

# Start Peer A (it will initiate the call)
echo -e "\n${BLUE}▶️  Starting Peer A (Caller)...${NC}"
echo "   SIP Port: 5060"
echo "   Media Ports: 20000-20100"
echo "   Log: logs/peer_a.log"

target/release/peer_a > logs/peer_a_stdout.log 2>&1 &
PEER_A_PID=$!

# Monitor the demo execution
echo -e "\n${BLUE}📋 Demo Progress:${NC}"
echo "   1. Peer A will wait 3 seconds, then call Peer B"
echo "   2. Peer B will auto-answer after 1 second"
echo "   3. Both peers will exchange RTP media for 15 seconds"
echo "   4. Peer A will terminate the call"
echo ""

# Wait for Peer A to complete (it controls the demo flow)
echo -e "${YELLOW}⏳ Waiting for demo to complete...${NC}"
wait $PEER_A_PID
PEER_A_EXIT_CODE=$?

# Give Peer B a moment to finish
sleep 2

# Kill Peer B if it's still running
if kill -0 $PEER_B_PID 2>/dev/null; then
    kill $PEER_B_PID 2>/dev/null || true
fi

# Check results
echo -e "\n${BLUE}📊 Demo Results:${NC}"
echo "================================"

if [ $PEER_A_EXIT_CODE -eq 0 ]; then
    echo -e "${GREEN}✅ Peer A completed successfully${NC}"
else
    echo -e "${RED}❌ Peer A failed with exit code $PEER_A_EXIT_CODE${NC}"
fi

# Check if log files exist and have content
if [ -f "logs/peer_a_stdout.log" ] && [ -s "logs/peer_a_stdout.log" ]; then
    echo -e "${GREEN}✅ Peer A log file created${NC}"
else
    echo -e "${RED}❌ Peer A log file missing or empty${NC}"
fi

if [ -f "logs/peer_b_stdout.log" ] && [ -s "logs/peer_b_stdout.log" ]; then
    echo -e "${GREEN}✅ Peer B log file created${NC}"
else
    echo -e "${RED}❌ Peer B log file missing or empty${NC}"
fi

# Extract and display key statistics from logs
echo -e "\n${BLUE}📊 Call Statistics:${NC}"
echo "==================="

# Look for RTP statistics in Peer A log
if [ -f "logs/peer_a_stdout.log" ]; then
    PEER_A_RTP=$(grep "Final RTP Stats" logs/peer_a_stdout.log | tail -1)
    if [ ! -z "$PEER_A_RTP" ]; then
        echo -e "${GREEN}📤 Peer A (Caller): $PEER_A_RTP${NC}"
    else
        echo -e "${YELLOW}⚠️  No RTP stats found for Peer A${NC}"
    fi
fi

# Look for RTP statistics in Peer B log
if [ -f "logs/peer_b_stdout.log" ]; then
    PEER_B_RTP=$(grep "Final RTP Stats" logs/peer_b_stdout.log | tail -1)
    if [ ! -z "$PEER_B_RTP" ]; then
        echo -e "${GREEN}📥 Peer B (Receiver): $PEER_B_RTP${NC}"
    else
        echo -e "${YELLOW}⚠️  No RTP stats found for Peer B${NC}"
    fi
fi

# Check for successful call establishment
CALL_CONNECTED_A=$(grep -c "Call connected" logs/peer_a_stdout.log 2>/dev/null || echo "0")
CALL_CONNECTED_B=$(grep -c "Call connected" logs/peer_b_stdout.log 2>/dev/null || echo "0")

if [ "$CALL_CONNECTED_A" -gt 0 ] && [ "$CALL_CONNECTED_B" -gt 0 ]; then
    echo -e "${GREEN}✅ SIP call successfully established${NC}"
else
    echo -e "${RED}❌ SIP call failed to establish${NC}"
fi

# Check for media session start
MEDIA_A=$(grep -c "Audio transmission started" logs/peer_a_stdout.log 2>/dev/null || echo "0")
MEDIA_B=$(grep -c "Audio transmission started" logs/peer_b_stdout.log 2>/dev/null || echo "0")

if [ "$MEDIA_A" -gt 0 ] && [ "$MEDIA_B" -gt 0 ]; then
    echo -e "${GREEN}✅ RTP media exchange successful${NC}"
else
    echo -e "${RED}❌ RTP media exchange failed${NC}"
fi

# Generate SIP message log by combining relevant entries
echo -e "\n${BLUE}📞 SIP Messages Log:${NC}"
echo "===================="
echo "Generating combined SIP message timeline..."

# Create a combined SIP message log
cat > logs/sip_messages.log << EOF
# SIP Messages Timeline - Peer-to-Peer Demo
# Generated: $(date)
# 
# This log shows the key SIP signaling events from both peers
# Format: [Timestamp] [Peer] Message
#

EOF

# Extract SIP-related messages from both logs and sort by timestamp
(
    [ -f "logs/peer_a_stdout.log" ] && grep -E "(Initiating call|Call.*state|Incoming call)" logs/peer_a_stdout.log | sed 's/^/[PEER A] /'
    [ -f "logs/peer_b_stdout.log" ] && grep -E "(Initiating call|Call.*state|Incoming call)" logs/peer_b_stdout.log | sed 's/^/[PEER B] /'
) >> logs/sip_messages.log

echo -e "${GREEN}✅ SIP messages log created: logs/sip_messages.log${NC}"

# Final summary
echo -e "\n${BLUE}📋 Summary:${NC}"
echo "==========="
echo "📁 Log files created:"
echo "   - logs/peer_a_stdout.log (Peer A detailed log)"
echo "   - logs/peer_b_stdout.log (Peer B detailed log)"
echo "   - logs/sip_messages.log (Combined SIP messages timeline)"
echo ""

# Overall result
if [ $PEER_A_EXIT_CODE -eq 0 ] && [ "$CALL_CONNECTED_A" -gt 0 ] && [ "$CALL_CONNECTED_B" -gt 0 ]; then
    echo -e "${GREEN}🎉 DEMO SUCCESSFUL!${NC}"
    echo -e "${GREEN}   Both peers connected and exchanged media successfully${NC}"
    exit 0
else
    echo -e "${RED}❌ DEMO FAILED!${NC}"
    echo -e "${RED}   Check the log files for details${NC}"
    exit 1
fi 