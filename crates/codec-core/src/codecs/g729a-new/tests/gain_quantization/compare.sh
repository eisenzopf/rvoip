#!/bin/bash

set -e

# This script is intended to be run from the g729a-new directory
# (crates/codec-core/src/codecs/g729a-new)

# Navigate to test directory
cd tests/gain_quantization

# Compile C test
make clean
make c_test

# Generate test vectors if they don't exist
if [ ! -f test_inputs.csv ]; then
    make generate_test_vectors
    ./generate_test_vectors
fi

# Run C implementation and save output
./c_test

# Convert c_output.txt to CSV format with test indices
awk 'BEGIN{print "test_id,index,gain_pit,gain_cod"} {split($0,a,","); print NR-1 "," a[1] "," a[2] "," a[3]}' c_output.txt > c_output.csv

# Navigate back to run Rust test
cd ../..

# Run Rust test and capture output (only lines with at least one digit)
cargo test --test gain_quantization test_gain_quantization_from_csv -- --nocapture 2>/dev/null | grep "^[0-9-].*,[0-9-].*,[0-9-].*$" > tests/gain_quantization/rust_raw_output.txt

# Navigate back to test directory
cd tests/gain_quantization

# Convert Rust output to CSV format with test indices
awk 'BEGIN{print "test_id,index,gain_pit,gain_cod"} {split($0,a,","); print NR-1 "," a[1] "," a[2] "," a[3]}' rust_raw_output.txt > rust_output.csv

# Create comparison CSV file
echo "GAIN QUANTIZATION COMPARISON" > comparison.csv
echo "==============================" >> comparison.csv
echo "" >> comparison.csv
echo "Test ID,C Index,C Gain Pit,C Gain Cod,Rust Index,Rust Gain Pit,Rust Gain Cod,Index Match,Gain Pit Match,Gain Cod Match" >> comparison.csv

# Process each test case
total_tests=0
matching_index=0
matching_gain_pit=0
matching_gain_cod=0
matching_all=0

while IFS=',' read -r test_id c_index c_gain_pit c_gain_cod; do
    # Skip header
    if [ "$test_id" = "test_id" ]; then
        continue
    fi
    
    # Get corresponding Rust values
    rust_line=$(grep "^$test_id," rust_output.csv || echo "")
    if [ -n "$rust_line" ]; then
        rust_index=$(echo $rust_line | cut -d',' -f2)
        rust_gain_pit=$(echo $rust_line | cut -d',' -f3)
        rust_gain_cod=$(echo $rust_line | cut -d',' -f4)
    else
        rust_index=""
        rust_gain_pit=""
        rust_gain_cod=""
    fi
    
    # Compare values
    if [ "$c_index" = "$rust_index" ]; then
        index_match="✓"
        ((matching_index++))
    else
        index_match="✗"
    fi
    
    if [ "$c_gain_pit" = "$rust_gain_pit" ]; then
        gain_pit_match="✓"
        ((matching_gain_pit++))
    else
        gain_pit_match="✗"
    fi
    
    if [ "$c_gain_cod" = "$rust_gain_cod" ]; then
        gain_cod_match="✓"
        ((matching_gain_cod++))
    else
        gain_cod_match="✗"
    fi
    
    if [ "$index_match" = "✓" ] && [ "$gain_pit_match" = "✓" ] && [ "$gain_cod_match" = "✓" ]; then
        ((matching_all++))
    fi
    
    echo "$test_id,$c_index,$c_gain_pit,$c_gain_cod,$rust_index,$rust_gain_pit,$rust_gain_cod,$index_match,$gain_pit_match,$gain_cod_match" >> comparison.csv
    ((total_tests++))
done < c_output.csv

# Add summary
echo "" >> comparison.csv
echo "SUMMARY" >> comparison.csv
echo "=======" >> comparison.csv
echo "Total Tests: $total_tests" >> comparison.csv
echo "Matching Index: $matching_index" >> comparison.csv
echo "Matching Gain Pit: $matching_gain_pit" >> comparison.csv
echo "Matching Gain Cod: $matching_gain_cod" >> comparison.csv
echo "Matching All: $matching_all" >> comparison.csv
index_match_percentage=$(echo "scale=1; $matching_index * 100 / $total_tests" | bc)
gain_pit_match_percentage=$(echo "scale=1; $matching_gain_pit * 100 / $total_tests" | bc)
gain_cod_match_percentage=$(echo "scale=1; $matching_gain_cod * 100 / $total_tests" | bc)
all_match_percentage=$(echo "scale=1; $matching_all * 100 / $total_tests" | bc)
echo "Index Match Rate: ${index_match_percentage}%" >> comparison.csv
echo "Gain Pit Match Rate: ${gain_pit_match_percentage}%" >> comparison.csv
echo "Gain Cod Match Rate: ${gain_cod_match_percentage}%" >> comparison.csv
echo "All Match Rate: ${all_match_percentage}%" >> comparison.csv

# Create a readable side-by-side view
{
    echo "GAIN QUANTIZATION COMPARISON"
    echo "=========================="
    echo ""
    printf "%-8s %-8s %-10s %-10s %-8s %-10s %-10s %-6s %-6s %-6s\n" "Test ID" "C Index" "C G.Pit" "C G.Cod" "R Index" "R G.Pit" "R G.Cod" "I.Match" "P.Match" "C.Match"
    echo "-------------------------------------------------------------------------------------------------"
    
    # Show test results - stop when we hit empty line or SUMMARY
    tail -n +5 comparison.csv | while IFS=',' read -r test_id c_index c_gain_pit c_gain_cod rust_index rust_gain_pit rust_gain_cod imatch pmatch cmatch; do
        # Stop when we hit empty line or SUMMARY
        if [ -z "$test_id" ] || [ "$test_id" = "SUMMARY" ]; then
            break
        fi
        printf "%-8s %-8s %-10s %-10s %-8s %-10s %-10s %-6s %-6s %-6s\n" "$test_id" "$c_index" "$c_gain_pit" "$c_gain_cod" "$rust_index" "$rust_gain_pit" "$rust_gain_cod" "$imatch" "$pmatch" "$cmatch"
    done
    
    echo ""
    tail -n 10 comparison.csv
} > side_by_side.txt

# Display results
cat side_by_side.txt

# Clean up intermediate files
rm -f rust_raw_output.txt

# Exit with appropriate status
if [ $matching_all -eq $total_tests ]; then
    exit 0
else
    exit 1
fi 