#!/bin/bash

set -e

# This script runs from the lsp_quantizer test directory

# Compile C test
make clean
make

# Run C test and capture output
./c_test > c_output.csv

# Run Rust test and capture output from the project root
cd ../../../../../..
cargo test -p codec-core test_lsp_quantizer --release -- --nocapture | awk '/rust_function_name,rust_output/{flag=1; print; next} /^test result:/{flag=0} flag' > crates/codec-core/src/codecs/g729a-new/tests/lsp_quantizer/rust_output.csv
cd crates/codec-core/src/codecs/g729a-new/tests/lsp_quantizer

# Create the final CSV with a proper header
echo "c_function_name,c_output,rust_function_name,rust_output" > comparison.csv

# Combine the CSVs, skipping headers of intermediate files
paste -d ',' <(tail -n +2 c_output.csv) <(tail -n +2 rust_output.csv) >> comparison.csv

echo "Comparison complete. Results in comparison.csv"

# Analyze for differences
if diff <(tail -n +2 c_output.csv | cut -d, -f2) <(tail -n +2 rust_output.csv | cut -d, -f2); then
    echo "Success: C and Rust outputs match."
else
    echo "Failure: C and Rust outputs differ."
    diff -y <(tail -n +2 c_output.csv) <(tail -n +2 rust_output.csv) || true
fi 