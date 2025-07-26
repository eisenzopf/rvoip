#!/bin/bash

set -e

TEST_DIR=$(dirname "$0")
cd "$TEST_DIR"

echo "Building C test..."
make clean > /dev/null
make > /dev/null

echo "Running C test..."
./c_test > c_output.csv

echo "Running Rust test..."
cargo test --quiet --test perceptual_weighting -- --nocapture | grep -v 'Running' > rust_output.csv

echo "Comparing outputs and generating summary..."

# Create comparison CSV file
echo "test_id,parameter,c_value,rust_value,match" > comparison.csv
echo "" >> comparison.csv

total_matches=0
total_params=0

# Get headers from the first line of the C output
c_header=$(head -n 1 c_output.csv)
IFS=',' read -ra params <<< "$c_header"

# Read files line by line
exec 3< c_output.csv
exec 4< rust_output.csv

# Skip header lines
read -r <&3
read -r <&4

while read -r c_line <&3 && read -r r_line <&4; do
    # Split lines into arrays
    IFS=',' read -ra c_fields <<< "$c_line"
    IFS=',' read -ra r_fields <<< "$r_line"

    test_id=${c_fields[0]}
    echo "Test $test_id" >> comparison.csv

    # Compare all parameter fields (from index 1 onwards)
    for i in $(seq 1 $((${#params[@]} - 1))); do
        param_name=${params[$i]}
        c_val=${c_fields[$i]}
        r_val=${r_fields[$i]}
        match_char="✓"
        if [ "$c_val" != "$r_val" ]; then
            match_char="✗"
        else
            total_matches=$((total_matches + 1))
        fi
        total_params=$((total_params + 1))
        echo "$test_id,$param_name,$c_val,$r_val,$match_char" >> comparison.csv
    done
    echo "" >> comparison.csv
done

exec 3<&-
exec 4<&-

echo "SUMMARY" >> comparison.csv
echo "total_matches,$total_matches" >> comparison.csv
echo "total_parameters,$total_params" >> comparison.csv
if [ "$total_params" -gt 0 ]; then
    pass_rate=$((total_matches * 100 / total_params))
    echo "pass_rate,${pass_rate}%" >> comparison.csv
fi

echo "Comparison complete. Results in comparison.csv"

if [ "$total_matches" -eq "$total_params" ] && [ "$total_params" -gt 0 ]; then
    echo "SUCCESS: All outputs match!"
    exit 0
else
    echo "FAILURE: Outputs differ."
    cat comparison.csv
    exit 1
fi

