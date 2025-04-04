#!/bin/bash
# Test script to simulate a call flow using the call engine
# Modified for better SIP message formatting

# Configuration
SERVER_IP="127.0.0.1"
SERVER_PORT="5060"
ALICE_PORT="5070"
BOB_PORT="5071"
CALL_ID="test-call-$(date +%s)"
ALICE_TAG="alice-tag-$(date +%s)"
BOB_TAG="bob-tag-$(date +%s)"
BRANCH="z9hG4bK-$(date +%s)"

# Temp files for output
ALICE_OUTPUT="/tmp/alice_sip.txt"
BOB_OUTPUT="/tmp/bob_sip.txt"

echo "Call-ID: $CALL_ID"
echo "Alice tag: $ALICE_TAG"
echo "Bob tag: $BOB_TAG"
echo

# Clear previous outputs
> $ALICE_OUTPUT
> $BOB_OUTPUT

# Function to start a listener that will be automatically killed later
start_listener() {
    local port=$1
    local output_file=$2
    local name=$3

    echo "Starting listener for $name on port $port..."
    nc -u -l $port > $output_file &
    echo $!
}

# Start listeners
ALICE_PID=$(start_listener $ALICE_PORT $ALICE_OUTPUT "Alice")
BOB_PID=$(start_listener $BOB_PORT $BOB_OUTPUT "Bob")

sleep 1

# Function to properly send SIP messages via UDP
send_sip_message() {
    local message="$1"
    local server="$2"
    local port="$3"
    
    # Use netcat (nc) with UDP mode (-u) to send the message
    # The empty line between headers and body is critical in SIP
    # The message already has the proper format from the heredoc
    echo -n "$message" | nc -u "$server" "$port"
}

# 1. Register Alice with modified formatting
echo
echo "=== Registering Alice ==="
echo

REGISTER_MSG=$(cat << 'EOF'
REGISTER sip:rvoip.local SIP/2.0
Via: SIP/2.0/UDP 127.0.0.1:5070;branch=z9hG4bK-register
Max-Forwards: 70
From: <sip:alice@rvoip.local>;tag=alice-tag
To: <sip:alice@rvoip.local>
Call-ID: register-alice-12345
CSeq: 1 REGISTER
Contact: <sip:alice@127.0.0.1:5070>
Expires: 3600
User-Agent: SIP-Test-Client/0.1
Content-Length: 0

EOF
)

# Replace placeholders with actual values
REGISTER_MSG=$(echo "$REGISTER_MSG" | sed "s/127.0.0.1:5070/127.0.0.1:$ALICE_PORT/g" | \
                sed "s/z9hG4bK-register/$BRANCH-register/g" | \
                sed "s/alice-tag/$ALICE_TAG/g" | \
                sed "s/register-alice-12345/register-alice-$CALL_ID/g")

send_sip_message "$REGISTER_MSG" "$SERVER_IP" "$SERVER_PORT"
sleep 2

# Check if Alice received a response
echo "Checking response to Alice registration..."
if [ -s "$ALICE_OUTPUT" ]; then
    echo "Response received:"
    cat "$ALICE_OUTPUT"
    
    # Extract nonce and realm from 401 response
    NONCE=$(grep -o 'nonce="[^"]*"' "$ALICE_OUTPUT" | head -1 | cut -d'"' -f2)
    REALM=$(grep -o 'realm="[^"]*"' "$ALICE_OUTPUT" | head -1 | cut -d'"' -f2)
    
    if [ -n "$NONCE" ] && [ -n "$REALM" ]; then
        echo "Extracted nonce: $NONCE"
        echo "Extracted realm: $REALM"
        
        # Generate a simple MD5 hash for password authentication
        # Note: In a real system, use proper digest auth calculation
        USERNAME="alice"
        PASSWORD="password123"
        RESPONSE_HASH=$(echo -n "$USERNAME:$REALM:$PASSWORD" | md5sum | cut -d' ' -f1)
        
        echo
        echo "=== Sending authenticated REGISTER for Alice ==="
        echo
        
        AUTH_REGISTER_MSG=$(cat << 'EOF'
REGISTER sip:rvoip.local SIP/2.0
Via: SIP/2.0/UDP 127.0.0.1:5070;branch=z9hG4bK-register-auth
Max-Forwards: 70
From: <sip:alice@rvoip.local>;tag=alice-tag
To: <sip:alice@rvoip.local>
Call-ID: register-alice-12345
CSeq: 2 REGISTER
Contact: <sip:alice@127.0.0.1:5070>
Expires: 3600
Authorization: Digest username="alice", realm="rvoip", nonce="nonce-value", uri="sip:rvoip.local", response="response-hash", algorithm=MD5
User-Agent: SIP-Test-Client/0.1
Content-Length: 0

EOF
)
        
        # Replace placeholders with actual values
        AUTH_REGISTER_MSG=$(echo "$AUTH_REGISTER_MSG" | sed "s/127.0.0.1:5070/127.0.0.1:$ALICE_PORT/g" | \
                sed "s/z9hG4bK-register-auth/$BRANCH-register-auth/g" | \
                sed "s/alice-tag/$ALICE_TAG/g" | \
                sed "s/register-alice-12345/register-alice-$CALL_ID/g" | \
                sed "s/realm=\"rvoip\"/realm=\"$REALM\"/g" | \
                sed "s/nonce=\"nonce-value\"/nonce=\"$NONCE\"/g" | \
                sed "s/response=\"response-hash\"/response=\"$RESPONSE_HASH\"/g")
        
        send_sip_message "$AUTH_REGISTER_MSG" "$SERVER_IP" "$SERVER_PORT"
        
        sleep 2
        # Check for authentication response
        if [ -s "$ALICE_OUTPUT" ]; then
            echo "Authenticated response received:"
            cat "$ALICE_OUTPUT"
            > "$ALICE_OUTPUT"  # Clear output for next test
        else
            echo "No response received for authenticated request."
            echo "Exiting test."
            kill $ALICE_PID $BOB_PID 2>/dev/null || true
            exit 1
        fi
    else
        echo "Failed to extract authentication parameters."
        echo "Exiting test."
        kill $ALICE_PID $BOB_PID 2>/dev/null || true
        exit 1
    fi
    
    > "$ALICE_OUTPUT"  # Clear output for next test
else
    echo "No response received for Alice registration. Server might not be running or has issues."
    echo "Exiting test."
    kill $ALICE_PID $BOB_PID 2>/dev/null || true
    exit 1
fi

# 2. Register Bob with modified formatting - follow same authentication pattern as Alice
echo
echo "=== Registering Bob ==="
echo

BOB_REGISTER_MSG=$(cat << 'EOF'
REGISTER sip:rvoip.local SIP/2.0
Via: SIP/2.0/UDP 127.0.0.1:5071;branch=z9hG4bK-register
Max-Forwards: 70
From: <sip:bob@rvoip.local>;tag=bob-tag
To: <sip:bob@rvoip.local>
Call-ID: register-bob-12345
CSeq: 1 REGISTER
Contact: <sip:bob@127.0.0.1:5071>
Expires: 3600
User-Agent: SIP-Test-Client/0.1
Content-Length: 0

EOF
)

# Replace placeholders with actual values
BOB_REGISTER_MSG=$(echo "$BOB_REGISTER_MSG" | sed "s/127.0.0.1:5071/127.0.0.1:$BOB_PORT/g" | \
                sed "s/z9hG4bK-register/$BRANCH-register/g" | \
                sed "s/bob-tag/$BOB_TAG/g" | \
                sed "s/register-bob-12345/register-bob-$CALL_ID/g")

send_sip_message "$BOB_REGISTER_MSG" "$SERVER_IP" "$SERVER_PORT"
sleep 2

# Check if Bob received a response
echo "Checking response to Bob registration..."
if [ -s "$BOB_OUTPUT" ]; then
    echo "Response received:"
    cat "$BOB_OUTPUT"
    
    # Extract nonce and realm from 401 response
    NONCE=$(grep -o 'nonce="[^"]*"' "$BOB_OUTPUT" | head -1 | cut -d'"' -f2)
    REALM=$(grep -o 'realm="[^"]*"' "$BOB_OUTPUT" | head -1 | cut -d'"' -f2)
    
    if [ -n "$NONCE" ] && [ -n "$REALM" ]; then
        echo "Extracted nonce: $NONCE"
        echo "Extracted realm: $REALM"
        
        # Generate a simple MD5 hash for password authentication
        USERNAME="bob"
        PASSWORD="password123"
        RESPONSE_HASH=$(echo -n "$USERNAME:$REALM:$PASSWORD" | md5sum | cut -d' ' -f1)
        
        echo
        echo "=== Sending authenticated REGISTER for Bob ==="
        echo
        
        BOB_AUTH_REGISTER_MSG=$(cat << 'EOF'
REGISTER sip:rvoip.local SIP/2.0
Via: SIP/2.0/UDP 127.0.0.1:5071;branch=z9hG4bK-register-auth
Max-Forwards: 70
From: <sip:bob@rvoip.local>;tag=bob-tag
To: <sip:bob@rvoip.local>
Call-ID: register-bob-12345
CSeq: 2 REGISTER
Contact: <sip:bob@127.0.0.1:5071>
Expires: 3600
Authorization: Digest username="bob", realm="rvoip", nonce="nonce-value", uri="sip:rvoip.local", response="response-hash", algorithm=MD5
User-Agent: SIP-Test-Client/0.1
Content-Length: 0

EOF
)
        
        # Replace placeholders with actual values
        BOB_AUTH_REGISTER_MSG=$(echo "$BOB_AUTH_REGISTER_MSG" | sed "s/127.0.0.1:5071/127.0.0.1:$BOB_PORT/g" | \
                sed "s/z9hG4bK-register-auth/$BRANCH-register-auth/g" | \
                sed "s/bob-tag/$BOB_TAG/g" | \
                sed "s/register-bob-12345/register-bob-$CALL_ID/g" | \
                sed "s/realm=\"rvoip\"/realm=\"$REALM\"/g" | \
                sed "s/nonce=\"nonce-value\"/nonce=\"$NONCE\"/g" | \
                sed "s/response=\"response-hash\"/response=\"$RESPONSE_HASH\"/g")
        
        send_sip_message "$BOB_AUTH_REGISTER_MSG" "$SERVER_IP" "$SERVER_PORT"
        
        sleep 2
        # Check for authentication response
        if [ -s "$BOB_OUTPUT" ]; then
            echo "Authenticated response received:"
            cat "$BOB_OUTPUT"
            > "$BOB_OUTPUT"  # Clear output for next test
        else
            echo "No response received for authenticated request."
            echo "Exiting test."
            kill $ALICE_PID $BOB_PID 2>/dev/null || true
            exit 1
        fi
    else
        echo "Failed to extract authentication parameters."
        echo "Exiting test."
        kill $ALICE_PID $BOB_PID 2>/dev/null || true
        exit 1
    fi
    
    > "$BOB_OUTPUT"  # Clear output for next test
else
    echo "No response received for Bob registration. Server might not be running or has issues."
    echo "Exiting test."
    kill $ALICE_PID $BOB_PID 2>/dev/null || true
    exit 1
fi

# 3. Alice calls Bob - INVITE with modified formatting
echo
echo "=== Alice sends INVITE to Bob ==="
echo

INVITE_MSG=$(cat << 'EOF'
INVITE sip:bob@rvoip.local SIP/2.0
Via: SIP/2.0/UDP 127.0.0.1:5070;branch=z9hG4bK-invite
Max-Forwards: 70
From: <sip:alice@rvoip.local>;tag=alice-tag
To: <sip:bob@rvoip.local>
Call-ID: call-id-12345
CSeq: 1 INVITE
Contact: <sip:alice@127.0.0.1:5070>
Authorization: Digest username="alice", realm="rvoip", nonce="auth-nonce", uri="sip:bob@rvoip.local", response="auth-response", algorithm=MD5
Content-Type: application/sdp
User-Agent: SIP-Test-Client/0.1
Content-Length: 158

v=0
o=alice 123456 789012 IN IP4 127.0.0.1
s=Call
c=IN IP4 127.0.0.1
t=0 0
m=audio 10000 RTP/AVP 0
a=rtpmap:0 PCMU/8000
a=sendrecv
EOF
)

# Replace placeholders with actual values
INVITE_MSG=$(echo "$INVITE_MSG" | sed "s/127.0.0.1:5070/127.0.0.1:$ALICE_PORT/g" | \
            sed "s/z9hG4bK-invite/$BRANCH-invite/g" | \
            sed "s/alice-tag/$ALICE_TAG/g" | \
            sed "s/call-id-12345/$CALL_ID/g")

send_sip_message "$INVITE_MSG" "$SERVER_IP" "$SERVER_PORT"
sleep 2

# Check response files
echo "Checking responses to INVITE..."
if [ -s "$ALICE_OUTPUT" ]; then
    echo "Alice received response to INVITE:"
    cat "$ALICE_OUTPUT"
    > "$ALICE_OUTPUT"  # Clear output for next test
else
    echo "No response received by Alice for INVITE."
fi

if [ -s "$BOB_OUTPUT" ]; then
    echo "Bob received the INVITE:"
    cat "$BOB_OUTPUT"
    > "$BOB_OUTPUT"  # Clear output for next test
else
    echo "Bob did not receive the INVITE."
    echo "Exiting test."
    kill $ALICE_PID $BOB_PID 2>/dev/null || true
    exit 1
fi

# 4. Bob responds with 200 OK using modified formatting
echo
echo "=== Bob responds with 200 OK ==="
echo

OK_MSG=$(cat << 'EOF'
SIP/2.0 200 OK
Via: SIP/2.0/UDP 127.0.0.1:5070;branch=z9hG4bK-invite
From: <sip:alice@rvoip.local>;tag=alice-tag
To: <sip:bob@rvoip.local>;tag=bob-tag
Call-ID: call-id-12345
CSeq: 1 INVITE
Contact: <sip:bob@127.0.0.1:5071>
Content-Type: application/sdp
User-Agent: SIP-Test-Client/0.1
Content-Length: 156

v=0
o=bob 654321 210987 IN IP4 127.0.0.1
s=Call
c=IN IP4 127.0.0.1
t=0 0
m=audio 10001 RTP/AVP 0
a=rtpmap:0 PCMU/8000
a=sendrecv
EOF
)

# Replace placeholders with actual values
OK_MSG=$(echo "$OK_MSG" | sed "s/127.0.0.1:5070/127.0.0.1:$ALICE_PORT/g" | \
        sed "s/127.0.0.1:5071/127.0.0.1:$BOB_PORT/g" | \
        sed "s/z9hG4bK-invite/$BRANCH-invite/g" | \
        sed "s/alice-tag/$ALICE_TAG/g" | \
        sed "s/bob-tag/$BOB_TAG/g" | \
        sed "s/call-id-12345/$CALL_ID/g")

send_sip_message "$OK_MSG" "$SERVER_IP" "$SERVER_PORT"
sleep 2

# Check responses 
echo "Checking responses after 200 OK..."
if [ -s "$ALICE_OUTPUT" ]; then
    echo "Alice received response after 200 OK:"
    cat "$ALICE_OUTPUT"
    > "$ALICE_OUTPUT"  # Clear output for next test
else
    echo "No response received by Alice after 200 OK."
fi

# 5. Alice sends ACK with modified formatting
echo
echo "=== Alice sends ACK ==="
echo

ACK_MSG=$(cat << 'EOF'
ACK sip:bob@rvoip.local SIP/2.0
Via: SIP/2.0/UDP 127.0.0.1:5070;branch=z9hG4bK-ack
Max-Forwards: 70
From: <sip:alice@rvoip.local>;tag=alice-tag
To: <sip:bob@rvoip.local>;tag=bob-tag
Call-ID: call-id-12345
CSeq: 1 ACK
Contact: <sip:alice@127.0.0.1:5070>
User-Agent: SIP-Test-Client/0.1
Content-Length: 0

EOF
)

# Replace placeholders with actual values
ACK_MSG=$(echo "$ACK_MSG" | sed "s/127.0.0.1:5070/127.0.0.1:$ALICE_PORT/g" | \
        sed "s/z9hG4bK-ack/$BRANCH-ack/g" | \
        sed "s/alice-tag/$ALICE_TAG/g" | \
        sed "s/bob-tag/$BOB_TAG/g" | \
        sed "s/call-id-12345/$CALL_ID/g")

send_sip_message "$ACK_MSG" "$SERVER_IP" "$SERVER_PORT"
sleep 3

# Check for any messages after ACK
echo "Checking responses after ACK..."
if [ -s "$BOB_OUTPUT" ]; then
    echo "Bob received response after ACK:"
    cat "$BOB_OUTPUT"
    > "$BOB_OUTPUT"  # Clear output for next test
else
    echo "No response received by Bob after ACK."
fi

# 6. Alice terminates call with BYE using modified formatting
echo
echo "=== Alice sends BYE ==="
echo

BYE_MSG=$(cat << 'EOF'
BYE sip:bob@rvoip.local SIP/2.0
Via: SIP/2.0/UDP 127.0.0.1:5070;branch=z9hG4bK-bye
Max-Forwards: 70
From: <sip:alice@rvoip.local>;tag=alice-tag
To: <sip:bob@rvoip.local>;tag=bob-tag
Call-ID: call-id-12345
CSeq: 2 BYE
Authorization: Digest username="alice", realm="rvoip", nonce="auth-nonce", uri="sip:bob@rvoip.local", response="auth-response", algorithm=MD5
User-Agent: SIP-Test-Client/0.1
Content-Length: 0

EOF
)

# Replace placeholders with actual values
BYE_MSG=$(echo "$BYE_MSG" | sed "s/127.0.0.1:5070/127.0.0.1:$ALICE_PORT/g" | \
        sed "s/z9hG4bK-bye/$BRANCH-bye/g" | \
        sed "s/alice-tag/$ALICE_TAG/g" | \
        sed "s/bob-tag/$BOB_TAG/g" | \
        sed "s/call-id-12345/$CALL_ID/g")

send_sip_message "$BYE_MSG" "$SERVER_IP" "$SERVER_PORT"
sleep 2

# Final check for responses
echo "Checking final responses..."
if [ -s "$ALICE_OUTPUT" ]; then
    echo "Alice received final response:"
    cat "$ALICE_OUTPUT"
else
    echo "No final response received by Alice."
fi

if [ -s "$BOB_OUTPUT" ]; then
    echo "Bob received final response:"
    cat "$BOB_OUTPUT"
else
    echo "No final response received by Bob."
fi

# Kill listeners
kill $ALICE_PID $BOB_PID 2>/dev/null || true

# Clean up temp files
rm -f $ALICE_OUTPUT $BOB_OUTPUT

echo
echo "=== Test completed ==="
echo 