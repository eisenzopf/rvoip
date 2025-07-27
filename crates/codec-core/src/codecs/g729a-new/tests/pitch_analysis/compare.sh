#!/bin/bash

set -e

# This script is intended to be run from the g729a-new directory
# (crates/codec-core/src/codecs/g729a-new)

# Compile C test
make -C tests/pitch_analysis clean
make -C tests/pitch_analysis c_test

# Generate test vectors if they don't exist
if [ ! -f tests/pitch_analysis/test_inputs.csv ]; then
    make -C tests/pitch_analysis generate_test_vectors
    cd tests/pitch_analysis && ./generate_test_vectors && cd ../..
fi

# Run C implementation and save output
./tests/pitch_analysis/c_test

# Convert c_output.txt to CSV format with test indices
awk 'BEGIN{print "test_id,pitch_lag"} {print NR-1 "," $0}' tests/pitch_analysis/c_output.txt > tests/pitch_analysis/c_output.csv

# Run Rust test and capture output
cargo test --test pitch_analysis test_pitch_analysis_from_csv -- --nocapture 2>&1 | grep "^[0-9]" > tests/pitch_analysis/rust_raw_output.txt

# Convert Rust output to CSV format with test indices
awk 'BEGIN{print "test_id,pitch_lag"} {print NR-1 "," $0}' tests/pitch_analysis/rust_raw_output.txt > tests/pitch_analysis/rust_output.csv

# Create comparison CSV file
echo "PITCH ANALYSIS COMPARISON" > tests/pitch_analysis/comparison.csv
echo "========================" >> tests/pitch_analysis/comparison.csv
echo "" >> tests/pitch_analysis/comparison.csv
echo "Test ID,C Pitch Lag,Rust Pitch Lag,Match" >> tests/pitch_analysis/comparison.csv

# Process each test case
total_tests=0
matching_tests=0

while IFS=',' read -r test_id c_pitch_lag; do
    # Skip header
    if [ "$test_id" = "test_id" ]; then
        continue
    fi
    
    # Get corresponding Rust value
    rust_pitch_lag=$(grep "^$test_id," tests/pitch_analysis/rust_output.csv | cut -d',' -f2 || echo "")
    
    # Compare values
    if [ "$c_pitch_lag" = "$rust_pitch_lag" ]; then
        match="✓"
        ((matching_tests++))
    else
        match="✗"
    fi
    
    echo "$test_id,$c_pitch_lag,$rust_pitch_lag,$match" >> tests/pitch_analysis/comparison.csv
    ((total_tests++))
done < tests/pitch_analysis/c_output.csv

# Add summary
echo "" >> tests/pitch_analysis/comparison.csv
echo "SUMMARY" >> tests/pitch_analysis/comparison.csv
echo "=======" >> tests/pitch_analysis/comparison.csv
echo "Total Tests: $total_tests" >> tests/pitch_analysis/comparison.csv
echo "Matching: $matching_tests" >> tests/pitch_analysis/comparison.csv
echo "Mismatches: $((total_tests - matching_tests))" >> tests/pitch_analysis/comparison.csv
match_percentage=$(echo "scale=1; $matching_tests * 100 / $total_tests" | bc)
echo "Match Rate: ${match_percentage}%" >> tests/pitch_analysis/comparison.csv

# Create a readable side-by-side view
{
    echo "PITCH ANALYSIS COMPARISON"
    echo "========================"
    echo ""
    printf "%-10s %-15s %-15s %s\n" "Test ID" "C Pitch Lag" "Rust Pitch Lag" "Match"
    echo "----------------------------------------------------"
    
    # Show test results, handling case where there might be fewer than 7 lines
    if [ $total_tests -gt 7 ]; then
        tail -n +5 tests/pitch_analysis/comparison.csv | head -n -7 | while IFS=',' read -r test_id c_val rust_val match; do
            printf "%-10s %-15s %-15s %s\n" "$test_id" "$c_val" "$rust_val" "$match"
        done
    else
        tail -n +5 tests/pitch_analysis/comparison.csv | while IFS=',' read -r test_id c_val rust_val match; do
            if [ -n "$test_id" ] && [ "$test_id" != "SUMMARY" ]; then
                printf "%-10s %-15s %-15s %s\n" "$test_id" "$c_val" "$rust_val" "$match"
            fi
        done
    fi
    
    echo ""
    tail -n 7 tests/pitch_analysis/comparison.csv
} > tests/pitch_analysis/side_by_side.txt

# Display results
cat tests/pitch_analysis/side_by_side.txt

# Clean up intermediate files
rm -f tests/pitch_analysis/rust_raw_output.txt

# Exit with appropriate status
if [ $matching_tests -eq $total_tests ]; then
    exit 0
else
    exit 1
fi 