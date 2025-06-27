#!/bin/bash

# SIP REGISTER Demo Script
# This script demonstrates the REGISTER flow between a client and the CallCenterEngine server

set -e

echo "🚀 SIP REGISTER Demo"
echo "===================="
echo ""

# Colors for output
GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
NC='\033[0m' # No Color

# Check if we're in the right directory
if [ ! -f "Cargo.toml" ]; then
    echo -e "${RED}Error: Please run this script from the rvoip/crates/call-engine directory${NC}"
    exit 1
fi

# Build the examples
echo -e "${BLUE}📦 Building examples...${NC}"
cargo build --examples --quiet

# Start the server in the background
echo -e "${GREEN}🔧 Starting CallCenterEngine server...${NC}"
cargo run --example register_demo_server --quiet 2>&1 | tee server.log &
SERVER_PID=$!

# Give the server time to start
echo -e "${BLUE}⏳ Waiting for server to start...${NC}"
sleep 3

# Check if server is running
if ! ps -p $SERVER_PID > /dev/null; then
    echo -e "${RED}❌ Server failed to start. Check server.log for details${NC}"
    cat server.log
    exit 1
fi

echo -e "${GREEN}✅ Server running on PID $SERVER_PID${NC}"
echo ""

# Run the client
echo -e "${BLUE}📱 Starting SIP client (using session-core API)...${NC}"
echo ""

cargo run --example register_demo_client --quiet

# Cleanup
echo ""
echo -e "${BLUE}🧹 Cleaning up...${NC}"
kill $SERVER_PID 2>/dev/null || true
wait $SERVER_PID 2>/dev/null || true

echo -e "${GREEN}✅ Demo completed!${NC}"
echo ""
echo "Server log saved to: server.log"
echo ""
echo -e "${BLUE}📋 Implementation Status:${NC}"
echo "  ✅ SipClient trait defined and exported from session-core"
echo "  ✅ Client uses session-core API exclusively"
echo "  ⚠️  Full implementation pending dialog-core support for non-dialog requests"
echo ""
echo -e "${BLUE}Next steps:${NC}"
echo "  1. Add send_non_dialog_request() to dialog-core's UnifiedDialogApi"
echo "  2. Complete the register() implementation to send real SIP messages"
echo "  3. Test against real SIP servers" 