#!/bin/bash

set -e

# This script is intended to be run from the root of the project.

# Compile C test
make -C tests/discrete_comparison clean
make -C tests/discrete_comparison

# Run C test and capture output
./tests/discrete_comparison/c_test > tests/discrete_comparison/c_output.csv

# Run Rust test and capture output
cargo test --test discrete_comparison -- --nocapture | awk '/rust_function_name,rust_output/{flag=1; print; next} /^test test_all_functions/{flag=0} flag' > tests/discrete_comparison/rust_output.csv

# Create the final CSV with a proper header
echo "c_function_name,c_output,rust_function_name,rust_output" > tests/discrete_comparison/comparison.csv

# Combine the CSVs, skipping headers of intermediate files
paste -d ',' <(tail -n +2 tests/discrete_comparison/c_output.csv) <(tail -n +2 tests/discrete_comparison/rust_output.csv) >> tests/discrete_comparison/comparison.csv

echo "Comparison complete. Results in tests/discrete_comparison/comparison.csv"

# Analyze for differences
if diff <(tail -n +2 tests/discrete_comparison/c_output.csv | cut -d, -f2) <(tail -n +2 tests/discrete_comparison/rust_output.csv | cut -d, -f2); then
    echo "Success: C and Rust outputs match."
else
    echo "Failure: C and Rust outputs differ."
    diff -y <(tail -n +2 tests/discrete_comparison/c_output.csv) <(tail -n +2 tests/discrete_comparison/rust_output.csv)
fi

# Clean up intermediate files
rm tests/discrete_comparison/c_output.csv tests/discrete_comparison/rust_output.csv
