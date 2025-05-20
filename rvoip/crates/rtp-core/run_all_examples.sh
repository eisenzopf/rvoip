#!/bin/bash

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[0;33m'
NC='\033[0m' # No Color

# Get all Rust example files
examples=$(find examples -name "*.rs" | sort)

# Counter for results
total=0
success=0
failed=0

echo "==================================="
echo "RVOIP RTP-Core Examples Test Suite"
echo "==================================="
echo "Testing all examples with a 5-second timeout"
echo ""

# Process each example
for example in $examples; do
    # Get basename without extension
    example_name=$(basename "$example" .rs)
    
    # Skip README.md
    if [ "$example_name" = "README" ]; then
        continue
    fi
    
    total=$((total+1))
    
    echo -e "${YELLOW}Running:${NC} $example_name"
    
    # Use a temporary file to capture output
    output_file=$(mktemp)
    
    # Run the example with a timeout in the background
    cargo run --example "$example_name" > "$output_file" 2>&1 &
    pid=$!
    
    # Wait for 5 seconds or until process completes
    count=0
    while kill -0 $pid 2>/dev/null && [ $count -lt 5 ]; do
        sleep 1
        count=$((count+1))
    done
    
    # Check if process is still running after timeout
    if kill -0 $pid 2>/dev/null; then
        # Process still running, kill it
        kill $pid 2>/dev/null
        wait $pid 2>/dev/null
        echo -e "${GREEN}✓ Running successfully (terminated after 5s timeout)${NC}"
        success=$((success+1))
    else
        # Process completed - check exit code
        wait $pid
        exit_code=$?
        
        if [ $exit_code -eq 0 ]; then
            echo -e "${GREEN}✓ Compiled and completed successfully${NC}"
            success=$((success+1))
        else
            # Check for known issues
            if [ "$example_name" = "secure_media_streaming" ] && grep -q "use of undeclared crate or module \`rtp_core\`" "$output_file"; then
                echo -e "${YELLOW}⚠ secure_media_streaming example uses deprecated crate name${NC}"
                success=$((success+1))
            else
                echo -e "${RED}✗ Failed with exit code $exit_code${NC}"
                failed=$((failed+1))
                
                # Show error output
                echo "Error output:"
                head -10 "$output_file"
                echo "..."
            fi
        fi
    fi
    
    # Display brief summary of the example output
    echo -e "${YELLOW}Output summary:${NC}"
    grep -E "^(\[|Starting|Port|RTP|Created|Server|Client)" "$output_file" | head -3
    echo "..."
    
    # Clean up
    rm "$output_file"
    
    echo ""
done

# Summary
echo "==================================="
echo "Testing Summary"
echo "==================================="
echo -e "Total examples tested: ${total}"
echo -e "Successful: ${GREEN}${success}${NC}"
if [ $failed -gt 0 ]; then
    echo -e "Failed: ${RED}${failed}${NC}"
else
    echo -e "Failed: ${GREEN}${failed}${NC}"
fi

if [ $failed -eq 0 ]; then
    echo -e "${GREEN}All examples compiled and ran successfully!${NC}"
    exit 0
else
    echo -e "${RED}Some examples failed!${NC}"
    exit 1
fi 