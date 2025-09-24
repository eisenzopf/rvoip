#!/bin/bash
set -e

echo "🚀 Users-Core REST API Demo"
echo "=========================="
echo

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Get the directory of this script
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$SCRIPT_DIR/../.."

# Clean up any existing demo database
echo "🧹 Cleaning up previous demo data..."
rm -f examples/rest_api_demo/demo.db
rm -f examples/rest_api_demo/server.log

# Build the examples
echo "🔨 Building server and client..."
cargo build --example rest_api_demo_server
cargo build --example rest_api_demo_client --features client

# Start the server in the background
echo "🌐 Starting REST API server..."
cargo run --example rest_api_demo_server > examples/rest_api_demo/server.log 2>&1 &
SERVER_PID=$!

# Function to cleanup on exit
cleanup() {
    echo -e "\n${YELLOW}🛑 Shutting down server...${NC}"
    kill $SERVER_PID 2>/dev/null || true
    wait $SERVER_PID 2>/dev/null || true
}
trap cleanup EXIT

# Wait for server to start
echo "⏳ Waiting for server to start..."
for i in {1..10}; do
    if curl -s http://127.0.0.1:8082/health > /dev/null 2>&1; then
        echo -e "${GREEN}✅ Server is ready!${NC}"
        break
    fi
    if [ $i -eq 10 ]; then
        echo -e "${RED}❌ Server failed to start${NC}"
        echo "Server logs:"
        cat examples/rest_api_demo/server.log
        exit 1
    fi
    sleep 1
done

echo

# Run the client tests
echo "🧪 Running API tests..."
echo "------------------------"
if cargo run --example rest_api_demo_client --features client; then
    echo
    echo -e "${GREEN}✅ Demo completed successfully!${NC}"
    EXIT_CODE=0
else
    echo
    echo -e "${RED}❌ Demo failed${NC}"
    echo
    echo "Server logs:"
    echo "------------"
    tail -20 examples/rest_api_demo/server.log
    EXIT_CODE=1
fi

# Show some server logs
echo
echo "📋 Server activity log (last 10 lines):"
echo "---------------------------------------"
tail -10 examples/rest_api_demo/server.log | grep -E "(INFO|WARN|ERROR)" || echo "(No activity logged)"

exit $EXIT_CODE
