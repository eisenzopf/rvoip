#!/bin/bash

# Clean up any existing output
rm -rf examples/api_uac_uas/uac_output
rm -rf examples/api_uac_uas/uas_output

echo "Starting UAC/UAS API test..."
echo "==============================="

# Start UAS (server) in background
echo "Starting UAS (server) on port 5061..."
RUST_LOG=info cargo run --example uas &
UAS_PID=$!

# Give UAS time to start and be ready
sleep 3

# Start UAC (client)
echo "Starting UAC (client) on port 5060..."
RUST_LOG=info cargo run --example uac

# Wait for UAC to complete
UAC_EXIT=$?

# Give a moment for UAS to finish
sleep 2

# Kill UAS if still running
kill $UAS_PID 2>/dev/null

echo ""
echo "Test completed!"
echo "Check audio files in:"
echo "  - examples/api_uac_uas/uac_output/"
echo "  - examples/api_uac_uas/uas_output/"

exit $UAC_EXIT