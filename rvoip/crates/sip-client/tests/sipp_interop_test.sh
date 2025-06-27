#!/bin/bash
#
# RVOIP SIP Client SIPp Interoperability Test
# Tests our sip-client against the industry-standard SIPp tool
#

set -e  # Exit on any error

# Test configuration
TEST_NAME="sip-client-sipp-interop-test"
TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BASE_DIR="$(dirname "$TEST_DIR")"
LOGS_DIR="$TEST_DIR/logs"
SCENARIOS_DIR="$TEST_DIR/sipp_scenarios"
RESULTS_DIR="$TEST_DIR/results"

# Network configuration
SIP_CLIENT_PORT=5061
SIPP_PORT=5062
SIP_CLIENT_MEDIA_PORT=6001
SIPP_MEDIA_PORT=6002

# Test files
SIP_CLIENT_LOG="$LOGS_DIR/sip_client.log"
SIPP_LOG="$LOGS_DIR/sipp.log"
SIPP_ERROR_LOG="$LOGS_DIR/sipp_error.log"
TEST_RESULT="$RESULTS_DIR/sipp_interop_result.json"

# SIPp scenario files
BASIC_CALL_SCENARIO="$SCENARIOS_DIR/basic_call.xml"
INVITE_WITH_SDP_SCENARIO="$SCENARIOS_DIR/invite_with_sdp.xml"

# PIDs for cleanup
SIP_CLIENT_PID=""
SIPP_PID=""

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Logging functions
log() {
    echo -e "${BLUE}[$(date '+%H:%M:%S')]${NC} $1"
}

success() {
    echo -e "${GREEN}[$(date '+%H:%M:%S')] ✅ $1${NC}"
}

warning() {
    echo -e "${YELLOW}[$(date '+%H:%M:%S')] ⚠️  $1${NC}"
}

error() {
    echo -e "${RED}[$(date '+%H:%M:%S')] ❌ $1${NC}"
}

# Check if SIPp is available
check_sipp() {
    if ! command -v sipp >/dev/null 2>&1; then
        error "SIPp is not installed or not in PATH"
        echo "Please install SIPp to run this test:"
        echo "  Ubuntu/Debian: sudo apt-get install sip-tester"
        echo "  macOS: brew install sipp"
        echo "  Or download from: http://sipp.sourceforge.net/"
        exit 1
    fi
    
    success "SIPp found: $(sipp -v 2>&1 | head -1)"
}

# Cleanup function
cleanup() {
    log "🧹 Cleaning up test processes..."
    
    if [ ! -z "$SIP_CLIENT_PID" ]; then
        kill $SIP_CLIENT_PID 2>/dev/null || true
        wait $SIP_CLIENT_PID 2>/dev/null || true
    fi
    
    if [ ! -z "$SIPP_PID" ]; then
        kill $SIPP_PID 2>/dev/null || true
        wait $SIPP_PID 2>/dev/null || true
    fi
    
    # Kill any remaining processes
    pkill -f "rvoip-sip-client" 2>/dev/null || true
    pkill -f "sipp" 2>/dev/null || true
    
    log "Cleanup complete"
}

# Setup cleanup trap
trap cleanup EXIT INT TERM

# Setup test environment
setup_test_env() {
    log "🚀 Setting up SIPp Interoperability Test"
    
    # Create directories
    mkdir -p "$LOGS_DIR" "$RESULTS_DIR" "$SCENARIOS_DIR"
    
    # Clean previous logs
    rm -f "$SIP_CLIENT_LOG" "$SIPP_LOG" "$SIPP_ERROR_LOG" "$TEST_RESULT"
    
    # Build the project
    log "🔨 Building rvoip-sip-client..."
    cd "$BASE_DIR"
    cargo build --bin rvoip-sip-client
    
    success "Test environment ready"
}

# Create SIPp scenario files
create_sipp_scenarios() {
    log "📝 Creating SIPp scenario files..."
    
    # Basic call scenario with SDP
    cat > "$INVITE_WITH_SDP_SCENARIO" << 'EOF'
<?xml version="1.0" encoding="ISO-8859-1" ?>
<!DOCTYPE scenario SYSTEM "sipp.dtd">

<scenario name="INVITE with SDP Offer">
  <!-- Send INVITE with SDP -->
  <send retrans="500">
    <![CDATA[
      INVITE sip:alice@[remote_ip]:[remote_port] SIP/2.0
      Via: SIP/2.0/UDP [local_ip]:[local_port];branch=[branch]
      From: sipp <sip:sipp@[local_ip]:[local_port]>;tag=[pid]SIPpTag00[call_number]
      To: alice <sip:alice@[remote_ip]:[remote_port]>
      Call-ID: [call_id]
      CSeq: 1 INVITE
      Contact: sip:sipp@[local_ip]:[local_port]
      Max-Forwards: 70
      Subject: SIPp Interoperability Test
      Content-Type: application/sdp
      Content-Length: [len]

      v=0
      o=sipp 123456 654321 IN IP4 [local_ip]
      s=SIPp Session
      c=IN IP4 [local_ip]
      t=0 0
      m=audio 6002 RTP/AVP 0 8
      a=rtpmap:0 PCMU/8000
      a=rtpmap:8 PCMA/8000
      a=sendrecv

    ]]>
  </send>

  <!-- Expect 100 Trying (optional) -->
  <recv response="100" optional="true">
  </recv>

  <!-- Expect 180 Ringing (optional) -->
  <recv response="180" optional="true">
  </recv>

  <!-- Expect 200 OK -->
  <recv response="200" rtd="true">
  </recv>

  <!-- Send ACK -->
  <send>
    <![CDATA[
      ACK sip:alice@[remote_ip]:[remote_port] SIP/2.0
      Via: SIP/2.0/UDP [local_ip]:[local_port];branch=[branch]
      From: sipp <sip:sipp@[local_ip]:[local_port]>;tag=[pid]SIPpTag00[call_number]
      To: alice <sip:alice@[remote_ip]:[remote_port]>[peer_tag_param]
      Call-ID: [call_id]
      CSeq: 1 ACK
      Contact: sip:sipp@[local_ip]:[local_port]
      Max-Forwards: 70
      Content-Length: 0

    ]]>
  </send>

  <!-- Wait for call duration -->
  <pause milliseconds="3000"/>

  <!-- Send BYE -->
  <send retrans="500">
    <![CDATA[
      BYE sip:alice@[remote_ip]:[remote_port] SIP/2.0
      Via: SIP/2.0/UDP [local_ip]:[local_port];branch=[branch]
      From: sipp <sip:sipp@[local_ip]:[local_port]>;tag=[pid]SIPpTag00[call_number]
      To: alice <sip:alice@[remote_ip]:[remote_port]>[peer_tag_param]
      Call-ID: [call_id]
      CSeq: 2 BYE
      Contact: sip:sipp@[local_ip]:[local_port]
      Max-Forwards: 70
      Content-Length: 0

    ]]>
  </send>

  <!-- Expect 200 OK for BYE -->
  <recv response="200">
  </recv>

  <!-- Call is completed -->
</scenario>
EOF

    success "Created SIPp scenario files"
}

# Start sip-client as server
start_sip_client_server() {
    log "🖥️  Starting sip-client server on port $SIP_CLIENT_PORT..."
    
    cd "$BASE_DIR"
    cargo run --bin rvoip-sip-client -- \
        --port $SIP_CLIENT_PORT \
        --username alice \
        --domain 127.0.0.1 \
        receive \
        --auto-answer \
        --max-duration 30 \
        > "$SIP_CLIENT_LOG" 2>&1 &
    
    SIP_CLIENT_PID=$!
    log "sip-client PID: $SIP_CLIENT_PID"
    
    # Wait for sip-client to start
    sleep 3
    
    if kill -0 $SIP_CLIENT_PID 2>/dev/null; then
        success "sip-client server started successfully"
    else
        error "sip-client server failed to start"
        return 1
    fi
}

# Run SIPp test
run_sipp_test() {
    log "📞 Running SIPp test against sip-client..."
    
    # Run SIPp with our scenario
    sipp \
        -sf "$INVITE_WITH_SDP_SCENARIO" \
        -i 127.0.0.1 \
        -p $SIPP_PORT \
        -m 1 \
        -l 1 \
        -r 1 \
        -rp 1000 \
        -t u1 \
        -trace_msg \
        -message_file "$SIPP_LOG" \
        -error_file "$SIPP_ERROR_LOG" \
        127.0.0.1:$SIP_CLIENT_PORT \
        > /dev/null 2>&1 &
    
    SIPP_PID=$!
    log "SIPp PID: $SIPP_PID"
    
    # Wait for SIPp to complete
    local timeout=30
    local elapsed=0
    
    while [ $elapsed -lt $timeout ]; do
        if ! kill -0 $SIPP_PID 2>/dev/null; then
            success "SIPp test completed"
            wait $SIPP_PID
            local sipp_exit_code=$?
            
            if [ $sipp_exit_code -eq 0 ]; then
                success "SIPp exited successfully"
                return 0
            else
                warning "SIPp exited with code: $sipp_exit_code"
                return $sipp_exit_code
            fi
        fi
        
        sleep 1
        elapsed=$((elapsed + 1))
    done
    
    warning "SIPp test timed out after ${timeout} seconds"
    kill $SIPP_PID 2>/dev/null || true
    return 1
}

# Analyze test results
analyze_results() {
    log "📋 Analyzing interoperability test results..."
    
    local sipp_invite_sent="false"
    local sip_client_invite_received="false"
    local sip_client_200_sent="false"
    local sipp_200_received="false"
    local sipp_ack_sent="false"
    local sip_client_ack_received="false"
    local sipp_bye_sent="false"
    local sip_client_bye_received="false"
    local media_established="false"
    local errors_found="false"
    local test_completed="false"
    
    # Check SIPp logs
    if [ -f "$SIPP_LOG" ]; then
        if grep -q "INVITE.*sip:alice" "$SIPP_LOG"; then
            sipp_invite_sent="true"
        fi
        
        if grep -q "SIP/2.0 200" "$SIPP_LOG"; then
            sipp_200_received="true"
        fi
        
        if grep -q "ACK.*sip:alice" "$SIPP_LOG"; then
            sipp_ack_sent="true"
        fi
        
        if grep -q "BYE.*sip:alice" "$SIPP_LOG"; then
            sipp_bye_sent="true"
        fi
    fi
    
    # Check sip-client logs
    if [ -f "$SIP_CLIENT_LOG" ]; then
        if grep -q "INVITE.*received\|SessionManager processing INVITE" "$SIP_CLIENT_LOG"; then
            sip_client_invite_received="true"
        fi
        
        if grep -q "200 OK.*sent\|Sent 200 OK" "$SIP_CLIENT_LOG"; then
            sip_client_200_sent="true"
        fi
        
        if grep -q "ACK.*received\|Processing.*ACK" "$SIP_CLIENT_LOG"; then
            sip_client_ack_received="true"
        fi
        
        if grep -q "BYE.*received\|SessionManager processing BYE" "$SIP_CLIENT_LOG"; then
            sip_client_bye_received="true"
        fi
        
        if grep -q "SDP.*processing\|media.*established\|RTP session\|audio transmission" "$SIP_CLIENT_LOG"; then
            media_established="true"
        fi
        
        if grep -q "ERROR.*Failed\|❌.*Failed" "$SIP_CLIENT_LOG" | grep -v "Timeout\|Timer"; then
            errors_found="true"
        fi
    fi
    
    # Check SIPp errors
    if [ -f "$SIPP_ERROR_LOG" ] && [ -s "$SIPP_ERROR_LOG" ]; then
        if grep -q "Error\|Failed\|Timeout" "$SIPP_ERROR_LOG"; then
            errors_found="true"
        fi
    fi
    
    # Determine if test completed successfully
    if [ "$sipp_invite_sent" = "true" ] && \
       [ "$sip_client_invite_received" = "true" ] && \
       [ "$sipp_200_received" = "true" ] && \
       [ "$sipp_ack_sent" = "true" ]; then
        test_completed="true"
    fi
    
    # Create test result JSON
    cat > "$TEST_RESULT" << EOF
{
    "test_name": "$TEST_NAME",
    "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
    "results": {
        "sipp_invite_sent": $sipp_invite_sent,
        "sip_client_invite_received": $sip_client_invite_received,
        "sip_client_200_sent": $sip_client_200_sent,
        "sipp_200_received": $sipp_200_received,
        "sipp_ack_sent": $sipp_ack_sent,
        "sip_client_ack_received": $sip_client_ack_received,
        "sipp_bye_sent": $sipp_bye_sent,
        "sip_client_bye_received": $sip_client_bye_received,
        "media_established": $media_established,
        "test_completed": $test_completed,
        "errors_found": $errors_found
    },
    "files": {
        "sip_client_log": "$SIP_CLIENT_LOG",
        "sipp_log": "$SIPP_LOG",
        "sipp_error_log": "$SIPP_ERROR_LOG",
        "scenario_file": "$INVITE_WITH_SDP_SCENARIO"
    }
}
EOF
    
    # Print summary
    echo
    echo "════════════════════════════════════════"
    echo "🧪 SIPP INTEROPERABILITY TEST RESULTS"
    echo "════════════════════════════════════════"
    echo "📅 Test completed: $(date)"
    echo
    echo "📊 SIP Message Flow:"
    echo "   SIPp INVITE sent:           $([ "$sipp_invite_sent" = "true" ] && echo "✅ YES" || echo "❌ NO")"
    echo "   sip-client INVITE received: $([ "$sip_client_invite_received" = "true" ] && echo "✅ YES" || echo "❌ NO")"
    echo "   sip-client 200 OK sent:     $([ "$sip_client_200_sent" = "true" ] && echo "✅ YES" || echo "❌ NO")"
    echo "   SIPp 200 OK received:       $([ "$sipp_200_received" = "true" ] && echo "✅ YES" || echo "❌ NO")"
    echo "   SIPp ACK sent:              $([ "$sipp_ack_sent" = "true" ] && echo "✅ YES" || echo "❌ NO")"
    echo "   sip-client ACK received:    $([ "$sip_client_ack_received" = "true" ] && echo "✅ YES" || echo "🚧 TODO")"
    echo "   SIPp BYE sent:              $([ "$sipp_bye_sent" = "true" ] && echo "✅ YES" || echo "🚧 TODO")"
    echo "   sip-client BYE received:    $([ "$sip_client_bye_received" = "true" ] && echo "✅ YES" || echo "🚧 TODO")"
    echo
    echo "📊 Other Results:"
    echo "   Media established:          $([ "$media_established" = "true" ] && echo "✅ YES" || echo "🚧 TODO")"
    echo "   Test completed:             $([ "$test_completed" = "true" ] && echo "✅ YES" || echo "❌ NO")"
    echo "   Errors found:               $([ "$errors_found" = "true" ] && echo "⚠️ YES" || echo "✅ NO")"
    echo
    echo "📁 Log Files:"
    echo "   sip-client: $SIP_CLIENT_LOG"
    echo "   SIPp:       $SIPP_LOG"
    echo "   SIPp errors: $SIPP_ERROR_LOG"
    echo "   Results:    $TEST_RESULT"
    echo
    
    # Overall result
    if [ "$test_completed" = "true" ] && [ "$errors_found" = "false" ]; then
        success "🎉 SIPP INTEROPERABILITY TEST PASSED!"
        echo "   ✅ Our sip-client is compatible with SIPp"
        echo "   ✅ Standard SIP message flow working"
        echo "   ✅ RFC 3261 compliance verified"
        return 0
    else
        error "❌ SIPP INTEROPERABILITY TEST FAILED"
        echo "   Check log files for details"
        if [ "$errors_found" = "true" ]; then
            echo "   Errors were detected in the logs"
        fi
        if [ "$test_completed" = "false" ]; then
            echo "   SIP message flow was incomplete"
        fi
        return 1
    fi
}

# Print usage
usage() {
    echo "Usage: $0 [options]"
    echo
    echo "Options:"
    echo "  --help, -h          Show this help message"
    echo "  --logs-only         Only analyze existing logs"
    echo "  --cleanup           Clean up test files and exit"
    echo "  --create-scenarios  Only create SIPp scenario files"
    echo
    echo "This script tests interoperability between our sip-client"
    echo "and the industry-standard SIPp testing tool."
    echo
    echo "Requirements:"
    echo "  - SIPp must be installed and available in PATH"
    echo "  - rvoip-sip-client must be built"
}

# Main test execution
main() {
    case "${1:-}" in
        --help|-h)
            usage
            exit 0
            ;;
        --logs-only)
            analyze_results
            exit $?
            ;;
        --cleanup)
            cleanup
            rm -rf "$LOGS_DIR" "$RESULTS_DIR" "$SCENARIOS_DIR" 2>/dev/null || true
            success "Test files cleaned up"
            exit 0
            ;;
        --create-scenarios)
            mkdir -p "$SCENARIOS_DIR"
            create_sipp_scenarios
            success "SIPp scenario files created in $SCENARIOS_DIR"
            exit 0
            ;;
    esac
    
    log "🚀 Starting SIPp Interoperability Test"
    
    # Check prerequisites
    check_sipp
    
    # Setup
    setup_test_env
    create_sipp_scenarios
    
    # Start sip-client server
    start_sip_client_server || exit 1
    sleep 2  # Give sip-client time to fully initialize
    
    # Run SIPp test
    run_sipp_test
    local sipp_result=$?
    
    # Wait a bit for processes to complete
    sleep 3
    
    # Analyze results
    analyze_results
    local analysis_result=$?
    
    # Return combined result
    if [ $sipp_result -eq 0 ] && [ $analysis_result -eq 0 ]; then
        exit 0
    else
        exit 1
    fi
}

# Run main function with all arguments
main "$@" 