#!/bin/bash

echo "Building C debug version..."
make c_test_debug

echo "Running C debug test..."
./c_test_debug 2> c_debug.log > c_output_debug.csv

echo "Running Rust test with debug output..."
# Save current directory
SCRIPT_DIR=$(pwd)
# Go to project root
cd ../../../../../..
# Run rust test and redirect output back to the test directory
cargo test -p codec-core test_lsp_quantizer --release -- --nocapture 2> "$SCRIPT_DIR/rust_debug.log" > "$SCRIPT_DIR/rust_output_debug.csv"
# Return to script directory
cd "$SCRIPT_DIR"

echo "Debug logs saved to:"
echo "  - rust_debug.log"
echo "  - c_debug.log"
echo ""
echo "Key differences to look for:"
echo "1. In Lsp_select_2, check if C is accessing lspcb2[k1][j] where j>=5"
echo "2. Compare the values being read from lspcb2"
echo "3. Check if the selected indices (tindex1, tindex2) differ"
echo "4. Compare the intermediate buf values after each stage"

# Show specific Lsp_select_2 outputs
echo ""
echo "=== Lsp_select_2 Comparison ==="
echo "--- C version ---"
grep -A 50 "DEBUG: Lsp_select_2 starting" c_debug.log | head -20
echo ""
echo "--- Rust version ---"
grep -A 50 "DEBUG: lsp_select_2 starting" rust_debug.log | head -20 