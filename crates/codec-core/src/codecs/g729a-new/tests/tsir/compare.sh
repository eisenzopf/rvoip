#!/bin/bash

# Change to the test directory
cd tests/tsir

# Clean previous outputs
make clean

# Compile C code
make generate_test_vectors
make c_test

# Generate test vectors
./generate_test_vectors

# Run C implementation
./c_test

# Build and run Rust implementation
cd ../..
cargo build --bin rust_test
cd tests/tsir
../../target/debug/rust_test

# Compare outputs
echo "test_id,c_value,rust_value,difference,percent_diff" > comparison.csv

# Track test-level statistics using temporary files
rm -f /tmp/test_differences_* 2>/dev/null || true
total_tests=0
current_test_id=""

# Process the output files line by line
while IFS=, read -r test_id c_rest && IFS=, read -r rust_id rust_rest <&3; do
    if [ "$test_id" = "test_id" ]; then
        continue  # Skip header line
    fi

    # Track new test
    if [ "$test_id" != "$current_test_id" ]; then
        current_test_id="$test_id"
        ((total_tests++))
    fi

    # Split the rest of the lines into arrays
    IFS=',' read -ra c_values <<< "$c_rest"
    IFS=',' read -ra rust_values <<< "$rust_rest"

    # Compare each value
    for i in "${!c_values[@]}"; do
        c_val=${c_values[$i]}
        rust_val=${rust_values[$i]}
        
        # Calculate absolute difference
        diff=$((rust_val - c_val))
        abs_diff=${diff#-}  # Remove minus sign if present
        
        # Mark test as having differences if any value differs
        if [ "$diff" != "0" ]; then
            touch "/tmp/test_differences_$test_id"
        fi
        
        # Calculate percentage difference if c_val is not zero
        if [ "$c_val" -ne 0 ]; then
            percent_diff=$(echo "scale=2; ($diff * 100) / $c_val" | bc)
        else
            percent_diff="N/A"
        fi
        
        echo "$test_id,$c_val,$rust_val,$diff,$percent_diff" >> comparison.csv
    done
done < c_output.csv 3< rust_output.csv

# Count matching tests
matching_tests=0
failed_tests=0
failed_test_list=""

for test_num in $(seq 0 $((total_tests - 1))); do
    if [ ! -f "/tmp/test_differences_$test_num" ]; then
        ((matching_tests++))
    else
        ((failed_tests++))
        if [ -z "$failed_test_list" ]; then
            failed_test_list="$test_num"
        else
            failed_test_list="$failed_test_list, $test_num"
        fi
    fi
done

# Generate a summary
echo "Test Results Summary" > side_by_side.txt
echo "====================" >> side_by_side.txt
echo "" >> side_by_side.txt

# Test-level results
echo "TEST VECTOR MATCHING:" >> side_by_side.txt
echo "Total test vectors: $total_tests" >> side_by_side.txt
echo "Matching test vectors: $matching_tests" >> side_by_side.txt
echo "Failed test vectors: $failed_tests" >> side_by_side.txt

if [ "$matching_tests" -eq "$total_tests" ]; then
    echo "✓ ALL TESTS PASSED - Rust implementation matches C reference exactly!" >> side_by_side.txt
else
    echo "✗ SOME TESTS FAILED - Rust implementation differs from C reference" >> side_by_side.txt
    echo "" >> side_by_side.txt
    echo "Failed test IDs: $failed_test_list" >> side_by_side.txt
fi

echo "" >> side_by_side.txt

# Calculate value-level statistics
total_comparisons=0
total_differences=0
max_diff=0
max_percent=0

while IFS=, read -r test_id c_val rust_val diff percent; do
    if [ "$test_id" = "test_id" ]; then
        continue
    fi
    
    ((total_comparisons++))
    
    if [ "$diff" != "0" ]; then
        ((total_differences++))
        
        # Update max difference if current diff is larger
        abs_diff=${diff#-}
        if [ "$abs_diff" -gt "$max_diff" ]; then
            max_diff=$abs_diff
        fi
        
        # Update max percentage if applicable and larger
        if [ "$percent" != "N/A" ]; then
            abs_percent=${percent#-}
            if [ "${abs_percent%.*}" -gt "${max_percent%.*}" ]; then
                max_percent=$abs_percent
            fi
        fi
    fi
done < comparison.csv

echo "VALUE-LEVEL STATISTICS:" >> side_by_side.txt
echo "Total value comparisons: $total_comparisons" >> side_by_side.txt
echo "Values with differences: $total_differences" >> side_by_side.txt
echo "Maximum absolute difference: $max_diff" >> side_by_side.txt
echo "Maximum percentage difference: $max_percent%" >> side_by_side.txt

# Clean up temporary files
rm -f /tmp/test_differences_* 2>/dev/null || true

# Console output summary
echo ""
echo "=========================================="
echo "           TEST RESULTS SUMMARY"
echo "=========================================="
echo "Total test vectors: $total_tests"
echo "Matching test vectors: $matching_tests"
echo "Failed test vectors: $failed_tests"
echo ""

if [ "$matching_tests" -eq "$total_tests" ]; then
    echo "✓ ALL TESTS PASSED"
    echo "  Rust implementation matches C reference exactly!"
else
    echo "✗ SOME TESTS FAILED"
    echo "  Rust implementation differs from C reference"
    echo "  Failed tests: $failed_test_list"
    echo "  Check side_by_side.txt for details"
fi

echo ""
echo "Detailed results saved to:"
echo "  - comparison.csv (value-by-value comparison)"
echo "  - side_by_side.txt (summary and statistics)"
echo "==========================================" 