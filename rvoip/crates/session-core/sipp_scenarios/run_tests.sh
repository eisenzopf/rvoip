#!/bin/bash

# SIPp Test Runner for Session-Core
# This script runs all SIPp test scenarios against the session-core server

set -e

# Configuration
SERVER_IP="127.0.0.1"
SERVER_PORT="5060"
CLIENT_IP="127.0.0.1"
CLIENT_PORT="5061"
SCENARIOS_DIR="$(dirname "$0")"
RESULTS_DIR="$SCENARIOS_DIR/results"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Create results directory
mkdir -p "$RESULTS_DIR"

echo -e "${BLUE}=== Session-Core SIPp Test Suite ===${NC}"
echo "Server: $SERVER_IP:$SERVER_PORT"
echo "Client: $CLIENT_IP:$CLIENT_PORT"
echo "Results: $RESULTS_DIR"
echo ""

# Function to run a single test
run_test() {
    local scenario_file="$1"
    local test_name="$2"
    local call_count="${3:-1}"
    local call_rate="${4:-1}"
    local extra_args="$5"
    
    echo -e "${YELLOW}Running: $test_name${NC}"
    echo "Scenario: $scenario_file"
    echo "Calls: $call_count, Rate: $call_rate cps"
    
    local log_file="$RESULTS_DIR/${test_name}.log"
    local csv_file="$RESULTS_DIR/${test_name}.csv"
    
    # Run SIPp
    if sipp -sf "$scenario_file" \
           -i "$CLIENT_IP" -p "$CLIENT_PORT" \
           -m "$call_count" -r "$call_rate" \
           -trace_msg -trace_shortmsg \
           -message_file "$log_file" \
           -stf "$csv_file" \
           $extra_args \
           "$SERVER_IP:$SERVER_PORT" 2>&1; then
        echo -e "${GREEN}✓ PASSED: $test_name${NC}"
        return 0
    else
        echo -e "${RED}✗ FAILED: $test_name${NC}"
        return 1
    fi
}

# Function to check if SIPp is installed
check_sipp() {
    if ! command -v sipp &> /dev/null; then
        echo -e "${RED}Error: SIPp is not installed or not in PATH${NC}"
        echo "Please install SIPp: https://github.com/SIPp/sipp"
        exit 1
    fi
    
    echo -e "${GREEN}SIPp found: $(sipp -v 2>&1 | head -1)${NC}"
}

# Function to check if server is running
check_server() {
    echo "Checking if server is running on $SERVER_IP:$SERVER_PORT..."
    if nc -z "$SERVER_IP" "$SERVER_PORT" 2>/dev/null; then
        echo -e "${GREEN}✓ Server is running${NC}"
    else
        echo -e "${RED}✗ Server is not running on $SERVER_IP:$SERVER_PORT${NC}"
        echo "Please start the session-core server first:"
        echo "  cargo run --example sipp_server"
        exit 1
    fi
}

# Main test execution
main() {
    local failed_tests=0
    local total_tests=0
    
    check_sipp
    check_server
    
    echo -e "\n${BLUE}=== Starting Test Execution ===${NC}\n"
    
    # Basic functionality tests
    echo -e "${BLUE}--- Basic Functionality Tests ---${NC}"
    
    if run_test "$SCENARIOS_DIR/basic_call.xml" "basic_call" 1 1; then
        ((total_tests++))
    else
        ((failed_tests++))
        ((total_tests++))
    fi
    
    if run_test "$SCENARIOS_DIR/call_rejection.xml" "call_rejection" 1 1; then
        ((total_tests++))
    else
        ((failed_tests++))
        ((total_tests++))
    fi
    
    if run_test "$SCENARIOS_DIR/call_cancel.xml" "call_cancel" 1 1; then
        ((total_tests++))
    else
        ((failed_tests++))
        ((total_tests++))
    fi
    
    if run_test "$SCENARIOS_DIR/options_ping.xml" "options_ping" 1 1; then
        ((total_tests++))
    else
        ((failed_tests++))
        ((total_tests++))
    fi
    
    # Advanced functionality tests
    echo -e "\n${BLUE}--- Advanced Functionality Tests ---${NC}"
    
    if run_test "$SCENARIOS_DIR/hold_resume.xml" "hold_resume" 1 1; then
        ((total_tests++))
    else
        ((failed_tests++))
        ((total_tests++))
    fi
    
    if run_test "$SCENARIOS_DIR/early_media.xml" "early_media" 1 1; then
        ((total_tests++))
    else
        ((failed_tests++))
        ((total_tests++))
    fi
    
    if run_test "$SCENARIOS_DIR/multiple_codecs.xml" "multiple_codecs" 1 1; then
        ((total_tests++))
    else
        ((failed_tests++))
        ((total_tests++))
    fi
    
    if run_test "$SCENARIOS_DIR/forking_test.xml" "forking_test" 1 1; then
        ((total_tests++))
    else
        ((failed_tests++))
        ((total_tests++))
    fi
    
    # Stress and reliability tests
    echo -e "\n${BLUE}--- Stress and Reliability Tests ---${NC}"
    
    if run_test "$SCENARIOS_DIR/stress_test.xml" "stress_test_single" 10 2; then
        ((total_tests++))
    else
        ((failed_tests++))
        ((total_tests++))
    fi
    
    if run_test "$SCENARIOS_DIR/stress_test.xml" "stress_test_burst" 50 10; then
        ((total_tests++))
    else
        ((failed_tests++))
        ((total_tests++))
    fi
    
    if run_test "$SCENARIOS_DIR/timeout_test.xml" "timeout_test" 1 1; then
        ((total_tests++))
    else
        ((failed_tests++))
        ((total_tests++))
    fi
    
    # Summary
    echo -e "\n${BLUE}=== Test Summary ===${NC}"
    echo "Total tests: $total_tests"
    echo "Passed: $((total_tests - failed_tests))"
    echo "Failed: $failed_tests"
    
    if [ $failed_tests -eq 0 ]; then
        echo -e "${GREEN}🎉 All tests passed!${NC}"
        exit 0
    else
        echo -e "${RED}❌ $failed_tests test(s) failed${NC}"
        echo "Check logs in: $RESULTS_DIR"
        exit 1
    fi
}

# Handle command line arguments
case "${1:-all}" in
    "basic")
        echo "Running basic tests only..."
        # Run only basic_call test
        check_sipp
        check_server
        run_test "$SCENARIOS_DIR/basic_call.xml" "basic_call" 1 1
        ;;
    "stress")
        echo "Running stress tests only..."
        check_sipp
        check_server
        run_test "$SCENARIOS_DIR/stress_test.xml" "stress_test" 100 20
        ;;
    "all"|*)
        main
        ;;
esac 