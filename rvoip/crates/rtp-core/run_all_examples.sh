#!/bin/bash

echo "Running all examples in the rtp-core/examples directory"
echo "======================================================="
echo

# Find all example files, sort them, and run each one
find ./examples -name "*.rs" -type f | sort | while read example_file; do
  example_name=$(basename "$example_file" .rs)
  
  echo
  echo "======================================================="
  echo "Running example: $example_name"
  echo "======================================================="
  echo
  
  # Run the example and show all output
  cargo run --example "$example_name"
  
  echo
  echo "======================================================="
  echo "Finished running: $example_name"
  echo "======================================================="
  echo
  
  # Small pause between examples
  sleep 1
done

echo
echo "All examples completed!" 