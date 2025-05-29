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
CLIENT_C_IP="127.0.0.1"
CLIENT_C_PORT="5063"
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
echo "Client C: $CLIENT_C_IP:$CLIENT_C_PORT"
echo "Audio files: $AUDIO_DIR"
echo "Results: $RESULTS_DIR"
echo ""

# Function to create test audio files
create_test_audio() {
    local audio_a="$AUDIO_DIR/client_a_audio.wav"
    local audio_b="$AUDIO_DIR/client_b_audio.wav"
    local audio_c="$AUDIO_DIR/client_c_audio.wav"
    
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
        
        # Client C: 1320Hz (E6 note)
        if [ ! -f "$audio_c" ]; then
            sox -n -r 8000 -c 1 -b 16 "$audio_c" synth 30 sine 1320 vol 0.5
            printf "${GREEN}✓ Created Client C audio file (1320Hz)${NC}\n"
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
        
        # Client C: 1320Hz
        if [ ! -f "$audio_c" ]; then
            ffmpeg -f lavfi -i "sine=frequency=1320:duration=30:sample_rate=8000" \
                   -ac 1 -ar 8000 -sample_fmt s16 "$audio_c" -y -loglevel quiet
            printf "${GREEN}✓ Created Client C audio file (1320Hz) with ffmpeg${NC}\n"
        fi
    else
        printf "${YELLOW}⚠ Warning: No audio generation tool found (sox/ffmpeg)${NC}\n"
        echo "Creating empty placeholder files"
        touch "$audio_a" "$audio_b" "$audio_c"
    fi
}

# Function to start bridge server
start_bridge_server() {
    local server_log="$RESULTS_DIR/bridge_server.log"
    local server_example="${BRIDGE_SERVER:-bridge_server}"
    
    printf "${YELLOW}Starting Bridge Server ($server_example)...${NC}\n"
    
    # Kill any existing server
    pkill -f "$server_example" 2>/dev/null || true
    sleep 1
    
    # Start the bridge server in background
    cd "$(dirname "$SCENARIOS_DIR")"
    cargo run --example "$server_example" > "$server_log" 2>&1 &
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
    local server_example="${BRIDGE_SERVER:-bridge_server}"
    
    if [ -f "$pid_file" ]; then
        local server_pid=$(cat "$pid_file")
        printf "${YELLOW}Stopping bridge server (PID: $server_pid)...${NC}\n"
        kill "$server_pid" 2>/dev/null || true
        rm -f "$pid_file"
        sleep 2
    fi
    
    # Kill any remaining bridge servers
    pkill -f "$server_example" 2>/dev/null || true
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
    
    # Kill any remaining SIPp processes for all client ports
    pkill -f "sipp.*$SERVER_IP" 2>/dev/null || true
    pkill -f "sipp.*$CLIENT_A_PORT" 2>/dev/null || true
    pkill -f "sipp.*$CLIENT_B_PORT" 2>/dev/null || true
    pkill -f "sipp.*$CLIENT_C_PORT" 2>/dev/null || true
    
    # Kill any remaining tcpdump
    sudo pkill tcpdump 2>/dev/null || true
}

# Set up signal handlers
trap cleanup EXIT INT TERM

# Function to run 3-way bridged call test (for multi-session conferencing)
run_3way_bridge_test() {
    local test_name="$1"
    local duration="${2:-20}"
    
    printf "${PURPLE}=== Running 3-Way Bridge Test: $test_name ===${NC}\n"
    echo "Duration: ${duration}s"
    echo "Testing N-way conferencing with 3 participants"
    
    local log_a="$RESULTS_DIR/${test_name}_client_a.log"
    local log_b="$RESULTS_DIR/${test_name}_client_b.log"
    local log_c="$RESULTS_DIR/${test_name}_client_c.log"
    local csv_a="$RESULTS_DIR/${test_name}_client_a.csv"
    local csv_b="$RESULTS_DIR/${test_name}_client_b.csv"
    local csv_c="$RESULTS_DIR/${test_name}_client_c.csv"
    local rtp_dump="$RESULTS_DIR/${test_name}_rtp.pcap"
    
    # Start RTP packet capture
    local tcpdump_pid=""
    if command -v tcpdump &> /dev/null; then
        echo "Starting RTP packet capture for 3-way bridge..."
        sudo tcpdump -i lo0 -w "$rtp_dump" 'udp and portrange 10000-20000' &
        tcpdump_pid=$!
        sleep 1  # Give tcpdump time to start
    fi
    
    # Prepare SIPp commands for all three clients
    local audio_a="$AUDIO_DIR/client_a_audio.wav"
    local audio_b="$AUDIO_DIR/client_b_audio.wav"
    local audio_c="$AUDIO_DIR/client_c_audio.wav"
    
    local sipp_a_cmd="sipp -i $CLIENT_A_IP -p $CLIENT_A_PORT \
                           -m 1 -r 1 -d ${duration}000 \
                           -trace_msg -message_file $log_a -stf $csv_a"
    
    local sipp_b_cmd="sipp -i $CLIENT_B_IP -p $CLIENT_B_PORT \
                           -m 1 -r 1 -d ${duration}000 \
                           -trace_msg -message_file $log_b -stf $csv_b"
                           
    local sipp_c_cmd="sipp -i $CLIENT_C_IP -p $CLIENT_C_PORT \
                           -m 1 -r 1 -d ${duration}000 \
                           -trace_msg -message_file $log_c -stf $csv_c"
    
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
    
    if [ -f "$audio_c" ] && [ -s "$audio_c" ]; then
        sipp_c_cmd="$sipp_c_cmd -rtp_echo -ap $audio_c"
    else
        sipp_c_cmd="$sipp_c_cmd -rtp_echo"
    fi
    
    # Start clients with staggered timing to observe bridge building
    printf "${YELLOW}Starting Client A (440Hz)...${NC}\n"
    $sipp_a_cmd $SERVER_IP:$SERVER_PORT &
    local client_a_pid=$!
    
    # Wait before starting client B to see bridge pairing
    sleep 3
    
    printf "${YELLOW}Starting Client B (880Hz)...${NC}\n"
    $sipp_b_cmd $SERVER_IP:$SERVER_PORT &
    local client_b_pid=$!
    
    # Wait before starting client C to see 3-way bridge formation
    sleep 3
    
    printf "${YELLOW}Starting Client C (1320Hz) - Creating 3-way conference...${NC}\n"
    $sipp_c_cmd $SERVER_IP:$SERVER_PORT &
    local client_c_pid=$!
    
    # Wait for all clients to complete
    local client_a_result=0
    local client_b_result=0
    local client_c_result=0
    
    wait $client_a_pid || client_a_result=$?
    wait $client_b_pid || client_b_result=$?
    wait $client_c_pid || client_c_result=$?
    
    # Stop packet capture
    if [ ! -z "$tcpdump_pid" ]; then
        sleep 1
        sudo kill $tcpdump_pid 2>/dev/null || true
        echo "3-way bridge RTP packets captured to: $rtp_dump"
    fi
    
    # Analyze results
    if [ $client_a_result -eq 0 ] && [ $client_b_result -eq 0 ] && [ $client_c_result -eq 0 ]; then
        printf "${GREEN}✅ PASSED: $test_name (All 3 clients successful)${NC}\n"
        analyze_3way_bridge_flow "$rtp_dump" "$test_name"
        return 0
    else
        printf "${RED}❌ FAILED: $test_name${NC}\n"
        echo "Client A result: $client_a_result"
        echo "Client B result: $client_b_result"
        echo "Client C result: $client_c_result"
        return 1
    fi
}

# Function to analyze 3-way bridge RTP flow patterns
analyze_3way_bridge_flow() {
    local pcap_file="$1"
    local test_name="$2"
    
    if [ ! -f "$pcap_file" ]; then
        printf "${YELLOW}⚠ No packet capture file found${NC}\n"
        return
    fi
    
    printf "${BLUE}--- 3-Way Bridge RTP Flow Analysis for $test_name ---${NC}\n"
    
    if command -v tcpdump &> /dev/null; then
        local packet_count=$(tcpdump -r "$pcap_file" 2>/dev/null | wc -l)
        echo "Total RTP packets captured: $packet_count"
        
        if [ "$packet_count" -gt 0 ]; then
            printf "${GREEN}✅ RTP media flow detected in 3-way bridge${NC}\n"
            
            # Analyze port usage to understand 3-way bridge flow
            echo ""
            echo "3-way bridge RTP port analysis:"
            tcpdump -r "$pcap_file" -n 2>/dev/null | \
                awk '{print $3 " -> " $5}' | \
                sort | uniq -c | sort -nr | head -15
            
            # More detailed analysis if tshark is available
            if command -v tshark &> /dev/null; then
                echo ""
                echo "Detailed 3-way bridge RTP analysis:"
                
                # Count unique UDP flows (should show full-mesh bridged audio paths)
                local unique_flows=$(tshark -r "$pcap_file" -T fields -e ip.src -e ip.dst -e udp.srcport -e udp.dstport 2>/dev/null | sort -u | wc -l)
                echo "Unique UDP flows: $unique_flows"
                echo "Expected for 3-way bridge: 12+ flows (A↔Server, B↔Server, C↔Server in both directions)"
                
                # Show sample packet flow timeline
                echo ""
                echo "Sample 3-way bridge packet flow timeline:"
                tshark -r "$pcap_file" -T fields -e frame.time_relative -e ip.src -e ip.dst -e udp.srcport -e udp.dstport 2>/dev/null | head -15 | while read time src dst sport dport; do
                    echo "  ${time}s: $src:$sport → $dst:$dport"
                done
                
                # Analyze flow direction patterns for 3-way validation
                local src_ports=$(tshark -r "$pcap_file" -T fields -e udp.srcport 2>/dev/null | sort -u | wc -l)
                local dst_ports=$(tshark -r "$pcap_file" -T fields -e udp.dstport 2>/dev/null | sort -u | wc -l)
                
                echo ""
                echo "Flow analysis:"
                echo "  Source ports: $src_ports"
                echo "  Destination ports: $dst_ports"
                
                # Check for specific client ports to verify all 3 participants
                local has_client_a=$(tshark -r "$pcap_file" -T fields -e udp.srcport -e udp.dstport 2>/dev/null | grep -E "506[12]" | wc -l)
                local has_client_b=$(tshark -r "$pcap_file" -T fields -e udp.srcport -e udp.dstport 2>/dev/null | grep -E "506[23]" | wc -l)
                local has_client_c=$(tshark -r "$pcap_file" -T fields -e udp.srcport -e udp.dstport 2>/dev/null | grep -E "5063" | wc -l)
                
                echo "  Client A (5061/5062) packets: $has_client_a"
                echo "  Client B (5062/5063) packets: $has_client_b"
                echo "  Client C (5063) packets: $has_client_c"
                
                if [ "$src_ports" -ge 5 ] && [ "$dst_ports" -ge 5 ]; then
                    if [ "$has_client_a" -gt 0 ] && [ "$has_client_b" -gt 0 ] && [ "$has_client_c" -gt 0 ]; then
                        printf "${GREEN}✅ Full 3-way bridge flow detected with all participants${NC}\n"
                        printf "${GREEN}✅ N-way conferencing bridge is working correctly${NC}\n"
                    else
                        printf "${YELLOW}⚠ Some participants missing from bridge flow${NC}\n"
                    fi
                else
                    printf "${YELLOW}⚠ Limited flow patterns - 3-way bridge may not be fully active${NC}\n"
                fi
                
                # Check for full-mesh topology evidence
                echo ""
                echo "Full-mesh topology validation:"
                local total_endpoints=$(tshark -r "$pcap_file" -T fields -e ip.src -e ip.dst 2>/dev/null | tr '\t' '\n' | sort -u | wc -l)
                echo "  Unique IP endpoints: $total_endpoints (expecting 4+: 3 clients + server)"
                
                if [ "$total_endpoints" -ge 4 ]; then
                    printf "${GREEN}✅ Multiple endpoints detected - consistent with 3-way conference${NC}\n"
                else
                    printf "${YELLOW}⚠ Limited endpoints - may not be full 3-way conference${NC}\n"
                fi
            fi
        else
            printf "${RED}❌ No RTP packets captured - 3-way bridge may not be working${NC}\n"
        fi
    fi
}

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
    "multi")
        echo "Testing multi-session bridge demo with 3 participants..."
        export BRIDGE_SERVER="multi_session_bridge_demo"
        check_prerequisites
        create_test_audio
        
        printf "\n${BLUE}=== Starting 3-Way Conference Bridge Test ===${NC}\n\n"
        
        # Start the multi-session bridge server
        if ! start_bridge_server; then
            echo "Failed to start multi-session bridge server"
            exit 1
        fi
        
        sleep 2  # Allow server to fully initialize
        
        # Run 3-way bridge test
        printf "\n${PURPLE}Test: 3-Way Conference Bridge${NC}\n"
        if run_3way_bridge_test "3way_conference" 20; then
            printf "${GREEN}✅ 3-way bridge test passed${NC}\n"
        else
            printf "${RED}❌ 3-way bridge test failed${NC}\n"
            exit 1
        fi
        
        # Analyze server activity for 3-way bridge
        analyze_server_logs
        
        printf "\n${BLUE}=== 3-Way Bridge Test Summary ===${NC}\n"
        printf "${GREEN}🎉 Multi-session conferencing test completed!${NC}\n"
        printf "${GREEN}✅ N-way bridge infrastructure tested with 3 participants${NC}\n"
        ;;
    "help")
        echo "Bridge Test Suite Usage:"
        echo ""
        echo "  ./run_bridge_tests.sh [command]"
        echo ""
        echo "Commands:"
        echo "  all     - Run complete bridge test suite (default)"
        echo "  setup   - Setup test environment only"
        echo "  server  - Start bridge server and wait"
        echo "  quick   - Run quick 10-second test (2 participants)"
        echo "  multi   - Test multi-session bridge demo (3 participants)"
        echo "  help    - Show this help"
        echo ""
        echo "Environment Variables:"
        echo "  BRIDGE_SERVER - Choose server example (default: bridge_server)"
        echo "                  Options: bridge_server, multi_session_bridge_demo"
        echo ""
        echo "Examples:"
        echo "  ./run_bridge_tests.sh                    # Test 2-way bridge"
        echo "  ./run_bridge_tests.sh multi              # Test 3-way conference"
        echo "  BRIDGE_SERVER=bridge_server ./run_bridge_tests.sh  # Explicit 2-way"
        ;;
    "all"|*)
        main
        ;;
esac 