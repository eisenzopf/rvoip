INVITE sip:user@example.com SIP/2.0
To: sip:user@example.com
From: sip:user@east.example.com;tag=81x2
Via: SIP/2.0/UDP [::ffff:192.0.2.10]:19823;branch=z9hG4bKbh19
Via: SIP/2.0/UDP [::ffff:192.0.2.2];branch=z9hG4bKas3-111
Call-ID: SSG9559905523997077@hlau_4100
Contact: "T. desk phone" <sip:ted@[::ffff:192.0.2.2]>
CSeq: 612 INVITE
Max-Forwards: 70
Content-Type: application/sdp
Content-Length: 236

v=0
o=assistant 971731711378798081 0 IN IP6 ::ffff:192.0.2.2
s=Call me soon, please!
c=IN IP6 ::ffff:192.0.2.2
t=3338481189 3370017201
m=audio 6000 RTP/AVP 2
a=rtpmap:2 G726-32/8000
m=video 6024 RTP/AVP 107
a=rtpmap:107 H263-1998/90000 