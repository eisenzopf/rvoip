#!/bin/bash
# Script to run a test of the SIP call engine without authentication for testing

# Build the call engine if needed
cargo build --bin simple-call-engine

# Kill any existing call engine that might be running
pkill -f simple-call-engine || true

# Run the call engine with authentication disabled
cargo run --bin simple-call-engine -- --no-auth &
CALL_ENGINE_PID=$!

# Wait for the call engine to start up
echo "Waiting for call engine to start up..."
sleep 2

# Run the test script
echo "Running test script..."
./test_call.sh

# Kill the call engine when done
echo "Cleaning up..."
kill $CALL_ENGINE_PID 