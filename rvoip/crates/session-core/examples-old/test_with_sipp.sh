#!/bin/bash

# Test script for session-core SIP server with SIPp
# This script tests the complete INVITE/200 OK/ACK/BYE flow

echo "🧪 Testing session-core SIP Server with SIPp"
echo "=============================================="

# Check if SIPp is installed
if ! command -v sipp &> /dev/null; then
    echo "❌ SIPp is not installed. Please install SIPp first:"
    echo "   macOS: brew install sipp"
    echo "   Ubuntu: sudo apt-get install sipp"
    echo "   Or build from source: https://github.com/SIPp/sipp"
    exit 1
fi

echo "✅ SIPp found: $(which sipp)"

# Check if our server is running
if ! pgrep -f "sipp_server" > /dev/null; then
    echo "❌ SIP server is not running. Please start it first:"
    echo "   cargo run --example sipp_server"
    exit 1
fi

echo "✅ SIP server is running"

# Test parameters
SERVER_IP="127.0.0.1"
SERVER_PORT="5060"
SCENARIO_FILE="examples/sipp_test_basic.xml"

echo ""
echo "📋 Test Configuration:"
echo "  🎯 Server: ${SERVER_IP}:${SERVER_PORT}"
echo "  📄 Scenario: ${SCENARIO_FILE}"
echo "  📞 Test: Single call with 3-second duration"

echo ""
echo "🚀 Starting SIPp test..."
echo "========================"

# Run SIPp test
sipp -sf "${SCENARIO_FILE}" \
     -i "${SERVER_IP}" \
     -p 5061 \
     "${SERVER_IP}:${SERVER_PORT}" \
     -m 1 \
     -r 1 \
     -rp 1000 \
     -trace_msg \
     -trace_shortmsg \
     -max_socket 100

# Check the exit code
if [ $? -eq 0 ]; then
    echo ""
    echo "🎉 SIPp test completed successfully!"
    echo "✅ INVITE/200 OK/ACK/BYE flow working"
    echo "✅ Session-core server handling SIP traffic correctly"
else
    echo ""
    echo "❌ SIPp test failed"
    echo "💡 Check the server logs for details"
    echo "💡 Verify the server is accepting calls automatically"
fi

echo ""
echo "📊 Test Summary:"
echo "  📞 Protocol: SIP/2.0 over UDP"
echo "  🎵 Media: Basic SDP with PCMU codec"
echo "  ⏱️  Call Duration: 3 seconds"
echo "  🔄 Transaction Flow: INVITE → 200 OK → ACK → BYE → 200 OK" 