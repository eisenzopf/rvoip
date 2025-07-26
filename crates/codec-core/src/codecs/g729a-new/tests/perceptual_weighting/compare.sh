#!/bin/bash

# Change to the test directory
cd "$(dirname "$0")"

# Compile C test program
make clean && make

# Run C implementation and save output
./c_test > c_output.csv

# Run Rust implementation and save output
cd ../../
cargo test --test perceptual_weighting test_perceptual_weighting_from_csv --release -- --nocapture 2>&1 | grep -v "running\|test result\|warning" | grep "^[0-9]" > tests/perceptual_weighting/rust_output.csv
cd tests/perceptual_weighting

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
extract_data c_output.csv | sort > c_sorted.csv
extract_data rust_output.csv | sort > rust_sorted.csv

# Compare the sorted files
DIFF_OUTPUT=$(diff c_sorted.csv rust_sorted.csv)
DIFF_STATUS=$?

# Function to calculate differences for a single row
calculate_row_diff() {
    local c_line="$1"
    local rust_line="$2"
    
    IFS=',' read -ra C_VALS <<< "$c_line"
    IFS=',' read -ra RUST_VALS <<< "$rust_line"
    
    echo "Test ID: ${C_VALS[0]}"
    echo "Differences (C vs Rust):"
    
    # Compare p coefficients
    echo "P coefficients:"
    for i in {1..11}; do
        c_val=${C_VALS[$i]}
        rust_val=${RUST_VALS[$i]}
        if [ "$c_val" != "$rust_val" ]; then
            diff=$((rust_val - c_val))
            echo "  p$((i-1)): C=$c_val, Rust=$rust_val (diff=$diff)"
        fi
    done
    
    # Compare f coefficients
    echo "F coefficients:"
    for i in {12..22}; do
        c_val=${C_VALS[$i]}
        rust_val=${RUST_VALS[$i]}
        if [ "$c_val" != "$rust_val" ]; then
            diff=$((rust_val - c_val))
            echo "  f$((i-12)): C=$c_val, Rust=$rust_val (diff=$diff)"
        fi
    done
    echo "----------------------------------------"
}

# Function to generate side-by-side comparison
generate_comparison() {
    echo "SIDE-BY-SIDE COMPARISON" > comparison.csv
    echo "=======================" >> comparison.csv
    echo "" >> comparison.csv

    # Process each test case
    while IFS= read -r c_line; do
        test_id=$(echo "$c_line" | cut -d',' -f1)
        rust_line=$(grep "^$test_id," rust_sorted.csv 2>/dev/null || echo "")
        
        echo "Test $test_id:" >> comparison.csv
        echo "Parameter,C Value,Rust Value,Match" >> comparison.csv
        
        IFS=',' read -ra C_VALS <<< "$c_line"
        if [ -n "$rust_line" ]; then
            IFS=',' read -ra RUST_VALS <<< "$rust_line"
        else
            RUST_VALS=()
        fi
        
        # Compare p coefficients
        for i in {0..10}; do
            c_val=${C_VALS[$((i+1))]}
            rust_val=${RUST_VALS[$((i+1))]-}
            match="✗"
            if [ "$c_val" = "$rust_val" ]; then
                match="✓"
            fi
            echo "p$i,$c_val,$rust_val,$match" >> comparison.csv
        done
        
        # Compare f coefficients
        for i in {0..10}; do
            c_val=${C_VALS[$((i+12))]}
            rust_val=${RUST_VALS[$((i+12))]-}
            match="✗"
            if [ "$c_val" = "$rust_val" ]; then
                match="✓"
            fi
            echo "f$i,$c_val,$rust_val,$match" >> comparison.csv
        done
        
        echo "" >> comparison.csv
    done < c_sorted.csv
}

# Generate side-by-side comparison
generate_comparison

# If there are differences, analyze them
if [ $DIFF_STATUS -ne 0 ]; then
    echo "Found differences between C and Rust implementations!"
    echo "Detailed analysis:"
    echo "----------------------------------------"
    
    # Process each line from C output
    while IFS= read -r c_line; do
        test_id=$(echo "$c_line" | cut -d',' -f1)
        rust_line=$(grep "^$test_id," rust_sorted.csv 2>/dev/null || echo "")
        if [ -n "$rust_line" ]; then
            calculate_row_diff "$c_line" "$rust_line"
        else
            echo "No Rust output for test $test_id"
        fi
    done < c_sorted.csv
    
    # Save detailed comparison to file
    {
        echo "Detailed Comparison Report"
        echo "=========================="
        echo
        echo "Generated on: $(date)"
        echo
        echo "Raw differences:"
        echo "$DIFF_OUTPUT"
    } > comparison_output.txt
    
    echo "Detailed comparison saved to comparison_output.txt"
    echo "Side-by-side comparison saved to comparison.csv"
    exit 1
else
    echo "C and Rust implementations match exactly!"
    echo "Side-by-side comparison saved to comparison.csv"
    exit 0
fi

