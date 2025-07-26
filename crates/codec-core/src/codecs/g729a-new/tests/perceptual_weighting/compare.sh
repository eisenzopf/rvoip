#!/bin/bash

set -e

# This script is intended to be run from the g729a-new directory
# (crates/codec-core/src/codecs/g729a-new)

# Compile C test
make -C tests/perceptual_weighting clean
make -C tests/perceptual_weighting

# Run C implementation and save output
./tests/perceptual_weighting/c_test > tests/perceptual_weighting/c_output.csv

# Run Rust test and capture output
cargo test --test perceptual_weighting test_perceptual_weighting_from_csv -- --nocapture 2>&1 | grep -v "running\|test result\|warning" | grep "^[0-9]" > tests/perceptual_weighting/rust_output.csv

# Function to extract data rows (skip header)
extract_data() {
    if [ -f "$1" ]; then
        if [ -s "$1" ]; then
            tail -n +2 "$1"
        else
            echo "Warning: File $1 is empty"
            return 1
        fi
    else
        echo "Warning: File $1 does not exist"
        return 1
    fi
}

# Sort both outputs (excluding headers) and compare
extract_data tests/perceptual_weighting/c_output.csv | sort > tests/perceptual_weighting/c_sorted.csv
extract_data tests/perceptual_weighting/rust_output.csv | sort > tests/perceptual_weighting/rust_sorted.csv

# Create comparison CSV file
echo "SIDE-BY-SIDE COMPARISON" > tests/perceptual_weighting/comparison.csv
echo "=======================" >> tests/perceptual_weighting/comparison.csv
echo "" >> tests/perceptual_weighting/comparison.csv

# Process each test case
while IFS= read -r c_line; do
    test_id=$(echo "$c_line" | cut -d',' -f1)
    rust_line=$(grep "^$test_id," tests/perceptual_weighting/rust_sorted.csv 2>/dev/null || echo "")
    
    echo "Test $test_id:" >> tests/perceptual_weighting/comparison.csv
    echo "Parameter,C Value,Rust Value,Match" >> tests/perceptual_weighting/comparison.csv
    
    IFS=',' read -ra C_VALS <<< "$c_line"
    if [ -n "$rust_line" ]; then
        IFS=',' read -ra RUST_VALS <<< "$rust_line"
    else
        RUST_VALS=()
    fi
    
    # Initialize match counters
    total_matches=0
    total_params=22  # 11 p coefficients + 11 f coefficients
    
    # Compare p coefficients
    for i in {0..10}; do
        c_val=${C_VALS[$((i+1))]}
        rust_val=${RUST_VALS[$((i+1))]-}
        match="✗"
        if [ "$c_val" = "$rust_val" ]; then
            match="✓"
            ((total_matches++))
        fi
        echo "p$i,$c_val,$rust_val,$match" >> tests/perceptual_weighting/comparison.csv
    done
    
    # Compare f coefficients
    for i in {0..10}; do
        c_val=${C_VALS[$((i+12))]}
        rust_val=${RUST_VALS[$((i+12))]-}
        match="✗"
        if [ "$c_val" = "$rust_val" ]; then
            match="✓"
            ((total_matches++))
        fi
        echo "f$i,$c_val,$rust_val,$match" >> tests/perceptual_weighting/comparison.csv
    done
    
    # Add match summary for this test
    echo "Match Summary: $total_matches/$total_params parameters match" >> tests/perceptual_weighting/comparison.csv
    echo "" >> tests/perceptual_weighting/comparison.csv
done < tests/perceptual_weighting/c_sorted.csv

# Create a readable side-by-side view
{
    echo "SIDE-BY-SIDE COMPARISON (First 3 Tests)"
    echo "======================================="
    echo ""
    
    head -n 70 tests/perceptual_weighting/comparison.csv | while IFS= read -r line; do
        if [[ $line == Test* ]]; then
            echo -e "\n$line"
        elif [[ $line == Parameter* ]]; then
            printf "%-10s %10s %10s   %s\n" "Parameter" "C Value" "Rust Value" "Match"
            echo "----------------------------------------"
        elif [[ $line == Match* ]]; then
            echo -e "\n$line"
        elif [[ $line =~ ^[pf][0-9]+ ]]; then
            IFS=',' read -r param c_val rust_val match <<< "$line"
            printf "%-10s %10s %10s   %s\n" "$param" "$c_val" "$rust_val" "$match"
        fi
    done
} > tests/perceptual_weighting/side_by_side.txt

# Calculate overall statistics
total_tests=$(grep -c "^Test" tests/perceptual_weighting/comparison.csv)
perfect_matches=$(grep -c "Match Summary: 22/22" tests/perceptual_weighting/comparison.csv)
{
    echo ""
    echo "OVERALL SUMMARY"
    echo "==============="
    echo "Total Tests: $total_tests"
    echo "Perfect Matches: $perfect_matches"
    if [ $perfect_matches -eq $total_tests ]; then
        echo "✓ All tests match perfectly!"
    else
        echo "✗ Some tests have mismatches. Check comparison.csv for details."
    fi
} | tee -a tests/perceptual_weighting/side_by_side.txt

# Display the side-by-side view
cat tests/perceptual_weighting/side_by_side.txt

# Clean up intermediate files
rm -f tests/perceptual_weighting/c_sorted.csv tests/perceptual_weighting/rust_sorted.csv

# Exit with appropriate status
if [ $perfect_matches -eq $total_tests ]; then
    exit 0
else
    exit 1
fi

