#!/bin/bash

# Run all users-core examples
# This script runs each example in sequence, cleaning up databases between runs

set -e  # Exit on error

# Colors for output
GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Function to print headers
print_header() {
    echo
    echo -e "${BLUE}════════════════════════════════════════════════════════════════${NC}"
    echo -e "${BLUE}   $1${NC}"
    echo -e "${BLUE}════════════════════════════════════════════════════════════════${NC}"
    echo
}

# Function to print success
print_success() {
    echo -e "${GREEN}✅ $1${NC}"
}

# Function to print error
print_error() {
    echo -e "${RED}❌ $1${NC}"
}

# Function to print info
print_info() {
    echo -e "${YELLOW}ℹ️  $1${NC}"
}

# Function to clean up database files
cleanup_dbs() {
    print_info "Cleaning up database files..."
    rm -f *.db
    rm -f examples/*.db
    rm -f examples/rest_api_demo/*.db
}

# Function to run an example
run_example() {
    local example_name=$1
    local description=$2
    
    print_header "Running: $example_name - $description"
    
    # Clean up any existing databases
    cleanup_dbs
    
    # Run the example
    if cargo run --example "$example_name" --quiet; then
        print_success "$example_name completed successfully!"
    else
        print_error "$example_name failed!"
        return 1
    fi
    
    # Brief pause between examples
    sleep 1
}

# Main execution
print_header "Users-Core Examples Runner"
echo "This script will run all examples in sequence."
echo "Each example will create and clean up its own database."
echo

# Change to the users-core directory
cd "$(dirname "$0")/.."

# Build all examples first
print_info "Building all examples..."
if cargo build --examples --quiet; then
    print_success "All examples built successfully!"
else
    print_error "Failed to build examples!"
    exit 1
fi

# Run each example
echo
print_info "Starting example runs..."

# Keep track of results
declare -a passed=()
declare -a failed=()

# Run basic_usage example
if run_example "basic_usage" "Basic user management and authentication"; then
    passed+=("basic_usage")
else
    failed+=("basic_usage")
fi

# Run api_key_service example
if run_example "api_key_service" "API key creation and management"; then
    passed+=("api_key_service")
else
    failed+=("api_key_service")
fi

# Run sip_register_flow example
if run_example "sip_register_flow" "SIP REGISTER authentication flow"; then
    passed+=("sip_register_flow")
else
    failed+=("sip_register_flow")
fi

# Run token_validation example
if run_example "token_validation" "JWT token validation and introspection"; then
    passed+=("token_validation")
else
    failed+=("token_validation")
fi

# Run multi_device_presence example
if run_example "multi_device_presence" "Multi-device registration and presence"; then
    passed+=("multi_device_presence")
else
    failed+=("multi_device_presence")
fi

# Run session_core_v2_integration example
if run_example "session_core_v2_integration" "Complete session-core-v2 integration"; then
    passed+=("session_core_v2_integration")
else
    failed+=("session_core_v2_integration")
fi

# Final cleanup
cleanup_dbs

# Print summary
print_header "Summary"

echo -e "${GREEN}Passed (${#passed[@]}):${NC}"
for example in "${passed[@]}"; do
    echo -e "  ${GREEN}✓${NC} $example"
done

if [ ${#failed[@]} -gt 0 ]; then
    echo
    echo -e "${RED}Failed (${#failed[@]}):${NC}"
    for example in "${failed[@]}"; do
        echo -e "  ${RED}✗${NC} $example"
    done
fi

echo
if [ ${#failed[@]} -eq 0 ]; then
    print_success "All examples completed successfully! 🎉"
    exit 0
else
    print_error "Some examples failed. Please check the output above."
    exit 1
fi
