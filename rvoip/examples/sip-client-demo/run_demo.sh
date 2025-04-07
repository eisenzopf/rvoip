#!/bin/bash
# Simple script to start both SIP client demo components

# Build the SIP client demo
echo "Building SIP client demo..."
cd "$(dirname "$0")/../.."
cargo build --package sip-client-demo

# Check if build succeeded
if [ $? -ne 0 ]; then
    echo "Build failed, exiting."
    exit 1
fi

# Kill any existing instances 
echo "Stopping any running instances..."
pkill -f "sip-client-demo" || true

# Function to run a component in a new terminal
run_client() {
    local title="$1"
    local binary="$2"
    local args="$3"
    
    # Create the full command
    local cmd="cd '$(pwd)' && RUST_LOG=debug cargo run --package sip-client-demo --bin $binary -- $args"
    
    # Display the command
    echo "Starting $title with command:"
    echo "  $cmd"
    
    # On macOS, use osascript to open a new Terminal window
    # On Linux, you'd use something like gnome-terminal, xterm, etc.
    if [[ "$OSTYPE" == "darwin"* ]]; then
        osascript << EOF
        tell application "Terminal"
            do script "echo '$title'; $cmd"
            set custom title of front window to "$title"
        end tell
EOF
    else
        # For Linux, adjust as needed for your terminal
        if command -v gnome-terminal &> /dev/null; then
            gnome-terminal -- bash -c "echo '$title'; $cmd; exec bash"
        elif command -v xterm &> /dev/null; then
            xterm -title "$title" -e "echo '$title'; $cmd; exec bash" &
        else
            echo "Could not find a suitable terminal emulator. Please run this command manually:"
            echo "$cmd"
        fi
    fi
}

# Ask for confirmation before starting
echo "This will start two SIP client demo components in separate terminal windows:"
echo "  1. Receiver - Listens for incoming calls on port 5071"
echo "  2. Caller - Makes a call to the receiver from port 5070"
echo ""
echo "Press Enter to continue, or Ctrl+C to cancel..."
read

# Start receiver in user agent mode (listening)
run_client "SIP Receiver (Bob)" "receiver" "--local-addr 127.0.0.1:5071 --username bob"

echo "Waiting for receiver to start up..."
sleep 5

# Start caller 
run_client "SIP Caller (Alice)" "caller" "--local-addr 127.0.0.1:5070 --username alice --server-addr 127.0.0.1:5071 --target-uri sip:bob@rvoip.local"

echo ""
echo "Demo started! You should see a call being established."
echo "The call will last for 30 seconds by default."
echo "When you're done, close both terminal windows." 