#!/bin/bash

# G.729A Encoder Integration Test Comparison Script
# This script runs both C reference and Rust encoder implementations on test vectors
# and compares their bitstream outputs to verify correctness.
# Note: Only encoder testing is performed since the Rust decoder is not yet implemented.

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${YELLOW}G.729A Encoder Integration Test Comparison${NC}"
echo "=============================================="

# Paths - adjusted to run from g729a-new directory
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
TEST_VECTORS_DIR="${SCRIPT_DIR}/../test_vectors"
OUTPUT_DIR="${SCRIPT_DIR}/output"
C_OUTPUT_DIR="${OUTPUT_DIR}/c"
RUST_OUTPUT_DIR="${OUTPUT_DIR}/rust"

# Create output directories
mkdir -p "${C_OUTPUT_DIR}"
mkdir -p "${RUST_OUTPUT_DIR}"

# Build C test
echo -e "${YELLOW}Building C reference implementation...${NC}"
cd "${SCRIPT_DIR}"
make clean
make c_test
cd -

# Build Rust test
echo -e "${YELLOW}Building Rust implementation...${NC}"
cd "${SCRIPT_DIR}"
cargo build --release
cd -

# Function to run tests on a specific test vector
run_test() {
    local test_name=$1
    local input_file=$2
    local expected_bitstream=$3
    
    echo -e "\n${YELLOW}Testing: ${test_name}${NC}"
    echo "Input: ${input_file}"
    
    if [ ! -f "${TEST_VECTORS_DIR}/${input_file}" ]; then
        echo -e "${RED}ERROR: Input file ${input_file} not found${NC}"
        return 1
    fi
    
    # Run C encoder
    echo "Running C encoder..."
    "${SCRIPT_DIR}/c_test" encode "${TEST_VECTORS_DIR}/${input_file}" "${C_OUTPUT_DIR}/${test_name}.bit"
    
    # Run Rust encoder
    echo "Running Rust encoder..."
    "${SCRIPT_DIR}/target/release/rust_test" encode "${TEST_VECTORS_DIR}/${input_file}" "${RUST_OUTPUT_DIR}/${test_name}.bit"
    
    # Compare bitstream outputs
    echo "Comparing encoder outputs..."
    if cmp -s "${C_OUTPUT_DIR}/${test_name}.bit" "${RUST_OUTPUT_DIR}/${test_name}.bit"; then
        echo -e "${GREEN}✓ Encoded bitstreams match${NC}"
        BITSTREAM_MATCH=true
    else
        echo -e "${RED}✗ Encoded bitstreams differ${NC}"
        BITSTREAM_MATCH=false
    fi
    
    # If expected bitstream file exists, compare with reference
    if [ -n "${expected_bitstream}" ] && [ -f "${TEST_VECTORS_DIR}/${expected_bitstream}" ]; then
        echo "Comparing with expected reference bitstream..."
        if cmp -s "${C_OUTPUT_DIR}/${test_name}.bit" "${TEST_VECTORS_DIR}/${expected_bitstream}"; then
            echo -e "${GREEN}✓ C encoder output matches reference${NC}"
            C_REF_MATCH=true
        else
            echo -e "${RED}✗ C encoder output differs from reference${NC}"
            C_REF_MATCH=false
        fi
        
        if cmp -s "${RUST_OUTPUT_DIR}/${test_name}.bit" "${TEST_VECTORS_DIR}/${expected_bitstream}"; then
            echo -e "${GREEN}✓ Rust encoder output matches reference${NC}"
            RUST_REF_MATCH=true
        else
            echo -e "${RED}✗ Rust encoder output differs from reference${NC}"
            RUST_REF_MATCH=false
        fi
    else
        C_REF_MATCH=true  # No reference to compare against
        RUST_REF_MATCH=true
    fi
    
    # Generate detailed comparison if files differ
    if [ "$BITSTREAM_MATCH" = false ]; then
        echo "Generating detailed comparison files..."
        
        # Hexdump comparison for bitstream
        hexdump -C "${C_OUTPUT_DIR}/${test_name}.bit" > "${OUTPUT_DIR}/${test_name}_c_bitstream.hex"
        hexdump -C "${RUST_OUTPUT_DIR}/${test_name}.bit" > "${OUTPUT_DIR}/${test_name}_rust_bitstream.hex"
        echo "Bitstream hex dumps saved to ${OUTPUT_DIR}/${test_name}_*_bitstream.hex"
        
        # Show first few lines of difference
        echo "First 10 lines of difference:"
        diff "${OUTPUT_DIR}/${test_name}_c_bitstream.hex" "${OUTPUT_DIR}/${test_name}_rust_bitstream.hex" | head -10
    fi
    
    return 0
}

# Initialize counters
TOTAL_TESTS=0
PASSED_TESTS=0

# Test cases based on available test vectors
echo -e "\n${YELLOW}Running test cases...${NC}"

# Test 1: Basic speech test
run_test "speech" "SPEECH.IN" "SPEECH.BIT"
TOTAL_TESTS=$((TOTAL_TESTS + 1))
if [ "$BITSTREAM_MATCH" = true ]; then
    PASSED_TESTS=$((PASSED_TESTS + 1))
fi

# Test 2: Algorithmic test
run_test "algthm" "ALGTHM.IN" "ALGTHM.BIT"
TOTAL_TESTS=$((TOTAL_TESTS + 1))
if [ "$BITSTREAM_MATCH" = true ]; then
    PASSED_TESTS=$((PASSED_TESTS + 1))
fi

# Test 3: Fixed codebook test
run_test "fixed" "FIXED.IN" "FIXED.BIT"
TOTAL_TESTS=$((TOTAL_TESTS + 1))
if [ "$BITSTREAM_MATCH" = true ]; then
    PASSED_TESTS=$((PASSED_TESTS + 1))
fi

# Test 4: LSP quantization test
run_test "lsp" "LSP.IN" "LSP.BIT"
TOTAL_TESTS=$((TOTAL_TESTS + 1))
if [ "$BITSTREAM_MATCH" = true ]; then
    PASSED_TESTS=$((PASSED_TESTS + 1))
fi

# Test 5: Pitch analysis test
run_test "pitch" "PITCH.IN" "PITCH.BIT"
TOTAL_TESTS=$((TOTAL_TESTS + 1))
if [ "$BITSTREAM_MATCH" = true ]; then
    PASSED_TESTS=$((PASSED_TESTS + 1))
fi

# Test 6: Taming procedure test
run_test "tame" "TAME.IN" "TAME.BIT"
TOTAL_TESTS=$((TOTAL_TESTS + 1))
if [ "$BITSTREAM_MATCH" = true ]; then
    PASSED_TESTS=$((PASSED_TESTS + 1))
fi

# Summary
echo -e "\n${YELLOW}Test Summary${NC}"
echo "============"
echo "Total tests: ${TOTAL_TESTS}"
echo "Passed: ${PASSED_TESTS}"
echo "Failed: $((TOTAL_TESTS - PASSED_TESTS))"

if [ "$PASSED_TESTS" -eq "$TOTAL_TESTS" ]; then
    echo -e "${GREEN}All tests passed! 🎉${NC}"
    exit 0
else
    echo -e "${RED}Some tests failed. Check the output above for details.${NC}"
    echo "Detailed comparison files are available in the ${OUTPUT_DIR} directory."
    exit 1
fi 