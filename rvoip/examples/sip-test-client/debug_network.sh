#!/bin/bash
# Script to test and debug SIP network connectivity

# Kill any existing processes on the test ports
echo "Checking for existing processes on SIP ports..."
lsof -i:5070,5071 || echo "No processes found on ports 5070 and 5071."

# Kill any existing processes if they exist
pkill -f sip-test-client || echo "No SIP test client processes found."

# Build the test client
echo "Building SIP test client..."
cd ../..  # Go to workspace root
cargo build --package sip-test-client

# Check if build was successful
if [ $? -ne 0 ]; then
    echo "Build failed, exiting."
    exit 1
fi

# Open a terminal and monitor UDP traffic on the relevant ports
echo "Opening terminal to monitor network traffic..."
osascript << EOF
tell application "Terminal"
    do script "echo 'Monitoring SIP network traffic...'; sudo tcpdump -i lo0 -n udp port 5070 or udp port 5071"
end tell
EOF

# Wait for tcpdump to start
sleep 2

# Start Bob on port 5071 in the background
echo "Starting Bob on port 5071..."
cd examples/sip-test-client
RUST_LOG=trace cargo run --package sip-test-client -- --mode ua --username bob --local-addr 127.0.0.1:5071 > bob.log 2>&1 &
BOB_PID=$!

# Wait for Bob to start up
sleep 2

# Start Alice to call Bob
echo "Starting Alice to call Bob..."
RUST_LOG=trace cargo run --package sip-test-client -- --mode call --username alice --target-uri sip:bob@rvoip.local --server-addr 127.0.0.1:5071 --local-addr 127.0.0.1:5070 > alice.log 2>&1 &
ALICE_PID=$!

# Wait for possible communication
echo "Clients running, watching for 30 seconds..."
sleep 30

# Check if processes are still running
if ps -p $BOB_PID > /dev/null; then
    echo "Bob is still running (PID: $BOB_PID)"
else
    echo "Bob has exited!"
fi

if ps -p $ALICE_PID > /dev/null; then
    echo "Alice is still running (PID: $ALICE_PID)"
else
    echo "Alice has exited!"
fi

# Display log summaries
echo
echo "===== ALICE LOG SUMMARY ====="
grep -E "INFO|ERROR|WARN|DEBUG|TRACE" alice.log | tail -20
echo
echo "===== BOB LOG SUMMARY ====="
grep -E "INFO|ERROR|WARN|DEBUG|TRACE" bob.log | tail -20
echo

# Try a simple UDP connectivity test
echo "Running simple UDP connectivity test..."
nc -u -l 5072 > /dev/null 2>&1 &
NC_PID=$!
sleep 1
echo "Test message" | nc -u 127.0.0.1 5072
kill $NC_PID 2>/dev/null

echo
echo "Test complete. Log files are available at:"
echo "  - Alice: $(pwd)/alice.log"
echo "  - Bob: $(pwd)/bob.log"
echo
echo "You can kill the clients with:"
echo "  kill $BOB_PID $ALICE_PID" 