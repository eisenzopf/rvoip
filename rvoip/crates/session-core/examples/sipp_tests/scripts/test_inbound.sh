#!/bin/bash
#
# SIPp Inbound Test Script
# Tests where SIPp calls our Rust SIP test server
#
# Usage: ./test_inbound.sh <scenario> [options]
#

set -euo pipefail

# Script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

# Default configuration
SCENARIO="${1:-basic_call}"
RUST_SERVER_PORT="${RUST_SERVER_PORT:-5062}"
SIPP_PORT="${SIPP_PORT:-5060}"
TEST_DURATION="${TEST_DURATION:-30}"
CALL_RATE="${CALL_RATE:-1}"
MAX_CALLS="${MAX_CALLS:-10}"
CAPTURE_ENABLED="${CAPTURE_ENABLED:-true}"
CLEANUP_ON_EXIT="${CLEANUP_ON_EXIT:-true}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Logging functions
log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Global variables for cleanup
RUST_SERVER_PID=""
TCPDUMP_PID=""
TEST_START_TIME=""

# Cleanup function
cleanup() {
    log_info "🧹 Cleaning up test processes..."
    
    if [[ -n "$RUST_SERVER_PID" ]]; then
        log_info "Stopping Rust SIP test server (PID: $RUST_SERVER_PID)"
        kill -TERM "$RUST_SERVER_PID" 2>/dev/null || true
        wait "$RUST_SERVER_PID" 2>/dev/null || true
    fi
    
    if [[ -n "$TCPDUMP_PID" ]]; then
        log_info "Stopping packet capture (PID: $TCPDUMP_PID)"
        kill -TERM "$TCPDUMP_PID" 2>/dev/null || true
        wait "$TCPDUMP_PID" 2>/dev/null || true
    fi
    
    # Wait a moment for processes to clean up
    sleep 1
    log_success "✅ Cleanup completed"
}

# Set trap for cleanup on exit
if [[ "$CLEANUP_ON_EXIT" == "true" ]]; then
    trap cleanup EXIT INT TERM
fi

# Function to check if a port is in use
check_port() {
    local port=$1
    if lsof -Pi :$port -sTCP:LISTEN -t >/dev/null 2>&1; then
        return 0  # Port is in use
    else
        return 1  # Port is free
    fi
}

# Function to wait for port to be available
wait_for_port() {
    local host=$1
    local port=$2
    local timeout=${3:-30}
    
    log_info "⏳ Waiting for $host:$port to be available..."
    
    for ((i=0; i<timeout; i++)); do
        if nc -z "$host" "$port" 2>/dev/null; then
            log_success "✅ Port $host:$port is ready"
            return 0
        fi
        sleep 1
    done
    
    log_error "❌ Timeout waiting for $host:$port"
    return 1
}

# Function to start packet capture
start_packet_capture() {
    if [[ "$CAPTURE_ENABLED" != "true" ]]; then
        log_info "📡 Packet capture disabled"
        return 0
    fi
    
    log_info "📡 Starting packet capture..."
    
    # Create captures directory
    mkdir -p "$PROJECT_DIR/captures"
    
    # Generate capture filename with timestamp
    local timestamp=$(date +"%Y%m%d_%H%M%S")
    local capture_file="$PROJECT_DIR/captures/test_${SCENARIO}_${timestamp}.pcap"
    
    # Start tcpdump in background
    sudo tcpdump -i lo0 -w "$capture_file" \
        "port $SIPP_PORT or port $RUST_SERVER_PORT" \
        > "$PROJECT_DIR/captures/tcpdump_${timestamp}.log" 2>&1 &
    
    TCPDUMP_PID=$!
    log_success "✅ Packet capture started (PID: $TCPDUMP_PID)"
    log_info "📄 Capture file: $capture_file"
    
    # Give tcpdump a moment to start
    sleep 2
}

# Function to start Rust SIP test server
start_rust_server() {
    log_info "🚀 Starting Rust SIP test server on port $RUST_SERVER_PORT..."
    
    # Check if port is already in use
    if check_port "$RUST_SERVER_PORT"; then
        log_error "❌ Port $RUST_SERVER_PORT is already in use"
        return 1
    fi
    
    # Build the test server if needed
    log_info "🔨 Building SIP test server..."
    cd "$PROJECT_DIR"
    cargo build --bin sip_test_server --quiet
    
    # Start the server in background
    cargo run --bin sip_test_server -- \
        --port "$RUST_SERVER_PORT" \
        --mode auto-answer \
        --log-level info \
        --auto-shutdown $((TEST_DURATION + 10)) \
        > "logs/server_${TEST_START_TIME}.log" 2>&1 &
    
    RUST_SERVER_PID=$!
    log_success "✅ Rust SIP test server started (PID: $RUST_SERVER_PID)"
    
    # Wait for server to be ready
    if ! wait_for_port "127.0.0.1" "$RUST_SERVER_PORT" 30; then
        log_error "❌ Failed to start Rust SIP test server"
        return 1
    fi
    
    log_success "🎯 Rust SIP test server ready on port $RUST_SERVER_PORT"
}

# Function to run SIPp scenario
run_sipp_scenario() {
    local scenario_file="$PROJECT_DIR/scenarios/sipp_to_rust/${SCENARIO}.xml"
    
    if [[ ! -f "$scenario_file" ]]; then
        log_error "❌ Scenario file not found: $scenario_file"
        return 1
    fi
    
    log_info "🧪 Running SIPp scenario: $SCENARIO"
    log_info "📄 Scenario file: $scenario_file"
    log_info "🎯 Target: 127.0.0.1:$RUST_SERVER_PORT"
    log_info "📊 Call rate: $CALL_RATE calls/sec, Max calls: $MAX_CALLS"
    
    # Create reports directory
    mkdir -p "$PROJECT_DIR/reports"
    
    # Generate report filenames
    local timestamp=$(date +"%Y%m%d_%H%M%S")
    local csv_report="$PROJECT_DIR/reports/sipp_${SCENARIO}_${timestamp}.csv"
    local screen_file="$PROJECT_DIR/reports/sipp_${SCENARIO}_${timestamp}.log"
    
    # Run SIPp
    sipp -sf "$scenario_file" \
         -i 127.0.0.1 \
         -p "$SIPP_PORT" \
         127.0.0.1:"$RUST_SERVER_PORT" \
         -r "$CALL_RATE" \
         -m "$MAX_CALLS" \
         -d 1000 \
         -timeout 30s \
         -trace_msg \
         -trace_screen \
         -screen_file "$screen_file" \
         -stf "$csv_report" \
         -nostdin
    
    local sipp_exit_code=$?
    
    if [[ $sipp_exit_code -eq 0 ]]; then
        log_success "✅ SIPp scenario completed successfully"
        log_info "📊 Reports saved:"
        log_info "  📄 Screen log: $screen_file"
        log_info "  📊 CSV stats: $csv_report"
    else
        log_error "❌ SIPp scenario failed with exit code: $sipp_exit_code"
        if [[ -f "$screen_file" ]]; then
            log_info "📄 Check screen log for details: $screen_file"
        fi
        return 1
    fi
}

# Function to generate test report
generate_report() {
    log_info "📋 Generating test report..."
    
    # TODO: Implement comprehensive report generation
    # This would parse SIPp CSV output, server logs, and packet captures
    # to generate HTML/JUnit reports
    
    log_info "📄 Test report generation placeholder"
    log_info "  🔍 Would analyze SIPp statistics"
    log_info "  📊 Would parse server metrics"
    log_info "  📡 Would analyze packet captures"
    log_success "✅ Report generation completed"
}

# Main execution
main() {
    log_info "🧪 Starting SIPp Inbound Test"
    log_info "📋 Test Configuration:"
    log_info "  🎯 Scenario: $SCENARIO"
    log_info "  🏠 Rust server port: $RUST_SERVER_PORT"
    log_info "  📞 SIPp port: $SIPP_PORT"
    log_info "  ⏱️  Test duration: ${TEST_DURATION}s"
    log_info "  📊 Call rate: $CALL_RATE/sec"
    log_info "  📈 Max calls: $MAX_CALLS"
    
    # Initialize test environment
    TEST_START_TIME=$(date +"%Y%m%d_%H%M%S")
    mkdir -p "$PROJECT_DIR/logs"
    mkdir -p "$PROJECT_DIR/reports"
    mkdir -p "$PROJECT_DIR/captures"
    
    # Start packet capture
    start_packet_capture
    
    # Start Rust SIP test server
    start_rust_server
    
    # Give everything a moment to stabilize
    sleep 2
    
    # Run SIPp scenario
    if run_sipp_scenario; then
        log_success "🎉 Test completed successfully!"
    else
        log_error "💥 Test failed!"
        exit 1
    fi
    
    # Generate test report
    generate_report
    
    log_success "✅ SIPp Inbound Test completed"
}

# Show usage information
usage() {
    echo "Usage: $0 <scenario> [environment variables]"
    echo ""
    echo "Available scenarios:"
    echo "  basic_call       - Simple INVITE/200/ACK/BYE test"
    echo "  call_with_dtmf   - Call with DTMF INFO messages"
    echo "  call_with_hold   - Call with UPDATE hold/resume"
    echo ""
    echo "Environment variables:"
    echo "  RUST_SERVER_PORT - Port for Rust server (default: 5062)"
    echo "  SIPP_PORT        - Port for SIPp (default: 5060)"
    echo "  TEST_DURATION    - Test duration in seconds (default: 30)"
    echo "  CALL_RATE        - Calls per second (default: 1)"
    echo "  MAX_CALLS        - Maximum calls (default: 10)"
    echo "  CAPTURE_ENABLED  - Enable packet capture (default: true)"
    echo "  CLEANUP_ON_EXIT  - Cleanup on exit (default: true)"
    echo ""
    echo "Examples:"
    echo "  $0 basic_call"
    echo "  CALL_RATE=2 MAX_CALLS=20 $0 basic_call"
    echo "  CAPTURE_ENABLED=false $0 basic_call"
}

# Check for help flag
if [[ "${1:-}" == "--help" ]] || [[ "${1:-}" == "-h" ]]; then
    usage
    exit 0
fi

# Check for required tools
if ! command -v sipp >/dev/null 2>&1; then
    log_error "❌ SIPp not found. Please install SIPp first."
    log_info "💡 macOS: brew install sipp"
    log_info "💡 Ubuntu: apt-get install sipp"
    exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
    log_error "❌ Cargo not found. Please install Rust first."
    exit 1
fi

if [[ "$CAPTURE_ENABLED" == "true" ]] && ! command -v tcpdump >/dev/null 2>&1; then
    log_warning "⚠️ tcpdump not found. Packet capture will be disabled."
    CAPTURE_ENABLED="false"
fi

# Run main function
main "$@" 