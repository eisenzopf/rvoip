#!/bin/bash
# Script to run a complete SIP call test using the Rust client

# Build the test client from the workspace root
echo "Building SIP test client..."
cd ../..  # Go to workspace root
cargo build --package sip-test-client

# Check if build was successful
if [ $? -ne 0 ]; then
    echo "Build failed, exiting."
    exit 1
fi

cd examples/sip-test-client  # Go back to script directory

# Kill any existing processes
pkill -f sip-test-client || true

# Start the call engine (if needed)
if [ "$1" == "with-engine" ]; then
    echo "Starting call engine..."
    cd ../simple-call-engine
    cargo run --bin simple-call-engine -- --no-auth &
    CALL_ENGINE_PID=$!
    cd ../sip-test-client
    sleep 3  # Wait for call engine to start
fi

# Start the user agent (Bob) in the background
echo "Starting user agent (Bob)..."
cargo run --package sip-test-client -- --mode ua --username bob --local-addr 127.0.0.1:5071 &
UA_PID=$!

# Wait a moment for Bob to start
sleep 2

# Now run the client (Alice) to make a call to Bob
echo "Starting Alice client to call Bob..."
# Directly specify Bob's address for peer-to-peer communication
cargo run --package sip-test-client -- --mode call --username alice --target-uri sip:bob@rvoip.local --server-addr 127.0.0.1:5071

# Clean up
echo "Test complete, cleaning up..."
kill $UA_PID 2>/dev/null || true

if [ "$1" == "with-engine" ]; then
    kill $CALL_ENGINE_PID 2>/dev/null || true
fi

echo "Done!" 