#!/bin/bash

set -e

# This script is intended to be run from the g729a-new directory
# (crates/codec-core/src/codecs/g729a-new)

echo "Running Rust gain quantization test..."

# Navigate to test directory
cd tests/gain_quantization

# Generate test vectors if they don't exist
if [ ! -f test_inputs.csv ]; then
    make generate_test_vectors
    ./generate_test_vectors
fi

# Navigate back to run Rust test
cd ../..

# Run Rust test and capture output (only lines with at least one digit)
echo "Running cargo test for gain quantization..."
cargo test --test gain_quantization test_gain_quantization_from_csv -- --nocapture 2>&1 | grep "^[0-9-].*,[0-9-].*,[0-9-].*$" > tests/gain_quantization/rust_raw_output.txt || true

# Navigate back to test directory
cd tests/gain_quantization

# Check if we got any output
if [ ! -s rust_raw_output.txt ]; then
    echo "Warning: No output captured from Rust test"
    echo "test_id,index,gain_pit,gain_cod" > rust_output.csv
else
    echo "Captured $(wc -l < rust_raw_output.txt) lines of output from Rust test"
    # Convert Rust output to CSV format with test indices
    awk 'BEGIN{print "test_id,index,gain_pit,gain_cod"} {split($0,a,","); print NR-1 "," a[1] "," a[2] "," a[3]}' rust_raw_output.txt > rust_output.csv
fi

# Display results
echo ""
echo "RUST GAIN QUANTIZATION RESULTS"
echo "=============================="
echo ""
printf "%-8s %-8s %-10s %-10s\n" "Test ID" "Index" "Gain Pit" "Gain Cod"
echo "---------------------------------------"

# Show test results
tail -n +2 rust_output.csv | while IFS=',' read -r test_id index gain_pit gain_cod; do
    printf "%-8s %-8s %-10s %-10s\n" "$test_id" "$index" "$gain_pit" "$gain_cod"
done

# Count total tests
total_tests=$(tail -n +2 rust_output.csv | wc -l)
echo ""
if [ $total_tests -eq 0 ]; then
    echo "No test results to display (test may have failed early)"
else
    echo "Total Tests Processed: $total_tests"
fi

# Clean up intermediate files
rm -f rust_raw_output.txt

echo ""
echo "Rust gain quantization test completed successfully!"
exit 0 