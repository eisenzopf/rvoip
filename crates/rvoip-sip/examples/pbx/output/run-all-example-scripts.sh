#!/usr/bin/env bash
# Drive every rvoip-sip example with a run.sh (excluding pbx, which is
# covered by its own matrix runner). Sequential to avoid 5060-range port
# collisions between back-to-back peer demos.
set -u
DIR=/Users/jonathan/Developer/rvoip/crates/rvoip-sip/examples
OUT=/Users/jonathan/Developer/rvoip/crates/rvoip-sip/examples/pbx/output/example-scripts
mkdir -p "$OUT"
SUMMARY="$OUT/_summary.tsv"
: > "$SUMMARY"

scripts=(
  "$DIR"/regression/*/run.sh
  "$DIR"/stream_peer/*/run.sh
  "$DIR"/callback_peer/*/run.sh
  "$DIR"/unified/*/run.sh
)

pass=0; fail=0
for sh in "${scripts[@]}"; do
  rel=${sh#"$DIR/"}
  name=${rel%/run.sh}
  safe=${name//\//_}
  log="$OUT/$safe.log"
  echo "[run] $name"
  start=$(date +%s)
  if /usr/bin/env -i HOME="$HOME" PATH="$PATH" bash "$sh" > "$log" 2>&1; then
    elapsed=$(( $(date +%s) - start ))
    echo "  PASS (${elapsed}s)"
    printf '%s\tPASS\t%ds\n' "$name" "$elapsed" >> "$SUMMARY"
    pass=$((pass+1))
  else
    rc=$?
    elapsed=$(( $(date +%s) - start ))
    echo "  FAIL exit=$rc (${elapsed}s)  see $log"
    printf '%s\tFAIL_exit%d\t%ds\n' "$name" "$rc" "$elapsed" >> "$SUMMARY"
    fail=$((fail+1))
    tail -20 "$log" | sed 's/^/    | /'
  fi
done

echo
echo "==== Example-script summary ===="
echo "pass=$pass fail=$fail total=$((pass+fail))"
echo "summary file: $SUMMARY"
exit "$fail"
