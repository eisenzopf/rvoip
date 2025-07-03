#!/bin/bash

# Test script for RVOIP UAC/UAS RTP demo

set -e

# Colors for output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}🚀 RVOIP Client-Server RTP Demo${NC}"
echo "================================="

# Function to cleanup
cleanup() {
    echo -e "\n${YELLOW}🧹 Cleaning up...${NC}"
    # Kill server if running
    if [ ! -z "$SERVER_PID" ]; then
        kill $SERVER_PID 2>/dev/null || true
    fi
    echo -e "${GREEN}✅ Cleanup complete${NC}"
}

trap cleanup EXIT

# Build the binaries
echo -e "\n${BLUE}🔨 Building UAC and UAS...${NC}"
cargo build --release --bin uas_server --bin uac_client

# Start the UAS server
echo -e "\n${BLUE}▶️  Starting UAS Server...${NC}"
echo "   SIP Port: 5070"
echo "   Media Ports: 30000-31000"
echo "   RTP Debug: enabled"

cargo run --release --bin uas_server -- --port 5070 --media-port 30000 --rtp-debug &
SERVER_PID=$!

# Wait for server to start
echo -n "   Waiting for server to start"
for i in {1..10}; do
    if lsof -i :5070 >/dev/null 2>&1; then
        echo -e "\n${GREEN}✅ Server is ready${NC}"
        break
    fi
    echo -n "."
    sleep 1
done

# Give server a moment to fully initialize
sleep 2

# Run UAC client with multiple calls
echo -e "\n${BLUE}▶️  Starting UAC Client...${NC}"
echo "   Making 2 calls with 10 second duration each"

RUST_LOG=info,rvoip_client_core=debug,rvoip_media_core=debug target/release/uac_client \
    --server 127.0.0.1:5070 \
    --port 5071 \
    --num-calls 2 \
    --duration 10 \
    --rtp-debug &
UAC_PID=$!

wait $UAC_PID

echo "✅ Test completed!"
echo "======================================="

# Give server a moment to print final statistics
echo "📊 Waiting for server to print final statistics..."
sleep 2

# Send interrupt signal to server for graceful shutdown
echo "📊 Sending shutdown signal to server..."
kill -INT $SERVER_PID 2>/dev/null || true

# Wait a moment for graceful shutdown
sleep 1

# Check the server logs for RTP packet statistics
echo "📊 Check the server logs for RTP packet statistics"
echo "" 