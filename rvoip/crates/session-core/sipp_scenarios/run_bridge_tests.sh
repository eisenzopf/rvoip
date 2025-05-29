#!/bin/bash

# SIPp Bridge Test Runner for Session-Core
# This script tests the bridge infrastructure with real SIP calls and RTP media validation

set -e

# Configuration
SERVER_IP="127.0.0.1"
SERVER_PORT="5060"
CLIENT_A_IP="127.0.0.1"
CLIENT_A_PORT="5061"
CLIENT_B_IP="127.0.0.1"
CLIENT_B_PORT="5062"
SCENARIOS_DIR="$(dirname "$0")"
RESULTS_DIR="$SCENARIOS_DIR/bridge_results"
AUDIO_DIR="$SCENARIOS_DIR/audio_files"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
PURPLE='\033[0;35m'
NC='\033[0m' # No Color

# Create directories
mkdir -p "$RESULTS_DIR"
mkdir -p "$AUDIO_DIR"

printf "${BLUE}=== Session-Core Bridge Test Suite ===${NC}\n"
echo "Server: $SERVER_IP:$SERVER_PORT"
echo "Client A: $CLIENT_A_IP:$CLIENT_A_PORT"
echo "Client B: $CLIENT_B_IP:$CLIENT_B_PORT"
echo "Audio files: $AUDIO_DIR"
echo "Results: $RESULTS_DIR"
echo ""

# Function to create test audio files
create_test_audio() {
    local audio_a="$AUDIO_DIR/client_a_audio.wav"
    local audio_b="$AUDIO_DIR/client_b_audio.wav"
    
    echo "Creating test audio files for bridge testing..."
    
    # Create different frequency tones to distinguish between clients
    if command -v sox &> /dev/null; then
        # Client A: 440Hz (A4 note)
        if [ ! -f "$audio_a" ]; then
            sox -n -r 8000 -c 1 -b 16 "$audio_a" synth 30 sine 440 vol 0.5
            printf "${GREEN}✓ Created Client A audio file (440Hz)${NC}\n"
        fi
        
        # Client B: 880Hz (A5 note)
        if [ ! -f "$audio_b" ]; then
            sox -n -r 8000 -c 1 -b 16 "$audio_b" synth 30 sine 880 vol 0.5
            printf "${GREEN}✓ Created Client B audio file (880Hz)${NC}\n"
        fi
    elif command -v ffmpeg &> /dev/null; then
        # Client A: 440Hz
        if [ ! -f "$audio_a" ]; then
            ffmpeg -f lavfi -i "sine=frequency=440:duration=30:sample_rate=8000" \
                   -ac 1 -ar 8000 -sample_fmt s16 "$audio_a" -y -loglevel quiet
            printf "${GREEN}✓ Created Client A audio file (440Hz) with ffmpeg${NC}\n"
        fi
        
        # Client B: 880Hz  
        if [ ! -f "$audio_b" ]; then
            ffmpeg -f lavfi -i "sine=frequency=880:duration=30:sample_rate=8000" \
                   -ac 1 -ar 8000 -sample_fmt s16 "$audio_b" -y -loglevel quiet
            printf "${GREEN}✓ Created Client B audio file (880Hz) with ffmpeg${NC}\n"
        fi
    else
        printf "${YELLOW}⚠ Warning: No audio generation tool found (sox/ffmpeg)${NC}\n"
        echo "Creating empty placeholder files"
        touch "$audio_a" "$audio_b"
    fi
}

# Function to start bridge server
start_bridge_server() {
    local server_log="$RESULTS_DIR/bridge_server.log"
    
    printf "${YELLOW}Starting Bridge Server...${NC}\n"
    
    # Kill any existing server
    pkill -f "bridge_server" 2>/dev/null || true
    sleep 1
    
    # Start the bridge server in background
    cd "$(dirname "$SCENARIOS_DIR")"
    cargo run --example bridge_server > "$server_log" 2>&1 &
    local server_pid=$!
    
    # Wait for server to start
    echo "Waiting for bridge server to initialize..."
    local attempts=0
    while [ $attempts -lt 30 ]; do
        if check_server_running; then
            printf "${GREEN}✅ Bridge server started (PID: $server_pid)${NC}\n"
            echo "$server_pid" > "$RESULTS_DIR/server.pid"
            return 0
        fi
        sleep 1
        ((attempts++))
    done
    
    printf "${RED}❌ Bridge server failed to start${NC}\n"
    echo "Server log:"
    cat "$server_log"
    return 1
}

# Function to stop bridge server
stop_bridge_server() {
    local pid_file="$RESULTS_DIR/server.pid"
    
    if [ -f "$pid_file" ]; then
        local server_pid=$(cat "$pid_file")
        printf "${YELLOW}Stopping bridge server (PID: $server_pid)...${NC}\n"
        kill "$server_pid" 2>/dev/null || true
        rm -f "$pid_file"
        sleep 2
    fi
    
    # Kill any remaining bridge servers
    pkill -f "bridge_server" 2>/dev/null || true
}

# Function to check if server is running
check_server_running() {
    if command -v lsof &> /dev/null; then
        lsof -i :$SERVER_PORT | grep -q "LISTEN\|UDP" 2>/dev/null
    else
        # Fallback: try to connect
        python3 -c "
import socket
try:
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.settimeout(1)
    s.sendto(b'', ('$SERVER_IP', $SERVER_PORT))
    s.close()
except:
    exit(1)
" 2>/dev/null
    fi
}

# Function to run bridged call test
run_bridge_test() {
    local test_name="$1"
    local duration="${2:-20}"
    
    printf "${PURPLE}=== Running Bridge Test: $test_name ===${NC}\n"
    echo "Duration: ${duration}s"
    
    local log_a="$RESULTS_DIR/${test_name}_client_a.log"
    local log_b="$RESULTS_DIR/${test_name}_client_b.log"
    local csv_a="$RESULTS_DIR/${test_name}_client_a.csv"
    local csv_b="$RESULTS_DIR/${test_name}_client_b.csv"
    local rtp_dump="$RESULTS_DIR/${test_name}_rtp.pcap"
    
    # Start RTP packet capture
    local tcpdump_pid=""
    if command -v tcpdump &> /dev/null; then
        echo "Starting RTP packet capture..."
        sudo tcpdump -i lo0 -w "$rtp_dump" 'udp and portrange 10000-20000' &
        tcpdump_pid=$!
        sleep 1  # Give tcpdump time to start
    fi
    
    # Prepare SIPp commands
    local audio_a="$AUDIO_DIR/client_a_audio.wav"
    local audio_b="$AUDIO_DIR/client_b_audio.wav"
    
    local sipp_a_cmd="sipp -i $CLIENT_A_IP -p $CLIENT_A_PORT \
                           -m 1 -r 1 -d ${duration}000 \
                           -trace_msg -message_file $log_a -stf $csv_a"
    
    local sipp_b_cmd="sipp -i $CLIENT_B_IP -p $CLIENT_B_PORT \
                           -m 1 -r 1 -d ${duration}000 \
                           -trace_msg -message_file $log_b -stf $csv_b"
    
    # Add audio if available
    if [ -f "$audio_a" ] && [ -s "$audio_a" ]; then
        sipp_a_cmd="$sipp_a_cmd -rtp_echo -ap $audio_a"
    else
        sipp_a_cmd="$sipp_a_cmd -rtp_echo"
    fi
    
    if [ -f "$audio_b" ] && [ -s "$audio_b" ]; then
        sipp_b_cmd="$sipp_b_cmd -rtp_echo -ap $audio_b"
    else
        sipp_b_cmd="$sipp_b_cmd -rtp_echo"
    fi
    
    printf "${YELLOW}Starting Client A...${NC}\n"
    $sipp_a_cmd $SERVER_IP:$SERVER_PORT &
    local client_a_pid=$!
    
    # Wait a moment before starting client B
    sleep 2
    
    printf "${YELLOW}Starting Client B...${NC}\n"
    $sipp_b_cmd $SERVER_IP:$SERVER_PORT &
    local client_b_pid=$!
    
    # Wait for both clients to complete
    local client_a_result=0
    local client_b_result=0
    
    wait $client_a_pid || client_a_result=$?
    wait $client_b_pid || client_b_result=$?
    
    # Stop packet capture
    if [ ! -z "$tcpdump_pid" ]; then
        sleep 1
        sudo kill $tcpdump_pid 2>/dev/null || true
        echo "RTP packets captured to: $rtp_dump"
    fi
    
    # Analyze results
    if [ $client_a_result -eq 0 ] && [ $client_b_result -eq 0 ]; then
        printf "${GREEN}✅ PASSED: $test_name (Both clients successful)${NC}\n"
        analyze_bridge_flow "$rtp_dump" "$test_name"
        return 0
    else
        printf "${RED}❌ FAILED: $test_name${NC}\n"
        echo "Client A result: $client_a_result"
        echo "Client B result: $client_b_result"
        return 1
    fi
}

# Function to analyze RTP flow for bridge validation
analyze_bridge_flow() {
    local pcap_file="$1"
    local test_name="$2"
    
    if [ ! -f "$pcap_file" ]; then
        printf "${YELLOW}⚠ No packet capture file found${NC}\n"
        return
    fi
    
    printf "${BLUE}--- Bridge RTP Flow Analysis for $test_name ---${NC}\n"
    
    if command -v tcpdump &> /dev/null; then
        local packet_count=$(tcpdump -r "$pcap_file" 2>/dev/null | wc -l)
        echo "Total RTP packets captured: $packet_count"
        
        if [ "$packet_count" -gt 0 ]; then
            printf "${GREEN}✅ RTP media flow detected in bridge${NC}\n"
            
            # Analyze port usage to understand bridge flow
            echo "RTP port analysis:"
            tcpdump -r "$pcap_file" -n 2>/dev/null | \
                awk '{print $3 " -> " $5}' | \
                sort | uniq -c | sort -nr | head -10
            
            # More detailed analysis if tshark is available
            if command -v tshark &> /dev/null; then
                echo ""
                echo "Detailed bridge RTP analysis:"
                
                # Count unique UDP flows (should show bridged audio paths)
                local unique_flows=$(tshark -r "$pcap_file" -T fields -e ip.src -e ip.dst -e udp.srcport -e udp.dstport 2>/dev/null | sort -u | wc -l)
                echo "Unique UDP flows: $unique_flows (expecting 4+ for bridge: A→Server, Server→B, B→Server, Server→A)"
                
                # Show sample packet flow
                echo "Sample bridge packet flow:"
                tshark -r "$pcap_file" -T fields -e frame.time_relative -e ip.src -e ip.dst -e udp.srcport -e udp.dstport 2>/dev/null | head -10 | while read time src dst sport dport; do
                    echo "  ${time}s: $src:$sport → $dst:$dport"
                done
                
                # Check for bidirectional flow (evidence of successful bridge)
                local src_ports=$(tshark -r "$pcap_file" -T fields -e udp.srcport 2>/dev/null | sort -u | wc -l)
                local dst_ports=$(tshark -r "$pcap_file" -T fields -e udp.dstport 2>/dev/null | sort -u | wc -l)
                
                echo "Source ports: $src_ports, Destination ports: $dst_ports"
                
                if [ "$src_ports" -ge 3 ] && [ "$dst_ports" -ge 3 ]; then
                    printf "${GREEN}✅ Bidirectional bridge flow detected${NC}\n"
                else
                    printf "${YELLOW}⚠ Limited flow patterns - bridge may not be fully active${NC}\n"
                fi
            fi
        else
            printf "${RED}❌ No RTP packets captured - bridge may not be working${NC}\n"
        fi
    fi
}

# Function to run server log analysis
analyze_server_logs() {
    local server_log="$RESULTS_DIR/bridge_server.log"
    
    if [ -f "$server_log" ]; then
        printf "${BLUE}--- Server Bridge Activity Analysis ---${NC}\n"
        
        echo "Bridge events in server log:"
        grep -E "(📞|🌉|✅.*bridge|🛑)" "$server_log" | tail -20
        
        # Count key events
        local incoming_calls=$(grep -c "📞 New incoming call" "$server_log" 2>/dev/null || echo "0")
        local bridges_created=$(grep -c "✅ Created bridge" "$server_log" 2>/dev/null || echo "0")
        local bridges_destroyed=$(grep -c "✅ Bridge.*destroyed" "$server_log" 2>/dev/null || echo "0")
        
        echo ""
        echo "Bridge Statistics:"
        echo "  Incoming calls: $incoming_calls"
        echo "  Bridges created: $bridges_created"
        echo "  Bridges destroyed: $bridges_destroyed"
        
        if [ "$bridges_created" -gt 0 ]; then
            printf "${GREEN}✅ Bridge creation detected in server logs${NC}\n"
        else
            printf "${YELLOW}⚠ No bridge creation found in server logs${NC}\n"
        fi
    fi
}

# Function to check prerequisites
check_prerequisites() {
    echo "Checking bridge test prerequisites..."
    
    # Check SIPp
    if ! command -v sipp &> /dev/null; then
        printf "${RED}Error: SIPp is not installed${NC}\n"
        echo "Install SIPp: https://github.com/SIPp/sipp"
        exit 1
    fi
    printf "${GREEN}✓ SIPp found${NC}\n"
    
    # Check Rust/Cargo
    if ! command -v cargo &> /dev/null; then
        printf "${RED}Error: Cargo not found${NC}\n"
        exit 1
    fi
    printf "${GREEN}✓ Cargo found${NC}\n"
    
    # Check tcpdump (optional but recommended)
    if command -v tcpdump &> /dev/null; then
        printf "${GREEN}✓ tcpdump found (RTP capture enabled)${NC}\n"
    else
        printf "${YELLOW}⚠ tcpdump not found (no RTP capture)${NC}\n"
    fi
    
    # Check audio tools (optional)
    if command -v sox &> /dev/null; then
        printf "${GREEN}✓ sox found (audio generation enabled)${NC}\n"
    elif command -v ffmpeg &> /dev/null; then
        printf "${GREEN}✓ ffmpeg found (audio generation enabled)${NC}\n"
    else
        printf "${YELLOW}⚠ No audio tools found (limited audio testing)${NC}\n"
    fi
}

# Cleanup function
cleanup() {
    echo ""
    printf "${YELLOW}Cleaning up...${NC}\n"
    stop_bridge_server
    
    # Kill any remaining SIPp processes
    pkill -f "sipp.*$SERVER_IP" 2>/dev/null || true
    
    # Kill any remaining tcpdump
    sudo pkill tcpdump 2>/dev/null || true
}

# Set up signal handlers
trap cleanup EXIT INT TERM

# Main execution
main() {
    local failed_tests=0
    local total_tests=0
    
    check_prerequisites
    create_test_audio
    
    printf "\n${BLUE}=== Starting Bridge Infrastructure Tests ===${NC}\n\n"
    
    # Start the bridge server
    if ! start_bridge_server; then
        echo "Failed to start bridge server"
        exit 1
    fi
    
    sleep 2  # Allow server to fully initialize
    
    # Test 1: Basic bridge test (20 seconds)
    printf "\n${PURPLE}Test 1: Basic Bridge Test${NC}\n"
    if run_bridge_test "basic_bridge" 20; then
        ((total_tests++))
    else
        ((failed_tests++))
        ((total_tests++))
    fi
    
    # Brief pause between tests
    sleep 3
    
    # Test 2: Quick bridge test (10 seconds)
    printf "\n${PURPLE}Test 2: Quick Bridge Test${NC}\n"
    if run_bridge_test "quick_bridge" 10; then
        ((total_tests++))
    else
        ((failed_tests++))
        ((total_tests++))
    fi
    
    # Analyze server activity
    analyze_server_logs
    
    # Summary
    printf "\n${BLUE}=== Bridge Test Summary ===${NC}\n"
    echo "Total tests: $total_tests"
    echo "Passed: $((total_tests - failed_tests))"
    echo "Failed: $failed_tests"
    echo "Results saved to: $RESULTS_DIR"
    
    if [ $failed_tests -eq 0 ]; then
        printf "${GREEN}🎉 All bridge tests passed!${NC}\n"
        printf "${GREEN}✅ Bridge infrastructure is working correctly${NC}\n"
        exit 0
    else
        printf "${RED}❌ $failed_tests test(s) failed${NC}\n"
        printf "${RED}❌ Bridge infrastructure needs attention${NC}\n"
        exit 1
    fi
}

# Handle command line arguments
case "${1:-all}" in
    "setup")
        echo "Setting up bridge test environment..."
        check_prerequisites
        create_test_audio
        printf "${GREEN}✓ Bridge test setup complete${NC}\n"
        ;;
    "server")
        echo "Starting bridge server only..."
        check_prerequisites
        start_bridge_server
        echo "Bridge server running. Press Ctrl+C to stop."
        read -r
        ;;
    "quick")
        echo "Running quick bridge test only..."
        check_prerequisites
        create_test_audio
        start_bridge_server
        sleep 2
        run_bridge_test "quick_bridge_only" 10
        ;;
    "all"|*)
        main
        ;;
esac 