# Run SIPp test calls
echo -e "\n${YELLOW}Running SIPp test calls...${NC}"
echo "Making 5 calls, 1 call per second..."

cd "$SIPP_DIR"
sipp -sf customer_uac.xml \
    -s support \
    -i 127.0.0.1 \
    -p 5080 \
    -m 5 \
    -r 1 \
    -trace_msg \
    -trace_err \
    -trace_screen \
    -trace_stat \
    127.0.0.1:5060 \
    > "$SIPP_LOG" 2>&1 || true 