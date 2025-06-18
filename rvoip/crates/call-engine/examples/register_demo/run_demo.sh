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
cargo run --example register_demo_server --quiet > server.log 2>&1 &
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
echo -e "${BLUE}📱 Starting SIP client...${NC}"
echo ""

# For now, since we need to implement the SipClient trait first,
# we'll use the existing client that uses sip-transport directly
cargo run --example register_demo_client --quiet

# Cleanup
echo ""
echo -e "${BLUE}🧹 Cleaning up...${NC}"
kill $SERVER_PID 2>/dev/null || true
wait $SERVER_PID 2>/dev/null || true

echo -e "${GREEN}✅ Demo completed!${NC}"
echo ""
echo "Server log saved to: server.log" 