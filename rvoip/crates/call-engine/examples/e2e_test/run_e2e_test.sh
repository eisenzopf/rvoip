#!/bin/bash
#
# End-to-End Call Center Test Script
# This script:
# 1. Starts the call center server
# 2. Starts two agent clients (alice and bob)
# 3. Runs SIPp to make test calls
# 4. Captures packets and logs
# 5. Analyzes results

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
    sudo tcpdump -i lo -w "$PCAP_FILE" \
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
        --call-duration 15 \
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
        --call-duration 15 \
        > "$BOB_LOG" 2>&1 &
    BOB_PID=$!
    
    # Wait for Bob to register
    wait_for_ready "$BOB_LOG" "Successfully registered" 10 "Bob" || exit 1
    
    # Give agents time to fully register
    sleep 2
    
    # Check agent status in server log
    echo -e "\n${YELLOW}Checking agent registration status...${NC}"
    if grep -q "Updated agent.*alice.*status to available" "$SERVER_LOG"; then
        echo -e "Alice: ${GREEN}Registered${NC}"
    else
        echo -e "Alice: ${RED}Not registered${NC}"
    fi
    
    if grep -q "Updated agent.*bob.*status to available" "$SERVER_LOG"; then
        echo -e "Bob: ${GREEN}Registered${NC}"
    else
        echo -e "Bob: ${RED}Not registered${NC}"
    fi
    
    # Generate test audio PCAP if not exists
    if [ ! -f "$SIPP_DIR/pcap/g711a.pcap" ]; then
        echo -e "\n${YELLOW}Creating test audio PCAP...${NC}"
        mkdir -p "$SIPP_DIR/pcap"
        # Create a simple script to generate test tone
        cat > "$SIPP_DIR/pcap/generate_audio.py" << 'EOF'
#!/usr/bin/env python3
# This would generate a G.711 audio PCAP file
# For now, we'll skip this as it requires complex audio generation
print("Audio PCAP generation placeholder")
EOF
        echo -e "${YELLOW}Note: Using SIPp without audio PCAP for now${NC}"
    fi
    
    # Run SIPp test calls
    echo -e "\n${YELLOW}Running SIPp test calls...${NC}"
    echo "Making 5 calls, 1 call per second..."
    
    cd "$SIPP_DIR"
    sipp -sf customer_uac.xml \
        -s support \
        -i 127.0.0.1 \
        -p 5080 \
        -m 5 \
        -r 1 \
        -l 2 \
        -trace_msg \
        -trace_err \
        -trace_screen \
        -trace_stat \
        127.0.0.1:5060 \
        > "$SIPP_LOG" 2>&1 || true
    
    # Wait for calls to complete
    echo "Waiting for calls to complete..."
    sleep 20
    
    # Stop packet capture
    echo -e "\n${YELLOW}Stopping packet capture...${NC}"
    sudo kill -TERM $TCPDUMP_PID 2>/dev/null || true
    TCPDUMP_PID=""
    sleep 1
    
    # Analyze results
    echo -e "\n${GREEN}================================${NC}"
    echo -e "${GREEN}Test Results${NC}"
    echo -e "${GREEN}================================${NC}\n"
    
    # Check server log for successful calls
    echo "Analyzing server log..."
    CALLS_RECEIVED=$(grep -c "Received incoming call" "$SERVER_LOG" || true)
    CALLS_ESTABLISHED=$(grep -c "Call .* established" "$SERVER_LOG" || true)
    CALLS_ENDED=$(grep -c "Call .* ended" "$SERVER_LOG" || true)
    
    echo "Calls received: $CALLS_RECEIVED"
    echo "Calls established: $CALLS_ESTABLISHED"
    echo "Calls ended: $CALLS_ENDED"
    
    # Check agent logs
    echo -e "\nAnalyzing agent logs..."
    ALICE_CALLS=$(grep -c "Incoming call" "$ALICE_LOG" || true)
    BOB_CALLS=$(grep -c "Incoming call" "$BOB_LOG" || true)
    
    echo "Calls handled by Alice: $ALICE_CALLS"
    echo "Calls handled by Bob: $BOB_CALLS"
    
    # Check SIPp results
    echo -e "\nAnalyzing SIPp results..."
    if [ -f "${SIPP_DIR}/customer_uac_*.csv" ]; then
        SIPP_SUCCESS=$(tail -1 ${SIPP_DIR}/customer_uac_*.csv | cut -d';' -f5)
        SIPP_FAILED=$(tail -1 ${SIPP_DIR}/customer_uac_*.csv | cut -d';' -f6)
        echo "SIPp successful calls: $SIPP_SUCCESS"
        echo "SIPp failed calls: $SIPP_FAILED"
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
    
    # Determine overall success
    if [ "$CALLS_ESTABLISHED" -gt 0 ] && [ "$((ALICE_CALLS + BOB_CALLS))" -gt 0 ]; then
        echo -e "${GREEN}✅ E2E Test PASSED!${NC}"
        echo "Successfully routed calls from customers to agents"
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