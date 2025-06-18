#!/bin/bash
# Quick compile test to verify everything builds

echo "Testing compilation of E2E test components..."

cd ../..

echo "1. Building server..."
cargo build --example e2e_test_server || exit 1

echo "2. Building agent client..."
cargo build --example e2e_test_agent || exit 1

echo "✅ All components compile successfully!"
echo ""
echo "Run ./run_e2e_test.sh to execute the full test" 