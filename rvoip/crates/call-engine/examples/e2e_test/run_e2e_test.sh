#!/bin/bash
#
# End-to-End Call Center Test Script (PHASE 0.24+)
# This script:
# 1. Starts the call center server
# 2. Starts two agent clients (alice and bob)
# 3. Runs SIPp to make test calls
# 4. Captures packets and logs
# 5. Analyzes results (including server-initiated BYE completion)

set -e  # Exit on error

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOG_DIR="$SCRIPT_DIR/logs"
PCAP_DIR="$SCRIPT_DIR/pcaps"
SIPP_DIR="$SCRIPT_DIR/sipp_scenarios"
SERVER_LOG="$LOG_DIR/server.log"
ALICE_LOG="$LOG_DIR/alice.log"
BOB_LOG="$LOG_DIR/bob.log"
SIPP_LOG="$LOG_DIR/sipp.log"
PCAP_FILE="$PCAP_DIR/test_capture.pcap"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Cleanup function
cleanup() {
    echo -e "\n${YELLOW}Cleaning up...${NC}"
    
    # Kill all background processes
    if [[ -n "$SERVER_PID" ]]; then
        echo "Stopping server (PID: $SERVER_PID)..."
        kill -TERM $SERVER_PID 2>/dev/null || true
    fi
    
    if [[ -n "$ALICE_PID" ]]; then
        echo "Stopping Alice agent (PID: $ALICE_PID)..."
        kill -TERM $ALICE_PID 2>/dev/null || true
    fi
    
    if [[ -n "$BOB_PID" ]]; then
        echo "Stopping Bob agent (PID: $BOB_PID)..."
        kill -TERM $BOB_PID 2>/dev/null || true
    fi
    
    if [[ -n "$TCPDUMP_PID" ]]; then
        echo "Stopping tcpdump (PID: $TCPDUMP_PID)..."
        sudo kill -TERM $TCPDUMP_PID 2>/dev/null || true
    fi
    
    # Wait a bit for processes to die
    sleep 2
    
    echo -e "${GREEN}Cleanup complete${NC}"
}

# Set trap to cleanup on exit
trap cleanup EXIT

# Function to check if a command exists
check_command() {
    if ! command -v $1 &> /dev/null; then
        echo -e "${RED}Error: $1 is not installed${NC}"
        echo "Please install $1 to continue"
        exit 1
    fi
}

# Function to wait for a process to be ready
wait_for_ready() {
    local log_file=$1
    local ready_string=$2
    local timeout=$3
    local name=$4
    
    echo -n "Waiting for $name to be ready..."
    
    local count=0
    while [ $count -lt $timeout ]; do
        if grep -q "$ready_string" "$log_file" 2>/dev/null; then
            echo -e " ${GREEN}Ready!${NC}"
            return 0
        fi
        sleep 1
        count=$((count + 1))
        echo -n "."
    done
    
    echo -e " ${RED}Timeout!${NC}"
    echo "Failed to find '$ready_string' in $log_file"
    return 1
}

# Main test execution
main() {
    echo -e "${GREEN}================================${NC}"
    echo -e "${GREEN}Call Center E2E Test Runner${NC}"
    echo -e "${GREEN}================================${NC}\n"
    
    # Detect OS and set loopback interface name
    if [[ "$OSTYPE" == "darwin"* ]]; then
        LOOPBACK_IF="lo0"
        echo "Detected macOS, using loopback interface: $LOOPBACK_IF"
    else
        LOOPBACK_IF="lo"
        echo "Using loopback interface: $LOOPBACK_IF"
    fi
    
    # Check prerequisites
    echo "Checking prerequisites..."
    check_command cargo
    check_command tcpdump
    check_command sipp
    
    # Create directories
    echo "Creating directories..."
    mkdir -p "$LOG_DIR" "$PCAP_DIR"
    
    # Clean old logs
    rm -f "$LOG_DIR"/*.log
    
    # Build the project
    echo -e "\n${YELLOW}Building project...${NC}"
    cd "$SCRIPT_DIR/../.."
    cargo build --examples --release
    
    # Start packet capture
    echo -e "\n${YELLOW}Starting packet capture...${NC}"
    sudo tcpdump -i $LOOPBACK_IF -w "$PCAP_FILE" \
        'port 5060 or portrange 5070-5090 or portrange 10000-20000' \
        -s 0 &
    TCPDUMP_PID=$!
    sleep 1
    
    # Start the call center server
    echo -e "\n${YELLOW}Starting call center server...${NC}"
    RUST_LOG=info cargo run --example e2e_test_server --release > "$SERVER_LOG" 2>&1 &
    SERVER_PID=$!
    
    # Wait for server to be ready
    wait_for_ready "$SERVER_LOG" "CALL CENTER IS READY" 30 "server" || exit 1
    
    # Give server a moment to fully initialize
    sleep 2
    
    # Start Alice agent
    echo -e "\n${YELLOW}Starting Alice agent...${NC}"
    RUST_LOG=info cargo run --example e2e_test_agent --release -- \
        --username alice \
        --server 127.0.0.1:5060 \
        --port 5071 \
        --call-duration 5 \
        > "$ALICE_LOG" 2>&1 &
    ALICE_PID=$!
    
    # Wait for Alice to register
    wait_for_ready "$ALICE_LOG" "Successfully registered" 10 "Alice" || exit 1
    
    # Start Bob agent
    echo -e "\n${YELLOW}Starting Bob agent...${NC}"
    RUST_LOG=info cargo run --example e2e_test_agent --release -- \
        --username bob \
        --server 127.0.0.1:5060 \
        --port 5072 \
        --call-duration 5 \
        > "$BOB_LOG" 2>&1 &
    BOB_PID=$!
    
    # Wait for Bob to register
    wait_for_ready "$BOB_LOG" "Successfully registered" 10 "Bob" || exit 1
    
    # Give agents time to fully register
    sleep 2
    
    # Check agent status in server log
    echo -e "\n${YELLOW}Checking agent registration status...${NC}"
    if grep -q "Agent alice registered in database" "$SERVER_LOG"; then
        echo -e "Alice: ${GREEN}Registered${NC}"
    else
        echo -e "Alice: ${RED}Not registered${NC}"
    fi
    
    if grep -q "Agent bob registered in database" "$SERVER_LOG"; then
        echo -e "Bob: ${GREEN}Registered${NC}"
    else
        echo -e "Bob: ${RED}Not registered${NC}"
    fi
    
    # Generate test audio PCAP if not exists
    if [ ! -f "$PCAP_DIR/g711a.pcap" ]; then
        echo -e "\n${YELLOW}Creating test audio PCAP...${NC}"
        echo -e "${RED}Warning: No g711a.pcap found in $PCAP_DIR${NC}"
        echo -e "${YELLOW}Running test without audio (signaling only)${NC}"
    else
        echo -e "\n${GREEN}Found audio PCAP file: $PCAP_DIR/g711a.pcap${NC}"
        echo -e "${GREEN}Test will include G.711 audio${NC}"
    fi
    
    # PHASE 0.24: Enhanced SIPp test calls with BYE tracking
    # NOTE: This test uses SERVER-INITIATED BYE scenario:
    # 1. SIPp clients make calls to the server
    # 2. Server routes calls to agents (Alice/Bob)
    # 3. After call duration timeout (~20s), SERVER sends BYE to SIPp clients
    # 4. SIPp clients respond with 200 OK to terminate calls
    # This is different from client-initiated BYE where clients would send BYE first
    echo -e "\n${YELLOW}Running SIPp test calls (PHASE 0.24)...${NC}"
    echo "Making 5 calls waiting for server-initiated BYE (no duration limit)..."
    
    cd "$SIPP_DIR"
    sipp -sf customer_uac.xml \
        -s support \
        -i 127.0.0.1 \
        -p 5080 \
        -m 5 \
        -r 1 \
        -l 5 \
        -trace_msg \
        -trace_err \
        -trace_screen \
        -trace_stat \
        127.0.0.1:5060 \
        > "$SIPP_LOG" 2>&1 || true
    
    # PHASE 0.24: Wait for calls to complete with extended time for BYE processing
    echo "Waiting for calls to complete (45s - allowing time for 20s calls + BYE completion)..."
    sleep 45
    
    # Stop packet capture
    echo -e "\n${YELLOW}Stopping packet capture...${NC}"
    sudo kill -TERM $TCPDUMP_PID 2>/dev/null || true
    TCPDUMP_PID=""
    sleep 1
    
    # Analyze results
    echo -e "\n${GREEN}================================${NC}"
    echo -e "${GREEN}Test Results${NC}"
    echo -e "${GREEN}================================${NC}\n"
    
    # PHASE 0.24: Enhanced server log analysis with BYE tracking
    echo "Analyzing server log..."
    CALLS_RECEIVED=$(grep -c "Received incoming call" "$SERVER_LOG" || true)
    CALLS_ESTABLISHED=$(grep -c "Call .* established" "$SERVER_LOG" || true)
    CALLS_ENDED=$(grep -c "Call .* ended" "$SERVER_LOG" || true)
    
    echo "Calls received: $CALLS_RECEIVED"
    echo "Calls established: $CALLS_ESTABLISHED"
    echo "Calls ended: $CALLS_ENDED"
    
    # PHASE 0.24: Server-side BYE analysis
    echo -e "\nServer BYE handling analysis:"
    BYE_SEND_ATTEMPTS=$(grep -c "BYE-SEND: Attempting to send BYE" "$SERVER_LOG" || true)
    BYE_SEND_SUCCESS=$(grep -c "BYE-SEND: Successfully sent BYE" "$SERVER_LOG" || true)
    BYE_200OK_RECEIVED=$(grep -c "BYE-200OK: Received 200 OK" "$SERVER_LOG" || true)
    BYE_TIMEOUTS=$(grep -c "BYE timeout.*forcing dialog termination" "$SERVER_LOG" || true)
    BYE_FORWARD=$(grep -c "BYE-FORWARD: Successfully terminated" "$SERVER_LOG" || true)
    
    echo "BYE send attempts: $BYE_SEND_ATTEMPTS"
    echo "BYE send successful: $BYE_SEND_SUCCESS"
    echo "BYE 200 OK received: $BYE_200OK_RECEIVED"
    echo "BYE timeouts: $BYE_TIMEOUTS"
    echo "BYE forwarding successful: $BYE_FORWARD"
    
    if [ "$BYE_200OK_RECEIVED" -gt 0 ]; then
        echo -e "${GREEN}✅ Server BYE handling working - $BYE_200OK_RECEIVED successful terminations${NC}"
    else
        echo -e "${YELLOW}⚠️ Server BYE handling - no 200 OK confirmations logged${NC}"
    fi
    
    # Check agent logs
    echo -e "\nAnalyzing agent logs..."
    ALICE_CALLS=$(grep -c "Incoming call" "$ALICE_LOG" || true)
    BOB_CALLS=$(grep -c "Incoming call" "$BOB_LOG" || true)
    
    echo "Calls handled by Alice: $ALICE_CALLS"
    echo "Calls handled by Bob: $BOB_CALLS"
    
    # PHASE 0.24: Enhanced SIPp results analysis with BYE tracking
    echo -e "\nAnalyzing SIPp results..."
    if [ -f "${SIPP_DIR}/customer_uac_*.csv" ]; then
        SIPP_SUCCESS=$(tail -1 ${SIPP_DIR}/customer_uac_*.csv | cut -d';' -f5)
        SIPP_FAILED=$(tail -1 ${SIPP_DIR}/customer_uac_*.csv | cut -d';' -f6)
        echo "SIPp successful calls: $SIPP_SUCCESS"
        echo "SIPp failed calls: $SIPP_FAILED"
    fi
    
    # PHASE 0.24: BYE completion analysis (Server-Initiated BYE Scenario)
    echo -e "\nBYE completion analysis (Server-Initiated):"
    
    # In server-initiated BYE scenario:
    # - Server sends BYE to SIPp client
    # - SIPp client responds with 200 OK
    # - SIPp receives BYE (shown in statistics screen)
    # - SIPp sends 200 OK responses (shown in statistics screen)
    
    # Check SIPp statistics for BYE messages received
    BYE_RECEIVED_SIPP=$(grep -A 20 "Messages.*Retrans.*Timeout" "$SIPP_LOG" | grep "BYE <----------" | awk '{print $5}' || echo "0")
    BYE_200OK_SENT_SIPP=$(grep -A 20 "Messages.*Retrans.*Timeout" "$SIPP_LOG" | grep "200 ---------->" | awk '{print $5}' || echo "0")
    
    # Fallback: Check PCAP file for BYE/200 OK exchanges if available
    if [ -f "$PCAP_FILE" ]; then
        BYE_IN_PCAP=$(sudo tcpdump -r "$PCAP_FILE" -A 2>/dev/null | grep -c "BYE sip:" || true)
        BYE_200OK_IN_PCAP=$(sudo tcpdump -r "$PCAP_FILE" -A 2>/dev/null | grep -A 5 "200 OK" | grep -c "CSeq:.*BYE" || true)
        echo "PCAP BYE messages: $BYE_IN_PCAP"
        echo "PCAP BYE 200 OK responses: $BYE_200OK_IN_PCAP"
    else
        BYE_IN_PCAP=0
        BYE_200OK_IN_PCAP=0
    fi
    
    # Use PCAP data as primary source, SIPp stats as fallback
    if [ "$BYE_IN_PCAP" -gt 0 ] && [ "$BYE_200OK_IN_PCAP" -gt 0 ]; then
        BYE_RECEIVED=$BYE_IN_PCAP
        BYE_200OK_RESPONSES=$BYE_200OK_IN_PCAP
        echo "BYE messages received by SIPp: $BYE_RECEIVED (from PCAP)"
        echo "200 OK responses sent by SIPp: $BYE_200OK_RESPONSES (from PCAP)"
    else
        BYE_RECEIVED=$BYE_RECEIVED_SIPP
        BYE_200OK_RESPONSES=$BYE_200OK_SENT_SIPP
        echo "BYE messages received by SIPp: $BYE_RECEIVED (from SIPp stats)"
        echo "200 OK responses sent by SIPp: $BYE_200OK_RESPONSES (from SIPp stats)"
    fi
    
    # Evaluate BYE completion success
    if [ "$BYE_RECEIVED" -gt 0 ] && [ "$BYE_200OK_RESPONSES" -gt 0 ]; then
        echo -e "${GREEN}✅ BYE completion working - $BYE_200OK_RESPONSES server-initiated terminations completed${NC}"
        BYE_COMPLETION_SUCCESS=true
    elif [ "$BYE_200OK_RECEIVED" -gt 0 ]; then
        # Fallback to server-side analysis
        echo -e "${GREEN}✅ BYE completion working - $BYE_200OK_RECEIVED server-side confirmations${NC}"
        BYE_COMPLETION_SUCCESS=true
    else
        echo -e "${RED}❌ BYE completion issue - no successful termination confirmations found${NC}"
        BYE_COMPLETION_SUCCESS=false
    fi
    
    # PCAP analysis
    echo -e "\nPCAP file analysis..."
    if [ -f "$PCAP_FILE" ]; then
        PCAP_SIZE=$(du -h "$PCAP_FILE" | cut -f1)
        echo "PCAP file size: $PCAP_SIZE"
        echo "PCAP file: $PCAP_FILE"
        
        # Quick packet count
        PACKET_COUNT=$(sudo tcpdump -r "$PCAP_FILE" 2>/dev/null | wc -l)
        echo "Total packets captured: $PACKET_COUNT"
        
        # SIP message count
        SIP_INVITES=$(sudo tcpdump -r "$PCAP_FILE" -A 2>/dev/null | grep -c "INVITE sip:" || true)
        SIP_REGISTERS=$(sudo tcpdump -r "$PCAP_FILE" -A 2>/dev/null | grep -c "REGISTER sip:" || true)
        echo "SIP INVITEs: $SIP_INVITES"
        echo "SIP REGISTERs: $SIP_REGISTERS"
    fi
    
    # Summary
    echo -e "\n${GREEN}================================${NC}"
    echo -e "${GREEN}Test Summary${NC}"
    echo -e "${GREEN}================================${NC}\n"
    
    # PHASE 0.24: Enhanced success criteria including BYE completion
    if [ "$CALLS_ESTABLISHED" -gt 0 ] && [ "$((ALICE_CALLS + BOB_CALLS))" -gt 0 ]; then
        echo -e "${GREEN}✅ E2E Test PASSED!${NC}"
        echo "Successfully routed calls from customers to agents"
        
        # PHASE 0.24: BYE completion validation using improved analysis
        if [ "$BYE_COMPLETION_SUCCESS" = true ]; then
            echo -e "${GREEN}✅ BYE Completion PASSED!${NC}"
            echo "Server-initiated call termination working properly"
        else
            echo -e "${YELLOW}⚠️ BYE Completion PARTIAL${NC}"
            echo "Calls completed but BYE termination may need investigation"
        fi
    else
        echo -e "${RED}❌ E2E Test FAILED!${NC}"
        echo "Failed to establish calls between customers and agents"
        exit 1
    fi
    
    echo -e "\n${YELLOW}Logs and captures saved in:${NC}"
    echo "  Server log: $SERVER_LOG"
    echo "  Alice log: $ALICE_LOG"
    echo "  Bob log: $BOB_LOG"
    echo "  SIPp log: $SIPP_LOG"
    echo "  PCAP file: $PCAP_FILE"
    
    echo -e "\n${YELLOW}To analyze the PCAP file:${NC}"
    echo "  wireshark $PCAP_FILE"
    echo "  or"
    echo "  tcpdump -r $PCAP_FILE -A | less"
}

# Run the main function
main "$@" 