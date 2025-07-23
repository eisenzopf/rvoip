#!/bin/bash

set -e

# This script is intended to be run from the root of the project.

# Compile C test
make -C tests/lpc clean
make -C tests/lpc

# Run C test and capture output
./tests/lpc/c_test > tests/lpc/c_output.csv

# Run Rust test and capture output
cargo test --test lpc -- --nocapture | awk '/rust_function_name,rust_output/{flag=1; print; next} /^test test_lpc_functions/{flag=0} flag' > tests/lpc/rust_output.csv

# Create the final CSV with a proper header
echo "c_function_name,c_output,rust_function_name,rust_output" > tests/lpc/comparison.csv

# Combine the CSVs, skipping headers of intermediate files
paste -d ',' <(tail -n +2 tests/lpc/c_output.csv) <(tail -n +2 tests/lpc/rust_output.csv) >> tests/lpc/comparison.csv

echo "Comparison complete. Results in tests/lpc/comparison.csv"

# Analyze for differences
if diff <(tail -n +2 tests/lpc/c_output.csv | cut -d, -f2) <(tail -n +2 tests/lpc/rust_output.csv | cut -d, -f2); then
    echo "Success: C and Rust outputs match."
else
    echo "Failure: C and Rust outputs differ."
    diff -y <(tail -n +2 tests/lpc/c_output.csv) <(tail -n +2 tests/lpc/rust_output.csv)
fi

# Clean up intermediate files
rm tests/lpc/c_output.csv tests/lpc/rust_output.csv
