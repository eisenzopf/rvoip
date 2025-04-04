#!/bin/bash
# Minimal test script to send a REGISTER request with verbose debugging

# Configuration
SERVER_IP="127.0.0.1"
SERVER_PORT="5060"
CLIENT_PORT="5070"
CALL_ID="register-test-$(date +%s)"
TAG="tag-$(date +%s)"
OUTPUT_FILE="/tmp/register_response.txt"

echo "======= SIP REGISTER Debug Test ======="
echo "Server: $SERVER_IP:$SERVER_PORT"
echo "Client port: $CLIENT_PORT"
echo "Call-ID: $CALL_ID"

# Clear any previous output
> $OUTPUT_FILE

# Check if server port is actually open
echo -e "\nChecking if server is listening on $SERVER_IP:$SERVER_PORT..."
nc -z -v -u -w1 $SERVER_IP $SERVER_PORT 2>&1 || echo "Warning: Could not connect to server port"

# Set up a listener that writes to a file with tcpdump for debugging
echo -e "\nStarting tcpdump to monitor SIP traffic..."
sudo tcpdump -i any -n -s0 port $SERVER_PORT or port $CLIENT_PORT -vvv &
TCPDUMP_PID=$!

# Give tcpdump time to start
sleep 1

echo -e "\nStarting SIP listener on port $CLIENT_PORT..."
nc -u -l $CLIENT_PORT > $OUTPUT_FILE &
LISTENER_PID=$!

# Give the listener time to start
sleep 1

# Create the SIP request in a separate file for easier debugging
REQUEST_FILE="/tmp/register_request.txt"
cat << EOF > $REQUEST_FILE
REGISTER sip:rvoip.local SIP/2.0
Via: SIP/2.0/UDP 127.0.0.1:$CLIENT_PORT;branch=z9hG4bK-$(date +%s)-register
Max-Forwards: 70
From: <sip:test@rvoip.local>;tag=$TAG
To: <sip:test@rvoip.local>
Call-ID: $CALL_ID
CSeq: 1 REGISTER
Contact: <sip:test@127.0.0.1:$CLIENT_PORT>
Expires: 3600
User-Agent: RVOIP-Test-Client/0.1.0
Content-Length: 0

EOF

# Send the request with verbose option
echo -e "\nSending REGISTER request..."
cat $REQUEST_FILE
cat $REQUEST_FILE | nc -u -v $SERVER_IP $SERVER_PORT

# Wait for response with a timeout of 5 seconds
echo -e "\nWaiting for response (timeout: 5s)..."
sleep 5

# Check if we received a response
echo -e "\nChecking for response..."
if [ -s "$OUTPUT_FILE" ]; then
    echo "Response received:"
    cat "$OUTPUT_FILE"
else
    echo "No response received within timeout period."
    echo -e "\nDebugging information:"
    echo "1. Current netstat for SIP ports:"
    netstat -an | grep -E "$SERVER_PORT|$CLIENT_PORT"
    echo "2. Process listening on server port:"
    sudo lsof -i:$SERVER_PORT
    echo "3. Possible issues:"
    echo "   - Server is not running"
    echo "   - Server is not listening on $SERVER_IP:$SERVER_PORT"
    echo "   - Server is not processing SIP requests correctly"
    echo "   - Firewall is blocking traffic"
    echo "   - Server may need to explicitly bind to address $SERVER_IP rather than 0.0.0.0"
fi

# Clean up
echo -e "\nCleaning up..."
kill $LISTENER_PID 2>/dev/null || true
kill $TCPDUMP_PID 2>/dev/null || true
rm -f $OUTPUT_FILE $REQUEST_FILE

echo -e "\nTest completed." 