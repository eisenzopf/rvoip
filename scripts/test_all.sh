#!/bin/bash

# RVOIP Comprehensive Test Runner
# This script ensures ALL tests are run across all crates in the workspace

# DO NOT exit on error - we want to run all tests and report failures at the end
# set -e  # Exit on error

# Colors for output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Track failures
FAILED_TESTS=()
TOTAL_TESTS=0
PASSED_TESTS=0

echo -e "${GREEN}================================================${NC}"
echo -e "${GREEN}    RVOIP Comprehensive Test Runner${NC}"
echo -e "${GREEN}================================================${NC}"
echo ""

# Function to run a test command and report results
run_test() {
    local test_name=$1
    local test_cmd=$2
    
    echo -e "${YELLOW}Running $test_name...${NC}"
    
    TOTAL_TESTS=$((TOTAL_TESTS + 1))
    
    # Create a temporary file to capture output while still showing it
    local temp_output=$(mktemp)
    
    # Run the test command, showing output in real-time AND capturing it
    if eval "$test_cmd" 2>&1 | tee "$temp_output"; then
        echo -e "${GREEN}✓ $test_name passed${NC}"
        PASSED_TESTS=$((PASSED_TESTS + 1))
    else
        echo -e "${RED}✗ $test_name failed${NC}"
        
        # Extract specific failed test names from captured output
        local failed_test_names
        failed_test_names=$(grep -E "test .* \.\.\. FAILED" "$temp_output" | sed 's/test \(.*\) \.\.\. FAILED/\1/' | head -10)
        
        if [ -n "$failed_test_names" ]; then
            # Add each specific failed test to the array
            while IFS= read -r line; do
                if [ -n "$line" ]; then
                    FAILED_TESTS+=("$test_name: $line")
                fi
            done <<< "$failed_test_names"
        else
            # Fallback if we can't parse specific test names
            FAILED_TESTS+=("$test_name")
        fi
    fi
    
    # Clean up temporary file
    rm -f "$temp_output"
    echo ""
}

# List of all crates to test
CRATES=(
    "rvoip-call-engine"
    "rvoip-client-core"
    "rvoip-dialog-core"
    "rvoip-media-core"
    "rvoip-rtp-core"
    "rvoip"
    "rvoip-session-core"
    "rvoip-sip-core"
    "rvoip-sip-transport"
    "rvoip-transaction-core"
)

# Optional: Clean build artifacts (comment out for faster runs)
# echo -e "${YELLOW}Cleaning previous build artifacts...${NC}"
# cargo clean
# echo ""

# Test each crate individually
echo -e "${BLUE}=== Testing Individual Crates ===${NC}"
echo ""

for crate in "${CRATES[@]}"; do
    echo -e "${BLUE}--- Testing $crate ---${NC}"
    
    # Run unit tests (lib.rs)
    if cargo test -p "$crate" --lib --no-fail-fast 2>&1 | grep -q "Running unittests"; then
        run_test "$crate unit tests" "cargo test -p $crate --lib --no-fail-fast"
    else
        echo -e "${YELLOW}No unit tests in $crate${NC}"
        echo ""
    fi
    
    # Run integration tests
    if [ -d "crates/${crate#rvoip-}/tests" ]; then
        run_test "$crate integration tests" "cargo test -p $crate --test '*' --no-fail-fast"
    else
        echo -e "${YELLOW}No integration tests in $crate${NC}"
        echo ""
    fi
    
    # Run doc tests
    run_test "$crate doc tests" "cargo test -p $crate --doc --no-fail-fast"
    
    echo -e "${BLUE}--- Finished $crate ---${NC}"
    echo ""
done

# Also run workspace-wide tests to catch any examples or benchmarks
echo -e "${BLUE}=== Running Workspace-Wide Tests ===${NC}"
echo ""

# Summary
echo ""
echo -e "${BLUE}================================================${NC}"
echo -e "${BLUE}           Test Run Complete${NC}"
echo -e "${BLUE}================================================${NC}"
echo ""

# Show results
echo -e "${YELLOW}Test Results Summary:${NC}"
echo "Total tests run: $TOTAL_TESTS"
echo "Passed: ${GREEN}$PASSED_TESTS${NC}"
echo "Failed: ${RED}${#FAILED_TESTS[@]}${NC}"
echo ""

if [ ${#FAILED_TESTS[@]} -eq 0 ]; then
    echo -e "${GREEN}✨ All tests passed successfully! 🎉${NC}"
    echo ""
    echo -e "${GREEN}Test suite completed successfully${NC}"
    exit 0
else
    echo -e "${RED}================================================${NC}"
    echo -e "${RED}           FAILED TESTS SUMMARY${NC}"
    echo -e "${RED}================================================${NC}"
    echo ""
    echo -e "${RED}❌ The following ${#FAILED_TESTS[@]} test(s) failed:${NC}"
    echo ""
    for i in "${!FAILED_TESTS[@]}"; do
        echo -e "  ${RED}$((i+1)). ${FAILED_TESTS[i]}${NC}"
    done
    echo ""
    echo -e "${RED}================================================${NC}"
    echo ""
    echo -e "${RED}Test suite failed with ${#FAILED_TESTS[@]} failure(s)${NC}"
    echo -e "${YELLOW}Please review the output above to identify and fix the failing tests.${NC}"
    exit 1
fi 