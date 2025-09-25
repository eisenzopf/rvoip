#!/bin/bash

# Run all users-core examples (CI/CD version)
# Minimal output version for automated testing

set -e  # Exit on error

# Clean up function
cleanup() {
    rm -f *.db examples/*.db examples/rest_api_demo/*.db 2>/dev/null || true
}

# Ensure cleanup on exit
trap cleanup EXIT

# Change to the users-core directory
cd "$(dirname "$0")/.."

echo "Building examples..."
cargo build --examples --quiet

echo "Running examples..."

# Array to store results
declare -a results=()

# Run each example
for example in basic_usage api_key_service sip_register_flow token_validation multi_device_presence session_core_v2_integration; do
    echo -n "  $example... "
    cleanup
    if cargo run --example "$example" --quiet > /dev/null 2>&1; then
        echo "✓"
        results+=("✓ $example")
    else
        echo "✗"
        results+=("✗ $example")
    fi
done

# Print results
echo
echo "Results:"
for result in "${results[@]}"; do
    echo "  $result"
done

# Check if any failed
if [[ "${results[*]}" =~ "✗" ]]; then
    exit 1
fi
