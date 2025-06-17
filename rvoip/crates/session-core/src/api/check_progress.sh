#!/bin/bash
# Script to check progress on API improvements

echo "Session-Core API Improvements Progress"
echo "====================================="
echo

# Count total and completed tasks
TOTAL=$(grep -c "^- \[" API_IMPROVEMENTS.md)
COMPLETED=$(grep -c "^- \[x\]" API_IMPROVEMENTS.md)
REMAINING=$((TOTAL - COMPLETED))

echo "Total tasks: $TOTAL"
echo "Completed: $COMPLETED"
echo "Remaining: $REMAINING"
echo "Progress: $(( COMPLETED * 100 / TOTAL ))%"
echo

# Show tasks by phase
echo "Progress by Phase:"
echo "-----------------"

for phase in 1 2 3 4; do
    PHASE_TOTAL=$(sed -n "/^## Phase $phase/,/^## Phase/p" API_IMPROVEMENTS.md | grep -c "^- \[")
    PHASE_DONE=$(sed -n "/^## Phase $phase/,/^## Phase/p" API_IMPROVEMENTS.md | grep -c "^- \[x\]")
    echo "Phase $phase: $PHASE_DONE/$PHASE_TOTAL completed"
done

echo
echo "Next uncompleted tasks:"
echo "----------------------"
grep -n "^- \[ \]" API_IMPROVEMENTS.md | head -5 | while read -r line; do
    echo "$line"
done 