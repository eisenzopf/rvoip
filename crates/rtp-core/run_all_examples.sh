#!/bin/bash

set -e  # Exit on any error in the main script, but we'll handle individual example failures

echo "Running all examples in the rtp-core/examples directory"
echo "======================================================="
echo

# Change to the rtp-core directory if we're not already there
if [[ ! -d "./examples" ]]; then
    echo "Error: examples directory not found. Make sure you're in the rtp-core directory."
    exit 1
fi

# Initialize counters
total_examples=0
successful_examples=0
failed_examples=0
failed_list=()

# Get list of all example files
example_files=($(find ./examples -name "*.rs" -type f | sort))
total_examples=${#example_files[@]}

echo "Found $total_examples example files to run"
echo

# Run each example
for example_file in "${example_files[@]}"; do
    example_name=$(basename "$example_file" .rs)
    
    echo
    echo "======================================================="
    echo "Running example [$((successful_examples + failed_examples + 1))/$total_examples]: $example_name"
    echo "======================================================="
    echo
    
    # Run the example and capture exit status
    if cargo run --example "$example_name"; then
        echo
        echo "✅ SUCCESS: $example_name completed successfully"
        ((successful_examples++))
    else
        echo
        echo "❌ FAILED: $example_name failed to run"
        ((failed_examples++))
        failed_list+=("$example_name")
    fi
    
    echo
    echo "======================================================="
    echo "Finished running: $example_name"
    echo "======================================================="
    echo
    
    # Small pause between examples
    sleep 1
done

echo
echo "======================================================="
echo "SUMMARY"
echo "======================================================="
echo "Total examples: $total_examples"
echo "Successful: $successful_examples"
echo "Failed: $failed_examples"
echo

if [[ $failed_examples -gt 0 ]]; then
    echo "Failed examples:"
    for failed_example in "${failed_list[@]}"; do
        echo "  - $failed_example"
    done
    echo
    echo "❌ Some examples failed. Check the output above for details."
    exit 1
else
    echo "🎉 All examples completed successfully!"
    exit 0
fi 