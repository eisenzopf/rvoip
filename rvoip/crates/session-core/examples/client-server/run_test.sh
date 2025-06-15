#!/bin/bash

# SIP Client-Server Test Script
# This script runs both UAS server and UAC client automatically,
# capturing logs and demonstrating a complete SIP call flow.

set -uo pipefail  # Remove -e to prevent early exit on grep failures

# Script directory and configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$(dirname "$(dirname "$SCRIPT_DIR")")")"
TEST_START_TIME=$(date +"%Y%m%d_%H%M%S")
LOG_DIR="$SCRIPT_DIR/logs"
SERVER_LOG="$LOG_DIR/uas_server_${TEST_START_TIME}.log"
CLIENT_LOG="$LOG_DIR/uac_client_${TEST_START_TIME}.log"
COMBINED_LOG="$LOG_DIR/sip_test_${TEST_START_TIME}.log"

# Server configuration
SERVER_PORT="${SERVER_PORT:-5062}"

# Client configuration
CLIENT_PORT="${CLIENT_PORT:-5061}"
TARGET="${TARGET:-127.0.0.1:$SERVER_PORT}"
NUM_CALLS="${NUM_CALLS:-1}"
CALL_DURATION="${CALL_DURATION:-10}"
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

# Function to log a message
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

# Initialize combined log
echo "# SIP Client-Server Test Log" > "$COMBINED_LOG"
echo "# Started: $(date)" >> "$COMBINED_LOG"
echo "# Server port: $SERVER_PORT" >> "$COMBINED_LOG"
echo "# Client port: $CLIENT_PORT" >> "$COMBINED_LOG"
echo "# Target: $TARGET" >> "$COMBINED_LOG"
echo "# Calls: $NUM_CALLS" >> "$COMBINED_LOG"
echo "# Call duration: ${CALL_DURATION}s" >> "$COMBINED_LOG"
echo "" >> "$COMBINED_LOG"

# Function to analyze the test results
analyze_results() {
    print_header "Analyzing Test Results"
    
    log_info "Checking server log for successful calls..."
    # Strip ANSI color codes before grepping
    SERVER_CALLS=$(sed 's/\x1b\[[0-9;]*m//g' "$SERVER_LOG" | grep "uas_server.*Incoming call from" | wc -l | tr -d ' ')
    SERVER_ACCEPTED=$(sed 's/\x1b\[[0-9;]*m//g' "$SERVER_LOG" | grep "calls_accepted += 1" | wc -l | tr -d ' ')
    SERVER_ENDED=$(sed 's/\x1b\[[0-9;]*m//g' "$SERVER_LOG" | grep "uas_server.*Call.*ended:" | wc -l | tr -d ' ')
    
    log_info "Checking client log for successful calls..."
    CLIENT_INITIATED=$(sed 's/\x1b\[[0-9;]*m//g' "$CLIENT_LOG" | grep "Making call.*of" | wc -l | tr -d ' ')
    CLIENT_CONNECTED=$(sed 's/\x1b\[[0-9;]*m//g' "$CLIENT_LOG" | grep "uac_client.*Call.*established" | wc -l | tr -d ' ')
    CLIENT_ENDED=$(sed 's/\x1b\[[0-9;]*m//g' "$CLIENT_LOG" | grep "Terminated session:" | wc -l | tr -d ' ')
    CLIENT_SUCCESS=$(sed 's/\x1b\[[0-9;]*m//g' "$CLIENT_LOG" | grep "All calls completed successfully" | wc -l | tr -d ' ')
    
    log_info "Server received $SERVER_CALLS calls"
    log_info "Server accepted $SERVER_ACCEPTED calls"
    log_info "Server ended $SERVER_ENDED calls"
    log_info "Client initiated $CLIENT_INITIATED calls"
    log_info "Client connected $CLIENT_CONNECTED calls"
    log_info "Client ended $CLIENT_ENDED calls"
    
    # Determine test success
    if [ "$SERVER_CALLS" -eq "$NUM_CALLS" ] && [ "$CLIENT_ENDED" -eq "$NUM_CALLS" ] && [ "$CLIENT_SUCCESS" -eq "1" ]; then
        log_success "Test PASSED: All $NUM_CALLS calls were successful"
        TEST_SUCCESS=0
    else
        log_error "Test FAILED: Not all calls were successful"
        TEST_SUCCESS=1
    fi
    
    return $TEST_SUCCESS
}

# Main execution
main() {
    print_header "SIP Client-Server Test"
    
    log_info "Test started at $(date)"
    log_info "Log directory: $LOG_DIR"
    
    # Change to project directory
    cd "$PROJECT_DIR"
    
    # Start the server
    print_header "Starting UAS Server"
    log_info "Starting UAS server on port $SERVER_PORT..."
    log_info "Server log: $SERVER_LOG"
    
    # Run server in background with auto-shutdown after 60 seconds
    cargo run --bin uas_server -- --port "$SERVER_PORT" --log-level "$LOG_LEVEL" --auto-shutdown 60 > "$SERVER_LOG" 2>&1 &
    SERVER_PID=$!
    
    log_info "UAS server started with PID $SERVER_PID"
    
    # Wait for server to be ready
    log_info "Waiting for server to be ready..."
    for i in {1..10}; do
        if grep -q "UAS Server ready and listening" "$SERVER_LOG" 2>/dev/null; then
            log_success "UAS server is ready"
            break
        fi
        sleep 1
        if [ $i -eq 10 ]; then
            log_error "Timeout waiting for UAS server to start"
            kill $SERVER_PID 2>/dev/null || true
            exit 1
        fi
    done
    
    # Wait 2 seconds before starting client
    log_info "Waiting 2 seconds before starting client..."
    sleep 2
    
    # Start the client
    print_header "Starting UAC Client"
    log_info "Starting UAC client on port $CLIENT_PORT..."
    log_info "Client log: $CLIENT_LOG"
    log_info "Target: $TARGET"
    log_info "Number of calls: $NUM_CALLS"
    log_info "Call duration: ${CALL_DURATION}s"
    
    # Run client (not in background, so we wait for it to complete)
    cargo run --bin uac_client -- \
        --port "$CLIENT_PORT" \
        --target "$TARGET" \
        --calls "$NUM_CALLS" \
        --duration "$CALL_DURATION" \
        --log-level "$LOG_LEVEL" > "$CLIENT_LOG" 2>&1
    
    CLIENT_EXIT=$?
    
    if [ $CLIENT_EXIT -eq 0 ]; then
        log_success "UAC client completed successfully"
    else
        log_error "UAC client failed with exit code $CLIENT_EXIT"
    fi
    
    # Stop the server
    if ps -p $SERVER_PID > /dev/null 2>&1; then
        log_info "Stopping UAS server (PID: $SERVER_PID)..."
        kill $SERVER_PID 2>/dev/null || true
        wait $SERVER_PID 2>/dev/null || true
    fi
    
    # Analyze results
    analyze_results
    TEST_SUCCESS=$?
    
    # Generate summary
    print_header "Test Summary"
    
    if [ $TEST_SUCCESS -eq 0 ]; then
        log_success "Test completed successfully"
    else
        log_error "Test failed"
    fi
    
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