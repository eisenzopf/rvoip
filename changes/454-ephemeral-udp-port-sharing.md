---
section: Fixed
breaking: false
---
A SIP UDP transport bound to an ephemeral port (port 0) no longer sets
`SO_REUSEADDR`. On Linux that let another reuse-flagged UDP socket, in the
same process or another, be given the same port and take every datagram sent
to it, so a call could silently lose its SIP responses and time out.
