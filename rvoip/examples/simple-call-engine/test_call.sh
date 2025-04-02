#!/bin/bash
# Test script to simulate a call flow using the call engine

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

# Start listening for responses to Alice
echo "Starting listener for Alice on port $ALICE_PORT..."
nc -u -l $ALICE_PORT > $ALICE_OUTPUT &
ALICE_PID=$!

# Start listening for responses to Bob
echo "Starting listener for Bob on port $BOB_PORT..."
nc -u -l $BOB_PORT > $BOB_OUTPUT &
BOB_PID=$!

sleep 1

# 1. Register Alice
echo -e "\n=== Registering Alice ===\n"
echo -n "REGISTER sip:rvoip.local SIP/2.0
Via: SIP/2.0/UDP 127.0.0.1:$ALICE_PORT;branch=$BRANCH-register
Max-Forwards: 70
From: <sip:alice@rvoip.local>;tag=$ALICE_TAG
To: <sip:alice@rvoip.local>
Call-ID: register-alice-$CALL_ID
CSeq: 1 REGISTER
Contact: <sip:alice@127.0.0.1:$ALICE_PORT>
Expires: 3600
User-Agent: SIP-Test-Client/0.1
Content-Length: 0

" | nc -u $SERVER_IP $SERVER_PORT

sleep 2

# 2. Register Bob
echo -e "\n=== Registering Bob ===\n"
echo -n "REGISTER sip:rvoip.local SIP/2.0
Via: SIP/2.0/UDP 127.0.0.1:$BOB_PORT;branch=$BRANCH-register
Max-Forwards: 70
From: <sip:bob@rvoip.local>;tag=$BOB_TAG
To: <sip:bob@rvoip.local>
Call-ID: register-bob-$CALL_ID
CSeq: 1 REGISTER
Contact: <sip:bob@127.0.0.1:$BOB_PORT>
Expires: 3600
User-Agent: SIP-Test-Client/0.1
Content-Length: 0

" | nc -u $SERVER_IP $SERVER_PORT

sleep 2

# 3. Alice calls Bob - INVITE
echo -e "\n=== Alice sends INVITE to Bob ===\n"
echo -n "INVITE sip:bob@rvoip.local SIP/2.0
Via: SIP/2.0/UDP 127.0.0.1:$ALICE_PORT;branch=$BRANCH-invite
Max-Forwards: 70
From: <sip:alice@rvoip.local>;tag=$ALICE_TAG
To: <sip:bob@rvoip.local>
Call-ID: $CALL_ID
CSeq: 1 INVITE
Contact: <sip:alice@127.0.0.1:$ALICE_PORT>
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
" | nc -u $SERVER_IP $SERVER_PORT

sleep 2

# 4. Look for responses and create Bob's 200 OK
echo -e "\n=== Bob responds with 200 OK ===\n"
echo -n "SIP/2.0 200 OK
Via: SIP/2.0/UDP 127.0.0.1:$ALICE_PORT;branch=$BRANCH-invite
From: <sip:alice@rvoip.local>;tag=$ALICE_TAG
To: <sip:bob@rvoip.local>;tag=$BOB_TAG
Call-ID: $CALL_ID
CSeq: 1 INVITE
Contact: <sip:bob@127.0.0.1:$BOB_PORT>
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
" | nc -u $SERVER_IP $SERVER_PORT

sleep 2

# 5. Alice sends ACK
echo -e "\n=== Alice sends ACK ===\n"
echo -n "ACK sip:bob@127.0.0.1:$BOB_PORT SIP/2.0
Via: SIP/2.0/UDP 127.0.0.1:$ALICE_PORT;branch=$BRANCH-ack
Max-Forwards: 70
From: <sip:alice@rvoip.local>;tag=$ALICE_TAG
To: <sip:bob@rvoip.local>;tag=$BOB_TAG
Call-ID: $CALL_ID
CSeq: 1 ACK
Contact: <sip:alice@127.0.0.1:$ALICE_PORT>
User-Agent: SIP-Test-Client/0.1
Content-Length: 0

" | nc -u $SERVER_IP $SERVER_PORT

sleep 5

# 6. Alice terminates call with BYE
echo -e "\n=== Alice sends BYE ===\n"
echo -n "BYE sip:bob@127.0.0.1:$BOB_PORT SIP/2.0
Via: SIP/2.0/UDP 127.0.0.1:$ALICE_PORT;branch=$BRANCH-bye
Max-Forwards: 70
From: <sip:alice@rvoip.local>;tag=$ALICE_TAG
To: <sip:bob@rvoip.local>;tag=$BOB_TAG
Call-ID: $CALL_ID
CSeq: 2 BYE
User-Agent: SIP-Test-Client/0.1
Content-Length: 0

" | nc -u $SERVER_IP $SERVER_PORT

sleep 2

# Display all responses that Alice received
echo -e "\n=== Responses received by Alice ===\n"
cat $ALICE_OUTPUT

# Display all responses that Bob received
echo -e "\n=== Responses received by Bob ===\n"
cat $BOB_OUTPUT

# Kill listeners
kill $ALICE_PID $BOB_PID 2>/dev/null
wait $ALICE_PID $BOB_PID 2>/dev/null

# Clean up temp files
rm -f $ALICE_OUTPUT $BOB_OUTPUT

echo -e "\n=== Test completed ===\n" 