#!/bin/bash

set -e

# Compile C test
make -C tests clean
make -C tests

# Run C test and capture output
./tests/c_test > c_output.csv

# Run Rust test and capture output
cargo test --test rust_test -- --nocapture | awk '/rust_function_name,rust_output/{flag=1; print; next} /^test test_all_functions/{flag=0} flag' > rust_output.csv

# Create the final CSV with a proper header
echo "c_function_name,rust_function_name,c_output,rust_output" > comparison.csv

# Combine the CSVs, skipping headers of intermediate files
paste -d ',' <(tail -n +2 c_output.csv) <(tail -n +2 rust_output.csv) >> comparison.csv

echo "Comparison complete. Results in comparison.csv"

# Analyze for differences
if diff <(tail -n +2 c_output.csv | cut -d, -f2) <(tail -n +2 rust_output.csv | cut -d, -f2); then
    echo "Success: C and Rust outputs match."
else
    echo "Failure: C and Rust outputs differ."
    diff -y <(tail -n +2 c_output.csv) <(tail -n +2 rust_output.csv)
fi

# Clean up
rm c_output.csv rust_output.csv
