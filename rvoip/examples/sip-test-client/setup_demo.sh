#!/bin/bash
# Script to set up a complete SIP test environment with terminal windows

# Path to workspace root
WORKSPACE_ROOT="/Users/jonathan/Documents/Work/Rudeless Ventures/rvoip/rvoip"

# Build all components first
echo "Building components..."
cd "$WORKSPACE_ROOT"
cargo build --package sip-test-client --package simple-call-engine

# Check if build succeeded
if [ $? -ne 0 ]; then
    echo "Build failed, exiting."
    exit 1
fi

# Function to open a new terminal window and run a command
open_terminal() {
    local title="$1"
    local cmd="$2"
    
    # On macOS, we use osascript to open a new Terminal window
    osascript << EOF
    tell application "Terminal"
        do script "echo '$title'; cd '$WORKSPACE_ROOT'; $cmd"
        set custom title of front window to "$title"
    end tell
EOF
}

# Start the call engine in a separate terminal
open_terminal "RVOIP Call Engine" "cd examples/simple-call-engine && cargo run --bin simple-call-engine -- --no-auth"

echo "Waiting for call engine to start up..."
sleep 3

# Start the Bob user agent in a separate terminal
open_terminal "SIP User Agent (Bob)" "cd examples/sip-test-client && cargo run --package sip-test-client -- --mode ua --username bob --local-addr 127.0.0.1:5071"

echo "Waiting for Bob to start up..."
sleep 2

# Start the Alice client in a separate terminal
open_terminal "SIP Client (Alice)" "cd examples/sip-test-client && cargo run --package sip-test-client -- --mode call --username alice --target-uri sip:bob@rvoip.local --server-addr 127.0.0.1:5071"

echo "Demo setup complete! Check the terminal windows for logs."
echo "Press Ctrl+C in each window to stop the processes when done." 