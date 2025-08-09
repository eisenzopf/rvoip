#!/bin/bash
# Run all tests for sip-client including those requiring test-audio feature

echo "Running all sip-client tests (including full_roundtrip with test-audio feature)..."
cargo test --features test-audio "$@"