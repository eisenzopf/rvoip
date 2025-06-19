#!/bin/bash

# SIPp Integration Test Script for RVOIP Client Core
# This script demonstrates a full SIP call lifecycle with audio exchange

set -e  # Exit on error

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
SIP_PORT=5060
MEDIA_PORT=20000
SIPP_PORT=5061
SIPP_MEDIA_PORT=30000
NUM_CALLS=5
CALL_RATE=1
CALL_DURATION=10

# Paths
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$SCRIPT_DIR/../../../.."
PCAP_DIR="$SCRIPT_DIR/pcap"

echo -e "${BLUE}🚀 RVOIP Client Core - SIPp Integration Test${NC}"
echo "================================================"

# Function to cleanup on exit
cleanup() {
    echo -e "\n${YELLOW}🧹 Cleaning up...${NC}"
    # Kill the server if it's running
    if [ ! -z "$SERVER_PID" ]; then
        kill $SERVER_PID 2>/dev/null || true
    fi
    echo -e "${GREEN}✅ Cleanup complete${NC}"
}

# Set trap to cleanup on exit
trap cleanup EXIT

# Check dependencies
echo -e "\n${BLUE}📋 Checking dependencies...${NC}"

# Check if SIPp is installed
if ! command -v sipp &> /dev/null; then
    echo -e "${RED}❌ SIPp is not installed${NC}"
    echo "   Please install SIPp:"
    echo "   - macOS: brew install sipp"
    echo "   - Ubuntu: sudo apt-get install sip-tester"
    echo "   - From source: https://github.com/SIPp/sipp"
    exit 1
fi

echo -e "${GREEN}✅ SIPp found:${NC} $(sipp -v 2>&1 | grep -m1 'SIPp' || echo 'version unknown')"

# Check if cargo is installed
if ! command -v cargo &> /dev/null; then
    echo -e "${RED}❌ Cargo is not installed${NC}"
    echo "   Please install Rust: https://rustup.rs/"
    exit 1
fi

echo -e "${GREEN}✅ Cargo found${NC}"

# Download PCAP files if they don't exist
echo -e "\n${BLUE}📦 Checking for audio PCAP files...${NC}"
mkdir -p "$PCAP_DIR"

if [ ! -f "$PCAP_DIR/g711a.pcap" ]; then
    echo "   Downloading G.711 A-law PCAP..."
    if command -v wget &> /dev/null; then
        wget -q -O "$PCAP_DIR/g711a.pcap" https://raw.githubusercontent.com/SIPp/sipp/master/pcap/g711a.pcap
    elif command -v curl &> /dev/null; then
        curl -s -o "$PCAP_DIR/g711a.pcap" https://raw.githubusercontent.com/SIPp/sipp/master/pcap/g711a.pcap
    else
        echo -e "${RED}   ❌ Neither wget nor curl found${NC}"
        exit 1
    fi
    echo -e "${GREEN}   ✅ Downloaded g711a.pcap${NC}"
fi

if [ ! -f "$PCAP_DIR/g711u.pcap" ]; then
    echo "   Downloading G.711 μ-law PCAP..."
    if command -v wget &> /dev/null; then
        wget -q -O "$PCAP_DIR/g711u.pcap" https://raw.githubusercontent.com/SIPp/sipp/master/pcap/g711u.pcap
    elif command -v curl &> /dev/null; then
        curl -s -o "$PCAP_DIR/g711u.pcap" https://raw.githubusercontent.com/SIPp/sipp/master/pcap/g711u.pcap
    else
        echo -e "${RED}   ❌ Neither wget nor curl found${NC}"
        exit 1
    fi
    echo -e "${GREEN}   ✅ Downloaded g711u.pcap${NC}"
fi

echo -e "${GREEN}✅ Audio files ready${NC}"

# Build the example
echo -e "\n${BLUE}🔨 Building RVOIP test server...${NC}"
cd "$PROJECT_ROOT"
cargo build --release --example sipp_integration_sip_test_server
echo -e "${GREEN}✅ Build successful${NC}"

# Kill any process using the SIP port
echo -e "\n${BLUE}🧹 Checking for processes using port $SIP_PORT...${NC}"
if lsof -ti :$SIP_PORT >/dev/null 2>&1; then
    echo "   Found process using port $SIP_PORT, killing it..."
    lsof -ti :$SIP_PORT | xargs kill -9 2>/dev/null || true
    sleep 1
    echo -e "${GREEN}   ✅ Port cleared${NC}"
else
    echo -e "${GREEN}   ✅ Port is free${NC}"
fi

# Start the server using cargo run
echo -e "\n${BLUE}▶️  Starting RVOIP SIP server...${NC}"
echo "   SIP Port: $SIP_PORT"
echo "   Media Port: $MEDIA_PORT"
echo "   Auto-answer: enabled"

# Start server with cargo run and capture output
cd "$PROJECT_ROOT"
SERVER_READY=false
cargo run --release --example sipp_integration_sip_test_server -- \
    $SIP_PORT \
    $MEDIA_PORT \
    auto > server.log 2>&1 &
SERVER_PID=$!

# Give server a moment to start binding
sleep 1

# Wait for server to start
echo -n "   Waiting for server to start"
for i in {1..10}; do
    # Check if our server process is running and has bound to the port
    if lsof -i :$SIP_PORT 2>/dev/null | grep -q "sipp_inte\|UDP"; then
        echo -e "\n${GREEN}✅ Server is ready${NC}"
        SERVER_READY=true
        break
    fi
    echo -n "."
    sleep 1
done

if [ "$SERVER_READY" != "true" ]; then
    echo -e "\n${RED}❌ Server failed to start${NC}"
    echo "Checking if port is in use:"
    lsof -i :$SIP_PORT || echo "Port $SIP_PORT not in use"
    echo "Server log:"
    tail -20 "$PROJECT_ROOT/server.log"
    exit 1
fi

# Run test based on argument
TEST_TYPE="${1:-media}"

if [ "$TEST_TYPE" == "simple" ]; then
    # Run simple test without media
    echo -e "\n${BLUE}📞 Running simple SIPp test (no media)...${NC}"
    echo "   Target: 127.0.0.1:$SIP_PORT"
    echo "   Calls: 1"
    
    cd "$SCRIPT_DIR"
    sipp -sf simple_uac.xml \
         -s test \
         127.0.0.1:$SIP_PORT \
         -l 1 \
         -m 1 \
         -trace_msg \
         -message_file sipp_messages.log \
         -trace_screen \
         -screen_file sipp_screen.log \
         -trace_err \
         -error_file sipp_errors.log
         
else
    # Run full test with media
    echo -e "\n${BLUE}📞 Running SIPp test scenario with media...${NC}"
    echo "   Target: 127.0.0.1:$SIP_PORT"
    echo "   Calls: $NUM_CALLS"
    echo "   Rate: $CALL_RATE call/s"
    echo "   Duration: $CALL_DURATION seconds per call"
    
    cd "$SCRIPT_DIR"
    sipp -sf uac_with_media.xml \
         -s service \
         127.0.0.1:$SIP_PORT \
         -l $NUM_CALLS \
         -r $CALL_RATE \
         -m $NUM_CALLS \
         -d $((CALL_DURATION * 1000)) \
         -mi 127.0.0.1 \
         -mp $SIPP_MEDIA_PORT \
         -rtp_echo \
         -trace_msg \
         -message_file sipp_messages.log \
         -trace_screen \
         -screen_file sipp_screen.log \
         -trace_err \
         -error_file sipp_errors.log
fi

# Check results
echo -e "\n${BLUE}📊 Test Results:${NC}"
if [ -f sipp_screen.log ]; then
    echo "SIPp Screen Output:"
    echo "==================="
    tail -20 sipp_screen.log
fi

if [ -f sipp_errors.log ]; then
    echo -e "\n${YELLOW}⚠️  SIPp Errors:${NC}"
    cat sipp_errors.log
fi

# Show server log
echo -e "\n${BLUE}📋 Server Log (last 30 lines):${NC}"
echo "================================"
tail -30 "$PROJECT_ROOT/server.log"

echo -e "\n${GREEN}✅ Test completed!${NC}"
echo "Log files available:"
echo "  - Server log: $PROJECT_ROOT/server.log"
echo "  - SIPp messages: $SCRIPT_DIR/sipp_messages.log"
echo "  - SIPp screen: $SCRIPT_DIR/sipp_screen.log"
echo "  - SIPp statistics: $SCRIPT_DIR/sipp_stats.csv" 