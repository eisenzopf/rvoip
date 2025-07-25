#!/bin/bash

echo "=== LSP Quantizer Output Comparison ==="
echo ""
echo "C Output (from c_output_new.csv):"
tail -n +2 c_output_new.csv | cut -d, -f2

echo ""
echo "Expected Rust output with fix:"
echo "32469"
echo "31454"
echo "27202"
echo "19509"
echo "10060"
echo "1268"
echo "-8913"
echo "-20140"
echo "-26512"
echo "-30834"

echo ""
echo "Previous Rust output (before fix):"
echo "32469"
echo "31454"
echo "27202"
echo "19509"
echo "10060"
echo "143"     # <- Was wrong
echo "-10137"  # <- Was wrong
echo "-19210"  # <- Was wrong
echo "-26520"  # <- Was wrong
echo "-31159"  # <- Was wrong

echo ""
echo "Summary: The fix in lsp_select_2() corrected the indexing from lspcb2[k1][j-NC] to lspcb2[k1][j]"
echo "This ensures we access the correct second half of the codebook entries (indices 5-9)" 