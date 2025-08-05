# Missing SIP Client Features

This document lists call control features that are commonly found in typical SIP clients but are not yet implemented in this library.

## Call Management
1. **Call waiting/multiple call handling** - Can't manage multiple simultaneous calls or switch between them
2. **Three-way conferencing** - No ability to merge calls into a conference
3. **Call park/unpark** - Can't park calls for retrieval from another extension
4. **Blind transfer** vs attended transfer - Only basic transfer implemented

## Call Control
5. **Call forwarding** - No conditional/unconditional call forwarding
6. **Do Not Disturb (DND)** - No ability to automatically reject calls
7. **Auto-answer** - No configurable auto-answer for incoming calls
8. **Early media** - No support for playing media before call is answered
9. **Music on hold** - No MOH when calls are on hold

## User Features
10. **Redial/call history** - No last number redial or call history management
11. **Speed dial** - No preset number storage/dialing
12. **Call recording control** - No start/stop recording during calls
13. **Caller ID manipulation** - Can't set custom caller ID

## Presence & Monitoring
14. **Voicemail indication** - No Message Waiting Indicator (MWI) support
15. **Busy lamp field (BLF)** - No presence/line status monitoring

## Protocol Support
16. **REFER handling** - For call transfer completion notifications
17. **UPDATE method** - For mid-call session changes without re-INVITE
18. **PRACK** - No support for reliable provisional responses
19. **Session timers** (RFC 4028) - No automatic call keep-alive
20. **Codec renegotiation** - Can't change codecs mid-call