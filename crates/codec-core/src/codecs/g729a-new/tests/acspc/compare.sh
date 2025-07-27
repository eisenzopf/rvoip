#!/bin/bash

set -e

# This script is intended to be run from the g729a-new directory
# (crates/codec-core/src/codecs/g729a-new)

# Compile C test
make -C tests/acspc clean
make -C tests/acspc c_test

# Generate test vectors if they don't exist
if [ ! -f tests/acspc/test_inputs.csv ]; then
    make -C tests/acspc generate_test_vectors
    cd tests/acspc && ./generate_test_vectors && cd ../..
fi

# Run C implementation and save output
./tests/acspc/c_test

# Convert c_output.txt to CSV format with test indices
awk 'BEGIN{print "test_id,pitch_delay,pit_frac"} {split($0,a,","); print NR-1 "," a[1] "," a[2]}' tests/acspc/c_output.txt > tests/acspc/c_output.csv

# Run Rust test and capture output (only lines with at least one digit)
cargo test --test adaptive_codebook_search test_adaptive_codebook_search_from_csv -- --nocapture 2>/dev/null | grep "^[0-9-].*,[0-9-].*$" > tests/acspc/rust_raw_output.txt

# Convert Rust output to CSV format with test indices
awk 'BEGIN{print "test_id,pitch_delay,pit_frac"} {split($0,a,","); print NR-1 "," a[1] "," a[2]}' tests/acspc/rust_raw_output.txt > tests/acspc/rust_output.csv

# Create comparison CSV file
echo "ADAPTIVE CODEBOOK SEARCH COMPARISON" > tests/acspc/comparison.csv
echo "==================================" >> tests/acspc/comparison.csv
echo "" >> tests/acspc/comparison.csv
echo "Test ID,C Pitch Delay,C Pit Frac,Rust Pitch Delay,Rust Pit Frac,Pitch Match,Frac Match" >> tests/acspc/comparison.csv

# Process each test case
total_tests=0
matching_pitch=0
matching_frac=0
matching_both=0

while IFS=',' read -r test_id c_pitch_delay c_pit_frac; do
    # Skip header
    if [ "$test_id" = "test_id" ]; then
        continue
    fi
    
    # Get corresponding Rust values
    rust_line=$(grep "^$test_id," tests/acspc/rust_output.csv || echo "")
    if [ -n "$rust_line" ]; then
        rust_pitch_delay=$(echo $rust_line | cut -d',' -f2)
        rust_pit_frac=$(echo $rust_line | cut -d',' -f3)
    else
        rust_pitch_delay=""
        rust_pit_frac=""
    fi
    
    # Compare values
    if [ "$c_pitch_delay" = "$rust_pitch_delay" ]; then
        pitch_match="✓"
        ((matching_pitch++))
    else
        pitch_match="✗"
    fi
    
    if [ "$c_pit_frac" = "$rust_pit_frac" ]; then
        frac_match="✓"
        ((matching_frac++))
    else
        frac_match="✗"
    fi
    
    if [ "$pitch_match" = "✓" ] && [ "$frac_match" = "✓" ]; then
        ((matching_both++))
    fi
    
    echo "$test_id,$c_pitch_delay,$c_pit_frac,$rust_pitch_delay,$rust_pit_frac,$pitch_match,$frac_match" >> tests/acspc/comparison.csv
    ((total_tests++))
done < tests/acspc/c_output.csv

# Add summary
echo "" >> tests/acspc/comparison.csv
echo "SUMMARY" >> tests/acspc/comparison.csv
echo "=======" >> tests/acspc/comparison.csv
echo "Total Tests: $total_tests" >> tests/acspc/comparison.csv
echo "Matching Pitch Delay: $matching_pitch" >> tests/acspc/comparison.csv
echo "Matching Frac: $matching_frac" >> tests/acspc/comparison.csv
echo "Matching Both: $matching_both" >> tests/acspc/comparison.csv
pitch_match_percentage=$(echo "scale=1; $matching_pitch * 100 / $total_tests" | bc)
frac_match_percentage=$(echo "scale=1; $matching_frac * 100 / $total_tests" | bc)
both_match_percentage=$(echo "scale=1; $matching_both * 100 / $total_tests" | bc)
echo "Pitch Match Rate: ${pitch_match_percentage}%" >> tests/acspc/comparison.csv
echo "Frac Match Rate: ${frac_match_percentage}%" >> tests/acspc/comparison.csv
echo "Both Match Rate: ${both_match_percentage}%" >> tests/acspc/comparison.csv

# Create a readable side-by-side view
{
    echo "ADAPTIVE CODEBOOK SEARCH COMPARISON"
    echo "=================================="
    echo ""
    printf "%-8s %-12s %-10s %-12s %-10s %-6s %-6s\n" "Test ID" "C Pitch" "C Frac" "Rust Pitch" "Rust Frac" "P.Match" "F.Match"
    echo "------------------------------------------------------------------------"
    
    # Show test results - stop when we hit empty line or SUMMARY
    tail -n +5 tests/acspc/comparison.csv | while IFS=',' read -r test_id c_pitch c_frac rust_pitch rust_frac pmatch fmatch; do
        # Stop when we hit empty line or SUMMARY
        if [ -z "$test_id" ] || [ "$test_id" = "SUMMARY" ]; then
            break
        fi
        printf "%-8s %-12s %-10s %-12s %-10s %-6s %-6s\n" "$test_id" "$c_pitch" "$c_frac" "$rust_pitch" "$rust_frac" "$pmatch" "$fmatch"
    done
    
    echo ""
    tail -n 10 tests/acspc/comparison.csv
} > tests/acspc/side_by_side.txt

# Display results
cat tests/acspc/side_by_side.txt

# Clean up intermediate files
rm -f tests/acspc/rust_raw_output.txt

# Exit with appropriate status
if [ $matching_both -eq $total_tests ]; then
    exit 0
else
    exit 1
fi 