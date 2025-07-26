#!/bin/bash

set -e

# This script is intended to be run from the g729a-new directory
# (crates/codec-core/src/codecs/g729a-new)

# Compile C test
make -C tests/lsp clean
make -C tests/lsp

# Run C test and capture output
./tests/lsp/c_test > tests/lsp/c_output.csv

# Run Rust test and capture output
cargo test --test lsp -- --nocapture | awk '/rust_function_name,rust_output/{flag=1; print; next} /^test test_az_lsp/{flag=0} flag' > tests/lsp/rust_output.csv

# Create the final CSV with a proper header
echo "c_function_name,c_output,rust_function_name,rust_output" > tests/lsp/comparison.csv

# Combine the CSVs, skipping headers of intermediate files
paste -d ',' <(tail -n +2 tests/lsp/c_output.csv) <(tail -n +2 tests/lsp/rust_output.csv) >> tests/lsp/comparison.csv

echo "Comparison complete. Results in tests/lsp/comparison.csv"

# Analyze for differences
if diff <(tail -n +2 tests/lsp/c_output.csv | cut -d, -f2) <(tail -n +2 tests/lsp/rust_output.csv | cut -d, -f2); then
    echo "Success: C and Rust outputs match."
else
    echo "Failure: C and Rust outputs differ."
    diff -y <(tail -n +2 tests/lsp/c_output.csv) <(tail -n +2 tests/lsp/rust_output.csv)
fi

# Clean up intermediate files
rm tests/lsp/c_output.csv tests/lsp/rust_output.csv
