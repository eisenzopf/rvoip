#!/bin/bash

set -e

# This script is intended to be run from the g729a-new directory.

# Compile C test
make -C tests/lsp_quantizer clean
make -C tests/lsp_quantizer

# Run C test and capture output
./tests/lsp_quantizer/c_test > tests/lsp_quantizer/c_output.csv

# Run Rust test and capture output  
cargo test --test lsp_quantizer -- --nocapture | grep -E "^[0-9]+," > tests/lsp_quantizer/rust_output.csv || true

# Create comparison CSV file with transposed format
echo "Creating transposed comparison..."

# First, create a header row with test IDs
echo -n "output_param" > tests/lsp_quantizer/comparison.csv
for i in {1..10}; do
    echo -n ",test_${i}_C,test_${i}_Rust,test_${i}_match" >> tests/lsp_quantizer/comparison.csv
done
echo "" >> tests/lsp_quantizer/comparison.csv

# Function to extract a specific field from CSV
extract_field() {
    local file=$1
    local test_id=$2
    local field=$3
    grep "^${test_id}," "$file" | cut -d',' -f"$field" || echo "N/A"
}

# Output each parameter row
for param_idx in {1..12}; do
    case $param_idx in
        1) param_name="lsp_q0" ;;
        2) param_name="lsp_q1" ;;
        3) param_name="lsp_q2" ;;
        4) param_name="lsp_q3" ;;
        5) param_name="lsp_q4" ;;
        6) param_name="lsp_q5" ;;
        7) param_name="lsp_q6" ;;
        8) param_name="lsp_q7" ;;
        9) param_name="lsp_q8" ;;
        10) param_name="lsp_q9" ;;
        11) param_name="ana0" ;;
        12) param_name="ana1" ;;
    esac
    
    echo -n "$param_name" >> tests/lsp_quantizer/comparison.csv
    
    # Add field index offset (test_id is field 1, so lsp_q0 is field 2, etc.)
    field_idx=$((param_idx + 1))
    
    # For each test ID (display as 1-10, but file uses 0-9)
    for display_id in {1..10}; do
        # Map display test IDs (1-based) to file indices (0-based)
        file_idx=$((display_id - 1))
        
        c_val=$(extract_field tests/lsp_quantizer/c_output.csv "$file_idx" "$field_idx")
        rust_val=$(extract_field tests/lsp_quantizer/rust_output.csv "$file_idx" "$field_idx")
        
        if [ "$c_val" = "$rust_val" ]; then
            match="✓"
        else
            match="✗"
        fi
        
        echo -n ",$c_val,$rust_val,$match" >> tests/lsp_quantizer/comparison.csv
    done
    echo "" >> tests/lsp_quantizer/comparison.csv
done

# Create a summary section
echo "" >> tests/lsp_quantizer/comparison.csv
echo "SUMMARY" >> tests/lsp_quantizer/comparison.csv
echo -n "total_matches" >> tests/lsp_quantizer/comparison.csv
for display_id in {1..10}; do
    file_idx=$((display_id - 1))
    
    # Count matches for this test
    matches=0
    for field_idx in {2..13}; do
        c_val=$(extract_field tests/lsp_quantizer/c_output.csv "$file_idx" "$field_idx")
        rust_val=$(extract_field tests/lsp_quantizer/rust_output.csv "$file_idx" "$field_idx")
        if [ "$c_val" = "$rust_val" ]; then
            ((matches++))
        fi
    done
    
    echo -n ",$matches/12,," >> tests/lsp_quantizer/comparison.csv
done
echo "" >> tests/lsp_quantizer/comparison.csv

# Also create a compact side-by-side view for specific tests
echo "" > tests/lsp_quantizer/side_by_side.txt
echo "SIDE-BY-SIDE COMPARISON (First 3 tests)" >> tests/lsp_quantizer/side_by_side.txt
echo "========================================" >> tests/lsp_quantizer/side_by_side.txt

for display_id in {1..3}; do
    file_idx=$((display_id - 1))
    echo "" >> tests/lsp_quantizer/side_by_side.txt
    echo "Test $display_id:" >> tests/lsp_quantizer/side_by_side.txt
    echo "Parameter     C Value    Rust Value   Match" >> tests/lsp_quantizer/side_by_side.txt
    echo "---------   ---------   ----------   -----" >> tests/lsp_quantizer/side_by_side.txt
    
    for param_idx in {1..12}; do
        case $param_idx in
            1) param_name="lsp_q0" ;;
            2) param_name="lsp_q1" ;;
            3) param_name="lsp_q2" ;;
            4) param_name="lsp_q3" ;;
            5) param_name="lsp_q4" ;;
            6) param_name="lsp_q5" ;;
            7) param_name="lsp_q6" ;;
            8) param_name="lsp_q7" ;;
            9) param_name="lsp_q8" ;;
            10) param_name="lsp_q9" ;;
            11) param_name="ana0  " ;;
            12) param_name="ana1  " ;;
        esac
        
        field_idx=$((param_idx + 1))
        c_val=$(extract_field tests/lsp_quantizer/c_output.csv "$file_idx" "$field_idx")
        rust_val=$(extract_field tests/lsp_quantizer/rust_output.csv "$file_idx" "$field_idx")
        
        if [ "$c_val" = "$rust_val" ]; then
            match="✓"
        else
            match="✗"
        fi
        
        printf "%-9s %9s   %10s     %s\n" "$param_name" "$c_val" "$rust_val" "$match" >> tests/lsp_quantizer/side_by_side.txt
    done
done

echo "Comparison complete. Results in:"
echo "  - tests/lsp_quantizer/comparison.csv (transposed format)"
echo "  - tests/lsp_quantizer/side_by_side.txt (readable format)"

# Show the side-by-side view
cat tests/lsp_quantizer/side_by_side.txt

# Clean up intermediate files
rm -f tests/lsp_quantizer/c_output.csv tests/lsp_quantizer/rust_output.csv
