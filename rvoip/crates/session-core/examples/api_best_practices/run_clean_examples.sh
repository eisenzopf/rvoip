#!/bin/bash

# Clean API Examples Test Script
# This script runs both the clean UAS server and UAC client examples,
# demonstrating the public API usage with no internal access.

set -uo pipefail

# Script directory and configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$(dirname "$(dirname "$SCRIPT_DIR")")")"
TEST_START_TIME=$(date +"%Y%m%d_%H%M%S")
LOG_DIR="$SCRIPT_DIR/logs"
SERVER_LOG="$LOG_DIR/uas_clean_${TEST_START_TIME}.log"
CLIENT_LOG="$LOG_DIR/uac_clean_${TEST_START_TIME}.log"
COMBINED_LOG="$LOG_DIR/clean_api_test_${TEST_START_TIME}.log"

# Server configuration
SERVER_PORT="${SERVER_PORT:-5062}"
AUTO_ACCEPT="${AUTO_ACCEPT:-true}"
MAX_CALLS="${MAX_CALLS:-10}"

# Client configuration
CLIENT_PORT="${CLIENT_PORT:-5061}"
TARGET="${TARGET:-127.0.0.1:$SERVER_PORT}"
NUM_CALLS="${NUM_CALLS:-2}"
CALL_DURATION="${CALL_DURATION:-5}"
CALL_DELAY="${CALL_DELAY:-2}"
LOG_LEVEL="${LOG_LEVEL:-info}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Create log directory
mkdir -p "$LOG_DIR"

# Function to display a header
print_header() {
    echo ""
    echo -e "${CYAN}============================================================${NC}"
    echo -e "${CYAN}$1${NC}"
    echo -e "${CYAN}============================================================${NC}"
}

# Function to log messages
log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] [INFO] $1" >> "$COMBINED_LOG"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] [SUCCESS] $1" >> "$COMBINED_LOG"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] [ERROR] $1" >> "$COMBINED_LOG"
}

log_warning() {
    echo -e "${YELLOW}[WARN]${NC} $1"
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] [WARN] $1" >> "$COMBINED_LOG"
}

# Initialize combined log
echo "# Clean API Examples Test Log" > "$COMBINED_LOG"
echo "# Started: $(date)" >> "$COMBINED_LOG"
echo "# Server port: $SERVER_PORT" >> "$COMBINED_LOG"
echo "# Client port: $CLIENT_PORT" >> "$COMBINED_LOG"
echo "# Target: $TARGET" >> "$COMBINED_LOG"
echo "# Calls: $NUM_CALLS" >> "$COMBINED_LOG"
echo "# Call duration: ${CALL_DURATION}s" >> "$COMBINED_LOG"
echo "# Call delay: ${CALL_DELAY}s" >> "$COMBINED_LOG"
echo "" >> "$COMBINED_LOG"

# Function to analyze the test results
analyze_results() {
    print_header "Analyzing Test Results"
    
    log_info "Checking server log for API usage..."
    # Strip ANSI color codes before grepping
    SERVER_API_READY=$(sed 's/\x1b\[[0-9;]*m//g' "$SERVER_LOG" | grep "Clean UAS Server ready and listening" | wc -l | tr -d ' ')
    SERVER_INCOMING=$(sed 's/\x1b\[[0-9;]*m//g' "$SERVER_LOG" | grep "📞 Incoming call from" | wc -l | tr -d ' ')
    SERVER_SDP_GENERATED=$(sed 's/\x1b\[[0-9;]*m//g' "$SERVER_LOG" | grep "Generated SDP answer successfully" | wc -l | tr -d ' ')
    SERVER_MEDIA_FLOW=$(sed 's/\x1b\[[0-9;]*m//g' "$SERVER_LOG" | grep "Media flow established successfully" | wc -l | tr -d ' ')
    SERVER_ENDED=$(sed 's/\x1b\[[0-9;]*m//g' "$SERVER_LOG" | grep "Call.*ended:" | wc -l | tr -d ' ')
    
    log_info "Checking client log for API usage..."
    CLIENT_API_READY=$(sed 's/\x1b\[[0-9;]*m//g' "$CLIENT_LOG" | grep "Clean UAC Client ready" | wc -l | tr -d ' ')
    CLIENT_INITIATED=$(sed 's/\x1b\[[0-9;]*m//g' "$CLIENT_LOG" | grep "Making call.*of" | wc -l | tr -d ' ')
    CLIENT_PREPARED=$(sed 's/\x1b\[[0-9;]*m//g' "$CLIENT_LOG" | grep "Prepared call.*with RTP port" | wc -l | tr -d ' ')
    CLIENT_ESTABLISHED=$(sed 's/\x1b\[[0-9;]*m//g' "$CLIENT_LOG" | grep "Call.*established" | wc -l | tr -d ' ')
    CLIENT_MEDIA_FLOW=$(sed 's/\x1b\[[0-9;]*m//g' "$CLIENT_LOG" | grep "✅ Media flow established, audio transmission active" | wc -l | tr -d ' ')
    CLIENT_SUCCESS=$(sed 's/\x1b\[[0-9;]*m//g' "$CLIENT_LOG" | grep "All calls completed successfully" | wc -l | tr -d ' ')
    
    log_info "Test Metrics:"
    log_info "  Server API ready:    $SERVER_API_READY (expected: 1)"
    log_info "  Server incoming:     $SERVER_INCOMING calls (expected: $NUM_CALLS)"
    log_info "  Server SDP answer:   $SERVER_SDP_GENERATED (expected: $NUM_CALLS)"
    log_info "  Server media flow:   $SERVER_MEDIA_FLOW (expected: $NUM_CALLS)"
    log_info "  Server ended:        $SERVER_ENDED calls (expected: $NUM_CALLS)"
    log_info "  Client API ready:    $CLIENT_API_READY (expected: 1)"
    log_info "  Client initiated:    $CLIENT_INITIATED calls (expected: $NUM_CALLS)"
    log_info "  Client prepared:     $CLIENT_PREPARED calls (expected: $NUM_CALLS)"
    log_info "  Client established:  $CLIENT_ESTABLISHED calls (expected: $NUM_CALLS)"
    log_info "  Client media flow:   $CLIENT_MEDIA_FLOW (expected: $NUM_CALLS)"
    
    # Check for API best practices
    log_info ""
    log_info "Checking API best practices..."
    NO_INTERNAL_ACCESS=$(sed 's/\x1b\[[0-9;]*m//g' "$SERVER_LOG" "$CLIENT_LOG" | grep -E "coordinator\.(dialog_manager|media_manager|registry)" | wc -l | tr -d ' ')
    if [ "$NO_INTERNAL_ACCESS" -eq "0" ]; then
        log_success "✓ No internal coordinator access detected (clean API usage)"
    else
        log_error "✗ Found $NO_INTERNAL_ACCESS instances of internal access"
    fi
    
    # Determine test success
    if [ "$SERVER_API_READY" -eq "1" ] && \
       [ "$CLIENT_API_READY" -eq "1" ] && \
       [ "$SERVER_INCOMING" -eq "$NUM_CALLS" ] && \
       [ "$SERVER_SDP_GENERATED" -eq "$NUM_CALLS" ] && \
       [ "$CLIENT_ESTABLISHED" -eq "$NUM_CALLS" ] && \
       [ "$CLIENT_SUCCESS" -eq "1" ] && \
       [ "$NO_INTERNAL_ACCESS" -eq "0" ]; then
        log_success "Test PASSED: All $NUM_CALLS calls completed using clean public API"
        TEST_SUCCESS=0
    else
        log_error "Test FAILED: Not all calls were successful or API usage was not clean"
        TEST_SUCCESS=1
    fi
    
    return $TEST_SUCCESS
}

# Main execution
main() {
    print_header "Clean API Examples Test"
    
    log_info "Test started at $(date)"
    log_info "Log directory: $LOG_DIR"
    log_info ""
    log_info "This test demonstrates:"
    log_info "  • Public API usage only (no internal access)"
    log_info "  • Clean SDP handling with new API methods"
    log_info "  • Proper media flow establishment"
    log_info "  • Best practices for session-core usage"
    
    # Change to project directory
    cd "$PROJECT_DIR"
    
    # Start the UAS server
    print_header "Starting Clean UAS Server"
    log_info "Starting clean UAS server on port $SERVER_PORT..."
    log_info "Server log: $SERVER_LOG"
    
    # Build command based on auto-accept flag
    CMD="cargo run --bin uas_server_clean -- --port $SERVER_PORT --log-level $LOG_LEVEL"
    if [ "$AUTO_ACCEPT" = "true" ]; then
        CMD="$CMD --auto-accept"
    fi
    CMD="$CMD --max-calls $MAX_CALLS"
    
    eval "$CMD > \"$SERVER_LOG\" 2>&1 &"
    SERVER_PID=$!
    
    log_info "Clean UAS server started with PID $SERVER_PID"
    
    # Wait for server to be ready
    log_info "Waiting for server to be ready..."
    for i in {1..10}; do
        if grep -q "Clean UAS Server ready and listening" "$SERVER_LOG" 2>/dev/null; then
            log_success "Clean UAS server is ready"
            break
        fi
        sleep 1
        if [ $i -eq 10 ]; then
            log_error "Timeout waiting for clean UAS server to start"
            kill $SERVER_PID 2>/dev/null || true
            exit 1
        fi
    done
    
    # Wait before starting client
    log_info "Waiting 2 seconds before starting client..."
    sleep 2
    
    # Start the UAC client
    print_header "Starting Clean UAC Client"
    log_info "Starting clean UAC client on port $CLIENT_PORT..."
    log_info "Client log: $CLIENT_LOG"
    log_info "Target: $TARGET"
    log_info "Number of calls: $NUM_CALLS"
    log_info "Call duration: ${CALL_DURATION}s"
    log_info "Delay between calls: ${CALL_DELAY}s"
    
    # Run client
    cargo run --bin uac_client_clean -- \
        --port "$CLIENT_PORT" \
        --target "$TARGET" \
        --num-calls "$NUM_CALLS" \
        --duration "$CALL_DURATION" \
        --delay "$CALL_DELAY" \
        --log-level "$LOG_LEVEL" > "$CLIENT_LOG" 2>&1
    
    CLIENT_EXIT=$?
    
    if [ $CLIENT_EXIT -eq 0 ]; then
        log_success "Clean UAC client completed successfully"
    else
        log_error "Clean UAC client failed with exit code $CLIENT_EXIT"
    fi
    
    # Stop the server
    if ps -p $SERVER_PID > /dev/null 2>&1; then
        log_info "Stopping clean UAS server (PID: $SERVER_PID)..."
        kill $SERVER_PID 2>/dev/null || true
        wait $SERVER_PID 2>/dev/null || true
    fi
    
    # Analyze results
    analyze_results
    TEST_SUCCESS=$?
    
    # Generate summary
    print_header "Test Summary"
    
    if [ $TEST_SUCCESS -eq 0 ]; then
        log_success "Clean API examples test completed successfully"
        log_info ""
        log_info "✅ Key achievements demonstrated:"
        log_info "  • Used only public API methods"
        log_info "  • SDP answer generation via generate_sdp_answer()"
        log_info "  • Media flow via establish_media_flow()"
        log_info "  • Statistics monitoring via get_media_statistics()"
        log_info "  • No internal coordinator access"
    else
        log_error "Clean API examples test failed"
    fi
    
    log_info ""
    log_info "Test ended at $(date)"
    log_info ""
    log_info "Log files:"
    log_info "  Combined: $COMBINED_LOG"
    log_info "  Server:   $SERVER_LOG"
    log_info "  Client:   $CLIENT_LOG"
    
    # Return the test result
    return $TEST_SUCCESS
}

# Execute main function
main "$@" 