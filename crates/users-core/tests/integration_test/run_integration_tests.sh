#!/bin/bash

# Integration test runner for users-core rate limiting tests
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"

echo "🧪 Users-Core Integration Tests"
echo "================================"

# Parse command line arguments
FULL_TESTS=false
VERBOSE=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --full)
            FULL_TESTS=true
            shift
            ;;
        --verbose|-v)
            VERBOSE=true
            shift
            ;;
        --help|-h)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --full      Run extended tests (takes longer)"
            echo "  --verbose   Show detailed output"
            echo "  --help      Show this help message"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            echo "Run with --help for usage information"
            exit 1
            ;;
    esac
done

# Set up environment
export RUST_BACKTRACE=1

if [ "$VERBOSE" = true ]; then
    export RUST_LOG=debug,users_core=trace
else
    export RUST_LOG=info
fi

# Change to users-core directory
cd "$PROJECT_ROOT/crates/users-core"

echo "📍 Working directory: $(pwd)"
echo ""

# Clean up any previous test databases
rm -f tests/integration_test/*.db 2>/dev/null || true

# Run the integration tests as a binary
echo "🚀 Running integration tests..."
echo ""

# Change to integration test directory
cd tests/integration_test

# Build the integration test binary
echo "🔨 Building integration test binary..."
cargo build --release

if [ $? -ne 0 ]; then
    echo "❌ Failed to build integration tests!"
    exit 1
fi

# Run the tests
if [ "$FULL_TESTS" = true ]; then
    echo "📋 Running ALL tests (including extended scenarios)..."
    if [ "$VERBOSE" = true ]; then
        RUST_LOG=$RUST_LOG ./target/release/integration_test --full
    else
        ./target/release/integration_test --full
    fi
else
    echo "📋 Running standard tests..."
    if [ "$VERBOSE" = true ]; then
        RUST_LOG=$RUST_LOG ./target/release/integration_test
    else
        ./target/release/integration_test
    fi
fi

TEST_EXIT_CODE=$?

echo ""
echo "================================"

if [ $TEST_EXIT_CODE -eq 0 ]; then
    echo "✅ All tests passed!"
else
    echo "❌ Some tests failed!"
fi

echo ""
echo "💡 Tips:"
echo "  - Run with --verbose to see detailed logs"
echo "  - Run with --full to include extended test scenarios"
echo "  - Check test output above for specific failures"

exit $TEST_EXIT_CODE
