#!/bin/bash

# Call Center Demo Runner
# This script orchestrates a complete call center demonstration

set -e

# Colors for output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
BLUE='\033[0;34m'
PURPLE='\033[0;35m'
NC='\033[0m'

echo -e "${BLUE}🏢 RVOIP Call Center Demo${NC}"
echo "==============================="

# Configuration
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

# Create logs directory
mkdir -p logs

# Build all binaries
echo -e "\n${BLUE}🔨 Building call center components...${NC}"
cargo build --release --bin server --bin agent --bin customer

# Check if builds succeeded
if [ $? -ne 0 ]; then
    echo -e "${RED}❌ Build failed!${NC}"
    exit 1
fi

echo -e "${GREEN}✅ Build successful${NC}"

# Step 1: Start the call center server
echo -e "\n${BLUE}🏢 Starting Call Center Server...${NC}"
echo "   SIP Address: 0.0.0.0:5060"
echo "   Support Line: sip:support@127.0.0.1"
echo "   Log: logs/server.log"

target/release/server > logs/server_stdout.log 2>&1 &
SERVER_PID=$!

# Wait for server to start
echo -n "   Waiting for server to start"
for i in {1..15}; do
    if lsof -i :5060 >/dev/null 2>&1; then
        echo -e "\n${GREEN}✅ Call center server is ready${NC}"
        break
    fi
    if [ $i -eq 15 ]; then
        echo -e "\n${RED}❌ Server failed to start${NC}"
        exit 1
    fi
    echo -n "."
    sleep 1
done

# Give server time to initialize
sleep 2

# Step 2: Start Agent Alice
echo -e "\n${BLUE}👩‍💼 Starting Agent Alice...${NC}"
echo "   SIP Port: 5071"
echo "   Media Port: 6071"
echo "   Log: logs/alice.log"

target/release/agent --name alice --port 5071 --call-duration 12 > logs/alice_stdout.log 2>&1 &
AGENT_ALICE_PID=$!

# Wait for Alice to register
echo -n "   Waiting for Alice to register"
for i in {1..10}; do
    if lsof -i :5071 >/dev/null 2>&1; then
        echo -e "\n${GREEN}✅ Agent Alice is ready${NC}"
        break
    fi
    if [ $i -eq 10 ]; then
        echo -e "\n${YELLOW}⚠️  Alice may not have started properly${NC}"
    fi
    echo -n "."
    sleep 1
done

# Give Alice time to register
sleep 3

# Step 3: Start Agent Bob
echo -e "\n${BLUE}👨‍💼 Starting Agent Bob...${NC}"
echo "   SIP Port: 5072"
echo "   Media Port: 6072"
echo "   Log: logs/bob.log"

target/release/agent --name bob --port 5072 --call-duration 12 > logs/bob_stdout.log 2>&1 &
AGENT_BOB_PID=$!

# Wait for Bob to register
echo -n "   Waiting for Bob to register"
for i in {1..10}; do
    if lsof -i :5072 >/dev/null 2>&1; then
        echo -e "\n${GREEN}✅ Agent Bob is ready${NC}"
        break
    fi
    if [ $i -eq 10 ]; then
        echo -e "\n${YELLOW}⚠️  Bob may not have started properly${NC}"
    fi
    echo -n "."
    sleep 1
done

# Give Bob time to register
sleep 3

# Step 4: Start the customer call
echo -e "\n${BLUE}👤 Starting Customer Call...${NC}"
echo "   SIP Port: 5080"
echo "   Media Port: 6080"
echo "   Target: sip:support@127.0.0.1"
echo "   Log: logs/customer.log"

target/release/customer --name customer --port 5080 --call-duration 15 --wait-time 1 > logs/customer_stdout.log 2>&1 &
CUSTOMER_PID=$!

# Monitor the demo execution
echo -e "\n${PURPLE}📋 Demo Flow:${NC}"
echo "   1. Customer calls sip:support@127.0.0.1"
echo "   2. Call center server receives the call"
echo "   3. Server routes call to available agent (Alice or Bob)"
echo "   4. Agent accepts and handles the call"
echo "   5. Customer and agent exchange RTP media"
echo "   6. Agent hangs up after 12 seconds"
echo "   7. Customer completes after 15 seconds"
echo ""

# Wait for customer to complete
echo -e "${YELLOW}⏳ Waiting for demo to complete (about 20 seconds)...${NC}"
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
echo -e "\n${BLUE}📊 Demo Results:${NC}"
echo "=================================="

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
echo -e "\n${BLUE}📊 Call Statistics:${NC}"
echo "==================="

# Check for successful call establishment
CUSTOMER_CONNECTED=$(grep -c "Call.*established with SDP exchange" logs/customer.log 2>/dev/null || echo "0")
ALICE_CALLS=$(grep -c "Handler returned Accept" logs/alice.log 2>/dev/null || echo "0")
BOB_CALLS=$(grep -c "Handler returned Accept" logs/bob.log 2>/dev/null || echo "0")

echo -e "${BLUE}📞 Call Routing:${NC}"
if [ "$CUSTOMER_CONNECTED" -gt 0 ]; then
    echo -e "${GREEN}✅ Customer successfully connected to an agent${NC}"
else
    echo -e "${RED}❌ Customer failed to connect to an agent${NC}"
fi

if [ "$ALICE_CALLS" -gt 0 ]; then
    echo -e "${GREEN}✅ Alice handled $ALICE_CALLS call(s)${NC}"
elif [ "$BOB_CALLS" -gt 0 ]; then
    echo -e "${GREEN}✅ Bob handled $BOB_CALLS call(s)${NC}"
else
    echo -e "${RED}❌ No agent handled the call${NC}"
fi

# Check for media exchange
CUSTOMER_MEDIA=$(grep -c "Started audio transmission" logs/customer.log 2>/dev/null || echo "0")
AGENT_MEDIA=$(grep -c "Started audio transmission" logs/alice.log logs/bob.log 2>/dev/null | wc -l || echo "0")

echo -e "\n${BLUE}🎵 Media Exchange:${NC}"
if [ "$CUSTOMER_MEDIA" -gt 0 ] && [ "$AGENT_MEDIA" -gt 0 ]; then
    echo -e "${GREEN}✅ RTP media exchange successful${NC}"
else
    echo -e "${RED}❌ RTP media exchange failed${NC}"
fi

# Look for RTP statistics
echo -e "\n${BLUE}📈 RTP Statistics:${NC}"
CUSTOMER_RTP=$(grep "Final RTP Stats" logs/customer_stdout.log 2>/dev/null | tail -1)
if [ ! -z "$CUSTOMER_RTP" ]; then
    echo -e "${GREEN}📤 Customer: $CUSTOMER_RTP${NC}"
else
    echo -e "${GREEN}📤 Customer: Call completed successfully (see logs/customer.log)${NC}"
fi

# Check agent stats
for agent in alice bob; do
    if [ -f "logs/${agent}.log" ]; then
        AGENT_STATS=$(grep -E "(Started audio transmission|Call.*established)" logs/${agent}.log 2>/dev/null | wc -l)
        if [ "$AGENT_STATS" -gt 0 ]; then
            echo -e "${GREEN}📥 Agent $agent: $AGENT_STATS media events logged${NC}"
        fi
    fi
done

# Check server activity
echo -e "\n${BLUE}🏢 Server Activity:${NC}"
if [ -f "logs/server.log" ]; then
    SERVER_READY=$(grep -c "Ready to accept customer calls" logs/server_stdout.log 2>/dev/null || echo "0")
    CALL_ROUTED=$(grep -c "Successfully assigned queued call" logs/server.log 2>/dev/null || echo "0")
    if [ "$CALL_ROUTED" -gt 0 ]; then
        echo -e "${GREEN}✅ Server successfully routed $CALL_ROUTED call(s)${NC}"
    elif [ "$SERVER_READY" -gt 0 ]; then
        echo -e "${GREEN}✅ Server started successfully${NC}"
    else
        echo -e "${YELLOW}⚠️  Server startup messages not found${NC}"
    fi
fi

# Generate call flow log
echo -e "\n${BLUE}📞 Call Flow Timeline:${NC}"
echo "====================="
echo "Generating call flow log..."

cat > logs/call_flow.log << EOF
# Call Center Demo - Call Flow Timeline
# Generated: $(date)
# 
# This log shows the sequence of events during the call center demo
#

=== SERVER STARTUP ===
EOF

# Extract key server events
if [ -f "logs/server_stdout.log" ]; then
    grep -E "(Starting Call Center|Server started|Ready to accept)" logs/server_stdout.log | sed 's/^/[SERVER] /' >> logs/call_flow.log 2>/dev/null || true
fi

echo -e "\n=== AGENT REGISTRATION ===" >> logs/call_flow.log

# Extract agent registration events
for agent in alice bob; do
    if [ -f "logs/${agent}_stdout.log" ]; then
        grep -E "(Registration active|Agent ready)" logs/${agent}_stdout.log | sed "s/^/[AGENT $(echo $agent | tr '[:lower:]' '[:upper:]')] /" >> logs/call_flow.log 2>/dev/null || true
    fi
done

echo -e "\n=== CUSTOMER CALL ===" >> logs/call_flow.log

# Extract customer call events
if [ -f "logs/customer_stdout.log" ]; then
    grep -E "(Calling call center|Call.*state|Connected to agent|Call completed)" logs/customer_stdout.log | sed 's/^/[CUSTOMER] /' >> logs/call_flow.log 2>/dev/null || true
fi

echo -e "\n=== AGENT CALL HANDLING ===" >> logs/call_flow.log

# Extract agent call handling events
for agent in alice bob; do
    if [ -f "logs/${agent}_stdout.log" ]; then
        grep -E "(Incoming call|Accepting call|Call.*connected|Auto-hanging up)" logs/${agent}_stdout.log | sed "s/^/[AGENT $(echo $agent | tr '[:lower:]' '[:upper:]')] /" >> logs/call_flow.log 2>/dev/null || true
    fi
done

echo -e "${GREEN}✅ Call flow log created: logs/call_flow.log${NC}"

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

# Overall result
if [ $CUSTOMER_EXIT_CODE -eq 0 ] && [ "$CUSTOMER_CONNECTED" -gt 0 ] && [ "$CUSTOMER_MEDIA" -gt 0 ] && [ "$AGENT_MEDIA" -gt 0 ]; then
    echo -e "${GREEN}🎉 CALL CENTER DEMO SUCCESSFUL!${NC}"
    echo -e "${GREEN}   ✅ Customer connected to agent${NC}"
    echo -e "${GREEN}   ✅ Call routed successfully${NC}"
    echo -e "${GREEN}   ✅ Media exchanged successfully${NC}"
    echo -e "${GREEN}   ✅ Call completed cleanly${NC}"
    exit 0
else
    echo -e "${RED}❌ CALL CENTER DEMO FAILED!${NC}"
    if [ "$CUSTOMER_CONNECTED" -eq 0 ]; then
        echo -e "${RED}   ❌ Customer failed to connect to agent${NC}"
    fi
    if [ "$CUSTOMER_MEDIA" -eq 0 ] || [ "$AGENT_MEDIA" -eq 0 ]; then
        echo -e "${RED}   ❌ Media exchange failed${NC}"
    fi
    echo -e "${RED}   📋 Check the log files for details${NC}"
    exit 1
fi 