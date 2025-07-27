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

# Process the output files line by line
while IFS=, read -r test_id c_rest && IFS=, read -r rust_id rust_rest <&3; do
    if [ "$test_id" = "test_id" ]; then
        continue  # Skip header line
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
        
        # Calculate percentage difference if c_val is not zero
        if [ "$c_val" -ne 0 ]; then
            percent_diff=$(echo "scale=2; ($diff * 100) / $c_val" | bc)
        else
            percent_diff="N/A"
        fi
        
        echo "$test_id,$c_val,$rust_val,$diff,$percent_diff" >> comparison.csv
    done
done < c_output.csv 3< rust_output.csv

# Generate a summary
echo "Comparison Summary" > side_by_side.txt
echo "==================" >> side_by_side.txt
echo "" >> side_by_side.txt

# Calculate statistics
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

echo "Total comparisons: $total_comparisons" >> side_by_side.txt
echo "Values with differences: $total_differences" >> side_by_side.txt
echo "Maximum absolute difference: $max_diff" >> side_by_side.txt
echo "Maximum percentage difference: $max_percent%" >> side_by_side.txt

echo "Done! Check comparison.csv and side_by_side.txt for results." 