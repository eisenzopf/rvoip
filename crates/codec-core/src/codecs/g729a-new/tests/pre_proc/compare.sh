#!/bin/bash

set -e

# This script is intended to be run from the root of the project.

# Compile C test
make -C tests/pre_proc clean
make -C tests/pre_proc

# Run C test and capture output
./tests/pre_proc/c_test > tests/pre_proc/c_output.csv

# Run Rust test and capture output
cargo test --test pre_proc -- --nocapture | awk '/rust_function_name,rust_output/{flag=1; print; next} /^test test_all_functions/{flag=0} flag' > tests/pre_proc/rust_output.csv

# Create the final CSV with a proper header
echo "c_function_name,c_output,rust_function_name,rust_output" > tests/pre_proc/comparison.csv

# Combine the CSVs, skipping headers of intermediate files
paste -d ',' <(tail -n +2 tests/pre_proc/c_output.csv) <(tail -n +2 tests/pre_proc/rust_output.csv) >> tests/pre_proc/comparison.csv

echo "Comparison complete. Results in tests/pre_proc/comparison.csv"

# Analyze for differences
if diff <(tail -n +2 tests/pre_proc/c_output.csv | cut -d, -f2) <(tail -n +2 tests/pre_proc/rust_output.csv | cut -d, -f2); then
    echo "Success: C and Rust outputs match."
else
    echo "Failure: C and Rust outputs differ."
    diff -y <(tail -n +2 tests/pre_proc/c_output.csv) <(tail -n +2 tests/pre_proc/rust_output.csv)
fi

# Clean up intermediate files
rm tests/pre_proc/c_output.csv tests/pre_proc/rust_output.csv
