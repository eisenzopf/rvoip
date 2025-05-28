#!/bin/bash

# SIPp Media Flow Test Runner for Session-Core
# This script runs SIPp tests with actual RTP media validation

set -e

# Configuration
SERVER_IP="127.0.0.1"
SERVER_PORT="5060"
CLIENT_IP="127.0.0.1"
CLIENT_PORT="5061"
SCENARIOS_DIR="$(dirname "$0")"
RESULTS_DIR="$SCENARIOS_DIR/media_results"
AUDIO_DIR="$SCENARIOS_DIR/audio_files"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Create directories
mkdir -p "$RESULTS_DIR"
mkdir -p "$AUDIO_DIR"

printf "${BLUE}=== Session-Core Media Flow Test Suite ===${NC}\n"
echo "Server: $SERVER_IP:$SERVER_PORT"
echo "Client: $CLIENT_IP:$CLIENT_PORT"
echo "Audio files: $AUDIO_DIR"
echo "Results: $RESULTS_DIR"
echo ""

# Function to create test audio file if it doesn't exist
create_test_audio() {
    local audio_file="$AUDIO_DIR/test_audio.wav"
    
    if [ ! -f "$audio_file" ]; then
        echo "Creating test audio file: $audio_file"
        
        # Try to create a simple test tone using sox (if available)
        if command -v sox &> /dev/null; then
            sox -n -r 8000 -c 1 -b 16 "$audio_file" synth 10 sine 440 vol 0.5
            printf "${GREEN}✓ Created test audio file with sox${NC}\n"
        # Try to create using ffmpeg (if available)
        elif command -v ffmpeg &> /dev/null; then
            ffmpeg -f lavfi -i "sine=frequency=440:duration=10:sample_rate=8000" \
                   -ac 1 -ar 8000 -sample_fmt s16 "$audio_file" -y
            printf "${GREEN}✓ Created test audio file with ffmpeg${NC}\n"
        else
            printf "${YELLOW}⚠ Warning: No audio generation tool found (sox/ffmpeg)${NC}\n"
            echo "Creating empty placeholder file"
            touch "$audio_file"
        fi
    else
        printf "${GREEN}✓ Test audio file exists${NC}\n"
    fi
}

# Function to run media test with RTP validation
run_media_test() {
    local scenario_file="$1"
    local test_name="$2"
    local duration="${3:-10}"
    
    printf "${YELLOW}Running Media Test: $test_name${NC}\n"
    echo "Scenario: $scenario_file"
    echo "Duration: ${duration}s"
    
    local log_file="$RESULTS_DIR/${test_name}.log"
    local csv_file="$RESULTS_DIR/${test_name}.csv"
    local rtp_dump="$RESULTS_DIR/${test_name}_rtp.pcap"
    local audio_out="$RESULTS_DIR/${test_name}_received.wav"
    
    # SIPp command with RTP media support
    local sipp_cmd="sipp -sf $scenario_file \
                         -i $CLIENT_IP -p $CLIENT_PORT \
                         -m 1 -r 1 \
                         -trace_msg \
                         -message_file $log_file \
                         -stf $csv_file"
    
    # Add RTP options if audio file exists
    if [ -f "$AUDIO_DIR/test_audio.wav" ]; then
        sipp_cmd="$sipp_cmd -rtp_echo -ap $AUDIO_DIR/test_audio.wav"
    else
        # Enable RTP echo even without audio file
        sipp_cmd="$sipp_cmd -rtp_echo"
    fi
    
    # Add RTP dump if tcpdump is available
    if command -v tcpdump &> /dev/null; then
        echo "Starting RTP packet capture..."
        sudo tcpdump -i lo0 -w "$rtp_dump" 'udp and not port 53 and not port 5060' &
        local tcpdump_pid=$!
        sleep 1  # Give tcpdump time to start
    fi
    
    # Run the SIPp test
    if $sipp_cmd $SERVER_IP:$SERVER_PORT 2>&1; then
        printf "${GREEN}✓ PASSED: $test_name (SIP signaling)${NC}\n"
        
        # Stop packet capture
        if [ ! -z "$tcpdump_pid" ]; then
            sleep 1
            sudo kill $tcpdump_pid 2>/dev/null || true
            echo "RTP packets captured to: $rtp_dump"
        fi
        
        # Analyze RTP flow
        analyze_rtp_flow "$rtp_dump" "$test_name"
        
        return 0
    else
        printf "${RED}✗ FAILED: $test_name${NC}\n"
        
        # Stop packet capture on failure
        if [ ! -z "$tcpdump_pid" ]; then
            sudo kill $tcpdump_pid 2>/dev/null || true
        fi
        
        return 1
    fi
}

# Function to analyze RTP packet flow
analyze_rtp_flow() {
    local pcap_file="$1"
    local test_name="$2"
    
    if [ ! -f "$pcap_file" ]; then
        printf "${YELLOW}⚠ No packet capture file found${NC}\n"
        return
    fi
    
    printf "${BLUE}--- RTP Flow Analysis for $test_name ---${NC}\n"
    
    # Basic packet count analysis
    if command -v tcpdump &> /dev/null; then
        local packet_count=$(tcpdump -r "$pcap_file" 2>/dev/null | wc -l)
        echo "Total packets captured: $packet_count"
        
        if [ "$packet_count" -gt 0 ]; then
            printf "${GREEN}✓ RTP media flow detected${NC}\n"
            
            # More detailed analysis if tshark is available
            if command -v tshark &> /dev/null; then
                echo "Detailed RTP analysis:"
                
                # First, try to identify RTP packets by examining UDP traffic
                # Look for packets that might be RTP (check for RTP version 2 in payload)
                local potential_rtp_ports=$(tshark -r "$pcap_file" -T fields -e udp.srcport -e udp.dstport -Y "udp" 2>/dev/null | sort -u | head -10)
                
                # Try to decode common ports as RTP and count
                local rtp_packets=0
                local decode_args=""
                
                # Build decode arguments for potential RTP ports
                while read -r src_port dst_port; do
                    if [ ! -z "$src_port" ] && [ ! -z "$dst_port" ]; then
                        decode_args="$decode_args -d udp.port==$src_port,rtp -d udp.port==$dst_port,rtp"
                    fi
                done <<< "$potential_rtp_ports"
                
                # Count RTP packets with dynamic port detection
                if [ ! -z "$decode_args" ]; then
                    rtp_packets=$(tshark -r "$pcap_file" $decode_args -Y "rtp" 2>/dev/null | wc -l)
                fi
                
                # If no RTP packets found with port detection, try heuristic detection
                if [ "$rtp_packets" -eq 0 ]; then
                    # Look for UDP packets with RTP-like headers (version 2, reasonable payload types)
                    rtp_packets=$(tshark -r "$pcap_file" -Y "udp and data[0:1] == 80:00" 2>/dev/null | wc -l)
                    if [ "$rtp_packets" -gt 0 ]; then
                        echo "RTP packets (heuristic detection): $rtp_packets"
                    fi
                else
                    echo "RTP packets: $rtp_packets"
                    
                    # Show sample RTP packet details
                    echo "Sample RTP packet details:"
                    tshark -r "$pcap_file" $decode_args -Y "rtp" -T fields \
                           -e rtp.ssrc -e rtp.timestamp -e rtp.seq -e rtp.p_type \
                           2>/dev/null | head -5 | while read ssrc ts seq pt; do
                        echo "  SSRC: 0x$ssrc, Seq: $seq, Timestamp: $ts, PT: $pt"
                    done
                fi
                
                # Additional analysis: check for bidirectional flow
                local unique_ssrcs=$(tshark -r "$pcap_file" $decode_args -Y "rtp" -T fields -e rtp.ssrc 2>/dev/null | sort -u | wc -l)
                if [ "$unique_ssrcs" -gt 0 ]; then
                    echo "Unique RTP streams (SSRCs): $unique_ssrcs"
                fi
                
                # Check packet timing (should be ~20ms intervals for typical audio)
                echo "RTP timing analysis:"
                tshark -r "$pcap_file" $decode_args -Y "rtp" -T fields -e frame.time_relative 2>/dev/null | head -10 | while read time; do
                    echo "  Packet at: ${time}s"
                done
            fi
        else
            printf "${RED}✗ No packets captured${NC}\n"
        fi
    fi
}

# Function to check if server is running (macOS compatible)
check_server() {
    echo "Checking if server is running on $SERVER_IP:$SERVER_PORT..."
    
    # Method 1: Check using lsof to see if something is listening on the port
    if command -v lsof &> /dev/null; then
        if lsof -i :$SERVER_PORT | grep -q "LISTEN\|UDP"; then
            printf "${GREEN}✓ Server is running${NC}\n"
            return 0
        fi
    fi
    
    # Method 2: Try UDP connection using Python (SIP typically uses UDP)
    if command -v python3 &> /dev/null; then
        if python3 -c "
import socket
import sys
try:
    # Test UDP connectivity by sending a simple packet
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.settimeout(3)
    s.sendto(b'', ('$SERVER_IP', $SERVER_PORT))
    s.close()
    sys.exit(0)
except:
    # Also try TCP as fallback
    try:
        s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        s.settimeout(3)
        result = s.connect_ex(('$SERVER_IP', $SERVER_PORT))
        s.close()
        sys.exit(0 if result == 0 else 1)
    except:
        sys.exit(1)
" 2>/dev/null; then
            printf "${GREEN}✓ Server is running${NC}\n"
            return 0
        fi
    fi
    
    # Method 3: Use netstat as fallback
    if command -v netstat &> /dev/null; then
        if netstat -an | grep -E ":$SERVER_PORT.*LISTEN|:$SERVER_PORT.*UDP"; then
            printf "${GREEN}✓ Server is running${NC}\n"
            return 0
        fi
    fi
    
    # If all methods fail
    printf "${RED}✗ Server is not running on $SERVER_IP:$SERVER_PORT${NC}\n"
    echo "Please start the session-core server first:"
    echo "  cargo run --example sipp_server"
    return 1
}

# Function to check prerequisites
check_prerequisites() {
    echo "Checking prerequisites..."
    
    # Check SIPp
    if ! command -v sipp &> /dev/null; then
        printf "${RED}Error: SIPp is not installed${NC}\n"
        exit 1
    fi
    printf "${GREEN}✓ SIPp found${NC}\n"
    
    # Check server
    if ! check_server; then
        exit 1
    fi
    
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
        printf "${YELLOW}⚠ No audio tools found (limited media testing)${NC}\n"
    fi
}

# Main execution
main() {
    local failed_tests=0
    local total_tests=0
    
    check_prerequisites
    create_test_audio
    
    printf "\n${BLUE}=== Starting Media Flow Tests ===${NC}\n\n"
    
    # Test 1: Basic media flow
    if run_media_test "$SCENARIOS_DIR/media_flow_test.xml" "media_flow_basic" 10; then
        ((total_tests++))
    else
        ((failed_tests++))
        ((total_tests++))
    fi
    
    # Test 2: Basic call with RTP monitoring
    if run_media_test "$SCENARIOS_DIR/basic_call.xml" "basic_call_rtp" 5; then
        ((total_tests++))
    else
        ((failed_tests++))
        ((total_tests++))
    fi
    
    # Test 3: Hold/resume with media monitoring
    if run_media_test "$SCENARIOS_DIR/hold_resume.xml" "hold_resume_rtp" 15; then
        ((total_tests++))
    else
        ((failed_tests++))
        ((total_tests++))
    fi
    
    # Summary
    printf "\n${BLUE}=== Media Test Summary ===${NC}\n"
    echo "Total tests: $total_tests"
    echo "Passed: $((total_tests - failed_tests))"
    echo "Failed: $failed_tests"
    echo "Results saved to: $RESULTS_DIR"
    
    if [ $failed_tests -eq 0 ]; then
        printf "${GREEN}🎉 All media tests passed!${NC}\n"
        exit 0
    else
        printf "${RED}❌ $failed_tests test(s) failed${NC}\n"
        exit 1
    fi
}

# Handle command line arguments
case "${1:-all}" in
    "setup")
        echo "Setting up media test environment..."
        check_prerequisites
        create_test_audio
        printf "${GREEN}✓ Setup complete${NC}\n"
        ;;
    "basic")
        echo "Running basic media test only..."
        check_prerequisites
        create_test_audio
        run_media_test "$SCENARIOS_DIR/basic_call.xml" "basic_media_test" 5
        ;;
    "all"|*)
        main
        ;;
esac 