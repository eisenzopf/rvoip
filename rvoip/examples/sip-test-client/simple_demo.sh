#!/bin/bash
# Simple script to start two SIP clients for a direct call demo

# Build the SIP test client
echo "Building SIP test client..."
cd "$(dirname "$0")/../.."
cargo build --package sip-test-client

# Check if build succeeded
if [ $? -ne 0 ]; then
    echo "Build failed, exiting."
    exit 1
fi

# Function to run a client in a new terminal
run_client() {
    local title="$1"
    local mode="$2"
    local username="$3"
    local local_port="$4"
    local args="$5"
    
    # Create the full command
    local cmd="cd '$(pwd)' && RUST_LOG=debug cargo run --package sip-test-client -- --mode $mode --username $username --local-addr 127.0.0.1:$local_port $args"
    
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
echo "This will start two SIP clients in separate terminal windows:"
echo "  1. Bob - User Agent mode (listening for calls) on port 5071"
echo "  2. Alice - Call mode (making a call to Bob) on port 5070"
echo ""
read -p "Press Enter to continue, or Ctrl+C to cancel..."

# Start Bob in user agent mode (listening)
run_client "SIP UA (Bob)" "ua" "bob" "5071" ""

echo "Waiting for Bob to start up..."
sleep 2

# Start Alice in call mode (calling Bob)
run_client "SIP Caller (Alice)" "call" "alice" "5070" "--server-addr 127.0.0.1:5071 --target-uri sip:bob@rvoip.local"

echo ""
echo "Clients started. To end the call, press Ctrl+C in the Alice (caller) window."
echo "When you're done, close both terminal windows." 