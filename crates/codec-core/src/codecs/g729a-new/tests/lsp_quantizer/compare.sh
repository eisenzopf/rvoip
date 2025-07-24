#!/bin/bash

set -e

# This script is intended to be run from the root of the project.

# Compile C test
make -C tests/lsp_quantizer clean
make -C tests/lsp_quantizer

# Run C test and capture output
./tests/lsp_quantizer/c_test > tests/lsp_quantizer/c_output.csv

# Run Rust test and capture output
cargo test --test lsp_quantizer -- --nocapture | awk '/rust_function_name,rust_output/{flag=1; print; next} /^test test_lsp_quantizer/{flag=0} flag' > tests/lsp_quantizer/rust_output.csv

# Create the final CSV with a proper header
echo "c_function_name,c_output,rust_function_name,rust_output" > tests/lsp_quantizer/comparison.csv

# Combine the CSVs, skipping headers of intermediate files
paste -d ',' <(tail -n +2 tests/lsp_quantizer/c_output.csv) <(tail -n +2 tests/lsp_quantizer/rust_output.csv) >> tests/lsp_quantizer/comparison.csv

echo "Comparison complete. Results in tests/lsp_quantizer/comparison.csv"

# Analyze for differences
if diff <(tail -n +2 tests/lsp_quantizer/c_output.csv | cut -d, -f2) <(tail -n +2 tests/lsp_quantizer/rust_output.csv | cut -d, -f2); then
    echo "Success: C and Rust outputs match."
else
    echo "Failure: C and Rust outputs differ."
    diff -y <(tail -n +2 tests/lsp_quantizer/c_output.csv) <(tail -n +2 tests/lsp_quantizer/rust_output.csv)
fi

# Clean up intermediate files
rm tests/lsp_quantizer/c_output.csv tests/lsp_quantizer/rust_output.csv
