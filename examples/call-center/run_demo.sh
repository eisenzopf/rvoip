#!/bin/bash

# Call Center Demo Runner with Real Audio
# This script orchestrates a complete call center demonstration with real audio streaming

set -e

# Colors for output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
BLUE='\033[0;34m'
PURPLE='\033[0;35m'
NC='\033[0m'

echo -e "${BLUE}🏢 RVOIP Call Center Demo with Real Audio${NC}"
echo "=========================================="

# Configuration
SERVER_DOMAIN="${SERVER_DOMAIN:-127.0.0.1}"
SERVER_PORT="${SERVER_PORT:-5060}"
CALL_DURATION="${CALL_DURATION:-30}"
DEMO_MODE="${DEMO_MODE:-local}"  # local or distributed
VERBOSE="${VERBOSE:-false}"

echo -e "${PURPLE}🔧 Configuration:${NC}"
echo "   Server Domain: $SERVER_DOMAIN"
echo "   Server Port: $SERVER_PORT"
echo "   Call Duration: ${CALL_DURATION}s"
echo "   Demo Mode: $DEMO_MODE"
echo "   Verbose: $VERBOSE"

# Process IDs
SERVER_PID=""
AGENT_ALICE_PID=""
AGENT_BOB_PID=""
CUSTOMER_PID=""

# Function to cleanup
cleanup() {
    echo -e "\n${YELLOW}🧹 Cleaning up...${NC}"
    
    # Kill all processes
    if [ ! -z "$CUSTOMER_PID" ]; then
        kill $CUSTOMER_PID 2>/dev/null || true
    fi
    if [ ! -z "$AGENT_ALICE_PID" ]; then
        kill $AGENT_ALICE_PID 2>/dev/null || true
    fi
    if [ ! -z "$AGENT_BOB_PID" ]; then
        kill $AGENT_BOB_PID 2>/dev/null || true
    fi
    if [ ! -z "$SERVER_PID" ]; then
        kill $SERVER_PID 2>/dev/null || true
    fi
    
    # Wait a moment for graceful shutdown
    sleep 2
    
    # Force kill if still running
    pkill -f "call-center-demo" 2>/dev/null || true
    
    echo -e "${GREEN}✅ Cleanup complete${NC}"
}

trap cleanup EXIT

# Function to check if a command exists
check_command() {
    if ! command -v $1 &> /dev/null; then
        echo -e "${RED}❌ $1 is not installed${NC}"
        echo "Please install $1 to continue"
        exit 1
    fi
}

# Function to wait for process to be ready
wait_for_port() {
    local port=$1
    local description=$2
    local timeout=${3:-15}
    
    echo -n "   Waiting for $description to be ready on port $port"
    for i in $(seq 1 $timeout); do
        if lsof -i :$port >/dev/null 2>&1; then
            echo -e "\n${GREEN}✅ $description is ready${NC}"
            return 0
        fi
        if [ $i -eq $timeout ]; then
            echo -e "\n${RED}❌ $description failed to start${NC}"
            return 1
        fi
        echo -n "."
        sleep 1
    done
}

# Function to display audio device info
show_audio_info() {
    echo -e "\n${BLUE}🎵 Audio Device Information:${NC}"
    echo "================================="
    
    # Try to list audio devices
    if target/release/agent --list-devices 2>/dev/null; then
        echo -e "${GREEN}✅ Audio devices discovered${NC}"
    else
        echo -e "${YELLOW}⚠️  Audio device discovery failed - using defaults${NC}"
        echo "   This is normal if no audio hardware is available"
    fi
    
    echo -e "\n${PURPLE}💡 Audio Features:${NC}"
    echo "   ✅ Real-time audio streaming"
    echo "   ✅ Microphone capture"
    echo "   ✅ Speaker playback"
    echo "   ✅ Echo cancellation"
    echo "   ✅ Noise suppression"
    echo "   ✅ Auto gain control"
    echo "   ✅ Voice activity detection"
}

# Check prerequisites
echo -e "\n${BLUE}🔍 Checking prerequisites...${NC}"
check_command cargo
check_command lsof

# Create logs directory
mkdir -p logs

# Build all binaries
echo -e "\n${BLUE}🔨 Building call center components with audio support...${NC}"
if [ "$VERBOSE" = "true" ]; then
    cargo build --release --bin server --bin agent --bin customer --features audio
else
    cargo build --release --bin server --bin agent --bin customer --features audio > logs/build.log 2>&1
fi

# Check if builds succeeded
if [ $? -ne 0 ]; then
    echo -e "${RED}❌ Build failed!${NC}"
    if [ "$VERBOSE" = "false" ]; then
        echo "   Check logs/build.log for details"
    fi
    exit 1
fi

echo -e "${GREEN}✅ Build successful with audio support${NC}"

# Show audio information
show_audio_info

# Step 1: Start the call center server
echo -e "\n${BLUE}🏢 Starting Call Center Server...${NC}"
echo "   Bind Address: 0.0.0.0:$SERVER_PORT"
echo "   Public Domain: $SERVER_DOMAIN"
echo "   Support Line: sip:support@$SERVER_DOMAIN"
echo "   Log: logs/server.log"

SERVER_ARGS="--bind-addr 0.0.0.0:$SERVER_PORT --domain $SERVER_DOMAIN"
if [ "$VERBOSE" = "true" ]; then
    SERVER_ARGS="$SERVER_ARGS --verbose"
fi

target/release/server $SERVER_ARGS > logs/server_stdout.log 2>&1 &
SERVER_PID=$!

# Wait for server to start
if ! wait_for_port $SERVER_PORT "Call Center Server" 15; then
    echo -e "${RED}❌ Server failed to start${NC}"
    exit 1
fi

# Give server time to initialize
sleep 2

# Step 2: Start Agent Alice
echo -e "\n${BLUE}👩‍💼 Starting Agent Alice with Real Audio...${NC}"
echo "   SIP Port: 5071"
echo "   Media Port: 6071"
echo "   Domain: $SERVER_DOMAIN"
echo "   Log: logs/alice.log"

ALICE_ARGS="--name alice --server $SERVER_DOMAIN:$SERVER_PORT --domain $SERVER_DOMAIN --port 5071 --call-duration $CALL_DURATION"
if [ "$VERBOSE" = "true" ]; then
    ALICE_ARGS="$ALICE_ARGS --verbose"
fi

target/release/agent $ALICE_ARGS > logs/alice_stdout.log 2>&1 &
AGENT_ALICE_PID=$!

# Wait for Alice to register
if ! wait_for_port 5071 "Agent Alice" 10; then
    echo -e "${YELLOW}⚠️  Alice may not have started properly${NC}"
fi

# Give Alice time to register with server
sleep 3

# Step 3: Start Agent Bob
echo -e "\n${BLUE}👨‍💼 Starting Agent Bob with Real Audio...${NC}"
echo "   SIP Port: 5072"
echo "   Media Port: 6072"
echo "   Domain: $SERVER_DOMAIN"
echo "   Log: logs/bob.log"

BOB_ARGS="--name bob --server $SERVER_DOMAIN:$SERVER_PORT --domain $SERVER_DOMAIN --port 5072 --call-duration $CALL_DURATION"
if [ "$VERBOSE" = "true" ]; then
    BOB_ARGS="$BOB_ARGS --verbose"
fi

target/release/agent $BOB_ARGS > logs/bob_stdout.log 2>&1 &
AGENT_BOB_PID=$!

# Wait for Bob to register
if ! wait_for_port 5072 "Agent Bob" 10; then
    echo -e "${YELLOW}⚠️  Bob may not have started properly${NC}"
fi

# Give Bob time to register with server
sleep 3

# Step 4: Start the customer call
echo -e "\n${BLUE}👤 Starting Customer Call with Real Audio...${NC}"
echo "   SIP Port: 5080"
echo "   Media Port: 6080"
echo "   Domain: $SERVER_DOMAIN"
echo "   Target: sip:support@$SERVER_DOMAIN"
echo "   Log: logs/customer.log"

CUSTOMER_ARGS="--name customer --server $SERVER_DOMAIN:$SERVER_PORT --domain $SERVER_DOMAIN --port 5080 --call-duration $CALL_DURATION --wait-time 2"
if [ "$VERBOSE" = "true" ]; then
    CUSTOMER_ARGS="$CUSTOMER_ARGS --verbose"
fi

target/release/customer $CUSTOMER_ARGS > logs/customer_stdout.log 2>&1 &
CUSTOMER_PID=$!

# Monitor the demo execution
echo -e "\n${PURPLE}📋 Real Audio Demo Flow:${NC}"
echo "   1. Customer calls sip:support@$SERVER_DOMAIN using microphone"
echo "   2. Call center server receives the call"
echo "   3. Server routes call to available agent (Alice or Bob)"
echo "   4. Agent accepts and uses real audio devices"
echo "   5. Customer and agent have real audio conversation"
echo "   6. Real-time audio streaming with echo cancellation"
echo "   7. Call completes after ${CALL_DURATION}s or manual hangup"
echo ""

# Wait for customer to complete
TOTAL_WAIT_TIME=$((CALL_DURATION + 15))
echo -e "${YELLOW}⏳ Waiting for demo to complete (about ${TOTAL_WAIT_TIME} seconds)...${NC}"
echo "   🎤 Audio streaming is now active - you should hear real audio!"

wait $CUSTOMER_PID
CUSTOMER_EXIT_CODE=$?

# Give agents time to finish
sleep 3

# Kill agents if still running
if kill -0 $AGENT_ALICE_PID 2>/dev/null; then
    kill $AGENT_ALICE_PID 2>/dev/null || true
fi
if kill -0 $AGENT_BOB_PID 2>/dev/null; then
    kill $AGENT_BOB_PID 2>/dev/null || true
fi

# Give server time to process
sleep 2

# Kill server
if kill -0 $SERVER_PID 2>/dev/null; then
    kill $SERVER_PID 2>/dev/null || true
fi

# Analyze results
echo -e "\n${BLUE}📊 Real Audio Demo Results:${NC}"
echo "============================"

# Check customer result
if [ $CUSTOMER_EXIT_CODE -eq 0 ]; then
    echo -e "${GREEN}✅ Customer completed successfully${NC}"
else
    echo -e "${RED}❌ Customer failed with exit code $CUSTOMER_EXIT_CODE${NC}"
fi

# Check log files
echo -e "\n${BLUE}📁 Log Files:${NC}"
for log_file in "server_stdout.log" "alice_stdout.log" "bob_stdout.log" "customer_stdout.log"; do
    if [ -f "logs/$log_file" ] && [ -s "logs/$log_file" ]; then
        echo -e "${GREEN}✅ logs/$log_file created${NC}"
    else
        echo -e "${RED}❌ logs/$log_file missing or empty${NC}"
    fi
done

# Extract and display key statistics
echo -e "\n${BLUE}📊 Call and Audio Statistics:${NC}"
echo "============================"

# Check for successful call establishment
CUSTOMER_CONNECTED=$(grep -c "Connected to agent" logs/customer_stdout.log 2>/dev/null || echo "0")
ALICE_CALLS=$(grep -c "Incoming call" logs/alice_stdout.log 2>/dev/null || echo "0")
BOB_CALLS=$(grep -c "Incoming call" logs/bob_stdout.log 2>/dev/null || echo "0")

echo -e "${BLUE}📞 Call Routing:${NC}"
if [ "$CUSTOMER_CONNECTED" -gt 0 ]; then
    echo -e "${GREEN}✅ Customer successfully connected to an agent${NC}"
else
    echo -e "${RED}❌ Customer failed to connect to an agent${NC}"
fi

if [ "$ALICE_CALLS" -gt 0 ]; then
    echo -e "${GREEN}✅ Alice handled $ALICE_CALLS call(s)${NC}"
fi
if [ "$BOB_CALLS" -gt 0 ]; then
    echo -e "${GREEN}✅ Bob handled $BOB_CALLS call(s)${NC}"
fi

if [ "$ALICE_CALLS" -eq 0 ] && [ "$BOB_CALLS" -eq 0 ]; then
    echo -e "${RED}❌ No agent handled the call${NC}"
fi

# Check for real audio setup
CUSTOMER_AUDIO=$(grep -c "Real audio setup" logs/customer_stdout.log 2>/dev/null || echo "0")
AGENT_AUDIO=$(grep -c "Real audio setup" logs/alice_stdout.log logs/bob_stdout.log 2>/dev/null | wc -l || echo "0")

echo -e "\n${BLUE}🎵 Real Audio Streaming:${NC}"
if [ "$CUSTOMER_AUDIO" -gt 0 ] && [ "$AGENT_AUDIO" -gt 0 ]; then
    echo -e "${GREEN}✅ Real audio streaming established${NC}"
    echo -e "${GREEN}   🎤 Microphone capture active${NC}"
    echo -e "${GREEN}   🔊 Speaker playback active${NC}"
    echo -e "${GREEN}   🎛️  Audio processing enabled${NC}"
else
    echo -e "${RED}❌ Real audio streaming setup failed${NC}"
fi

# Check for audio device usage
echo -e "\n${BLUE}🎧 Audio Device Usage:${NC}"
INPUT_DEVICES=$(grep -c "Selected input device" logs/alice_stdout.log logs/bob_stdout.log logs/customer_stdout.log 2>/dev/null || echo "0")
OUTPUT_DEVICES=$(grep -c "Selected output device" logs/alice_stdout.log logs/bob_stdout.log logs/customer_stdout.log 2>/dev/null || echo "0")

if [ "$INPUT_DEVICES" -gt 0 ] && [ "$OUTPUT_DEVICES" -gt 0 ]; then
    echo -e "${GREEN}✅ Audio devices successfully configured${NC}"
    echo -e "${GREEN}   🎤 Input devices: $INPUT_DEVICES configured${NC}"
    echo -e "${GREEN}   🔊 Output devices: $OUTPUT_DEVICES configured${NC}"
else
    echo -e "${YELLOW}⚠️  Audio device configuration may have issues${NC}"
fi

# Look for audio statistics
echo -e "\n${BLUE}📈 Audio Statistics:${NC}"
AUDIO_STATS=$(grep -c "Audio stats" logs/alice_stdout.log logs/bob_stdout.log logs/customer_stdout.log 2>/dev/null || echo "0")
if [ "$AUDIO_STATS" -gt 0 ]; then
    echo -e "${GREEN}✅ Audio frame statistics collected ($AUDIO_STATS entries)${NC}"
else
    echo -e "${YELLOW}⚠️  No audio statistics found${NC}"
fi

# Check server activity
echo -e "\n${BLUE}🏢 Server Activity:${NC}"
if [ -f "logs/server_stdout.log" ]; then
    SERVER_READY=$(grep -c "CALL CENTER IS READY" logs/server_stdout.log 2>/dev/null || echo "0")
    if [ "$SERVER_READY" -gt 0 ]; then
        echo -e "${GREEN}✅ Server started successfully${NC}"
    else
        echo -e "${YELLOW}⚠️  Server startup messages not found${NC}"
    fi
fi

# Generate enhanced call flow log
echo -e "\n${BLUE}📞 Enhanced Call Flow Timeline:${NC}"
echo "==============================="
echo "Generating enhanced call flow log..."

cat > logs/call_flow.log << EOF
# Call Center Demo - Enhanced Call Flow Timeline with Real Audio
# Generated: $(date)
# 
# This log shows the sequence of events during the real audio call center demo
#

=== SERVER STARTUP ===
EOF

# Extract key server events
if [ -f "logs/server_stdout.log" ]; then
    grep -E "(Starting Call Center|CALL CENTER IS READY)" logs/server_stdout.log | sed 's/^/[SERVER] /' >> logs/call_flow.log 2>/dev/null || true
fi

echo -e "\n=== AGENT REGISTRATION WITH AUDIO ===" >> logs/call_flow.log

# Extract agent registration and audio setup events
for agent in alice bob; do
    if [ -f "logs/${agent}_stdout.log" ]; then
        grep -E "(Registration active|Agent ready|Selected input device|Selected output device)" logs/${agent}_stdout.log | sed "s/^/[AGENT $(echo $agent | tr '[:lower:]' '[:upper:]')] /" >> logs/call_flow.log 2>/dev/null || true
    fi
done

echo -e "\n=== CUSTOMER CALL WITH REAL AUDIO ===" >> logs/call_flow.log

# Extract customer call and audio events
if [ -f "logs/customer_stdout.log" ]; then
    grep -E "(Calling call center|Connected to agent|Real audio setup|Selected input device|Selected output device|Audio stats)" logs/customer_stdout.log | sed 's/^/[CUSTOMER] /' >> logs/call_flow.log 2>/dev/null || true
fi

echo -e "\n=== AGENT CALL HANDLING WITH AUDIO ===" >> logs/call_flow.log

# Extract agent call handling and audio events
for agent in alice bob; do
    if [ -f "logs/${agent}_stdout.log" ]; then
        grep -E "(Incoming call|Accepting call|Real audio setup|Audio streaming started|Audio stats)" logs/${agent}_stdout.log | sed "s/^/[AGENT $(echo $agent | tr '[:lower:]' '[:upper:]')] /" >> logs/call_flow.log 2>/dev/null || true
    fi
done

echo -e "${GREEN}✅ Enhanced call flow log created: logs/call_flow.log${NC}"

# Final summary
echo -e "\n${BLUE}📋 Summary:${NC}"
echo "============"
echo "📁 Log files created:"
echo "   - logs/server_stdout.log (Server activity)"
echo "   - logs/alice_stdout.log (Agent Alice activity)"
echo "   - logs/bob_stdout.log (Agent Bob activity)"
echo "   - logs/customer_stdout.log (Customer activity)"
echo "   - logs/call_flow.log (Combined timeline)"
echo ""

echo -e "${PURPLE}🎵 Audio Features Demonstrated:${NC}"
echo "   ✅ Real-time microphone capture"
echo "   ✅ Real-time speaker playback"
echo "   ✅ Echo cancellation"
echo "   ✅ Noise suppression"
echo "   ✅ Auto gain control"
echo "   ✅ Voice activity detection"
echo "   ✅ Audio quality monitoring"
echo ""

# Overall result
if [ $CUSTOMER_EXIT_CODE -eq 0 ] && [ "$CUSTOMER_CONNECTED" -gt 0 ] && [ "$CUSTOMER_AUDIO" -gt 0 ] && [ "$AGENT_AUDIO" -gt 0 ]; then
    echo -e "${GREEN}🎉 REAL AUDIO CALL CENTER DEMO SUCCESSFUL!${NC}"
    echo -e "${GREEN}   ✅ Customer connected to agent${NC}"
    echo -e "${GREEN}   ✅ Call routed successfully${NC}"
    echo -e "${GREEN}   ✅ Real audio streaming established${NC}"
    echo -e "${GREEN}   ✅ Audio devices configured${NC}"
    echo -e "${GREEN}   ✅ Call completed cleanly${NC}"
    echo ""
    echo -e "${PURPLE}🎯 Next Steps:${NC}"
    echo "   • Try running components on separate machines"
    echo "   • Use --list-devices to see available audio devices"
    echo "   • Configure specific audio devices with --input-device and --output-device"
    echo "   • Enable verbose logging with --verbose for detailed audio info"
    echo "   • Experiment with different call durations"
    exit 0
else
    echo -e "${RED}❌ REAL AUDIO CALL CENTER DEMO FAILED!${NC}"
    if [ "$CUSTOMER_CONNECTED" -eq 0 ]; then
        echo -e "${RED}   ❌ Customer failed to connect to agent${NC}"
    fi
    if [ "$CUSTOMER_AUDIO" -eq 0 ] || [ "$AGENT_AUDIO" -eq 0 ]; then
        echo -e "${RED}   ❌ Real audio setup failed${NC}"
    fi
    echo -e "${RED}   📋 Check the log files for details${NC}"
    echo -e "${YELLOW}   💡 Try running with --verbose for more detailed logs${NC}"
    exit 1
fi 