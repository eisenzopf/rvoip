#!/bin/bash

# Script to run two audio peers for testing bidirectional audio streaming
# One peer listens, the other calls - both have real audio!

# Clean up any previous runs
cleanup() {
    echo "🧹 Cleaning up..."
    pkill -f "audio_peer" 2>/dev/null
    exit 0
}

trap cleanup EXIT

# Create logs directory
mkdir -p logs

# Clean up old logs
rm -f logs/peer1.log logs/peer2.log

echo "🚀 Starting Audio Streaming Demo..."
echo "📞 This demo will establish a real audio call between two peers"
echo ""

# Build the demo
echo "🔨 Building audio peer..."
cargo build --release --bin audio_peer

# Start Peer A (listener) on port 5060
echo "👂 Starting Peer A (Alice) as listener on port 5060..."
cargo run --release --bin audio_peer -- \
    --local-ip 127.0.0.1 \
    --local-port 5060 \
    --rtp-port-start 20000 \
    --display-name "Alice" \
    --answer-delay 1 \
    --local-demo-mode \
    > logs/peer1.log 2>&1 &

PEER_A_PID=$!
echo "   PID: $PEER_A_PID"

# Give Peer A time to start
sleep 3

# Start Peer B (caller) on port 5061 (different port!)
echo "📞 Starting Peer B (Bob) as caller on port 5061..."
cargo run --release --bin audio_peer -- \
    --local-ip 127.0.0.1 \
    --local-port 5061 \
    --rtp-port-start 20100 \
    --display-name "Bob" \
    --call 127.0.0.1 \
    --remote-port 5060 \
    --duration 30 \
    --local-demo-mode \
    > logs/peer2.log 2>&1 &

PEER_B_PID=$!
echo "   PID: $PEER_B_PID"

echo ""
echo "🎵 Audio streaming demo is running!"
echo ""
echo "In local demo mode:"
echo "  - Bob (caller) uses microphone only"
echo "  - Alice (listener) uses speakers only"
echo "  - This avoids hardware conflicts on the same computer"
echo ""
echo "📊 Monitoring logs:"
echo "  - Peer A (Alice): tail -f logs/peer1.log"
echo "  - Peer B (Bob): tail -f logs/peer2.log"
echo ""
echo "⏱️  The call will run for 30 seconds..."
echo ""

# Monitor the processes
while true; do
    if ! kill -0 $PEER_A_PID 2>/dev/null; then
        echo "❌ Peer A (Alice) has stopped"
        break
    fi
    if ! kill -0 $PEER_B_PID 2>/dev/null; then
        echo "✅ Peer B (Bob) has completed the call"
        break
    fi
    sleep 1
done

# Give a moment for graceful shutdown
sleep 2

echo ""
echo "🎉 Demo completed!"
echo ""
echo "📋 Check the logs for details:"
echo "  - logs/peer1.log (Alice - listener)"
echo "  - logs/peer2.log (Bob - caller)"
echo "" 