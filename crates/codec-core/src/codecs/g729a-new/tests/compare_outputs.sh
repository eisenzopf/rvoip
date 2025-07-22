#!/bin/bash

set -e

# Compile C test
make -C tests clean
make -C tests

# Run C test and capture output
./tests/c_test > c_output.txt

# Run Rust test and capture output
cargo test --test rust_test -- --nocapture | awk '/^running 1 test/{flag=1; next} /^test result:/{flag=0} flag' > rust_output.txt

# Compare outputs
if diff c_output.txt rust_output.txt; then
    echo "Success: C and Rust outputs match."
else
    echo "Failure: C and Rust outputs differ."
fi

# Clean up
rm c_output.txt rust_output.txt
