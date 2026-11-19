---
section: Security
breaking: true
---
SIP UDP transports and RTP/RTCP media sockets no longer set `SO_REUSEADDR`,
on any port. On Linux the flag let any other local socket that also set it,
in any process and under any user, bind a live SIP or media port and receive
every datagram sent to it, so signalling or call audio could be silently
taken. UDP has no TIME_WAIT, so a port can be bound again as soon as its
socket closes, including when its process is killed: restarts are not slowed.

On Windows, where a later `SO_REUSEADDR` socket could take a held port
whatever the holder set, SIP and media UDP sockets now set
`SO_EXCLUSIVEADDRUSE` before they bind. The Windows build is compile-checked
(`x86_64-pc-windows-gnu`) but not tested, since rvoip has no Windows CI. The
macOS/BSD behaviour follows the BSD socket rules and is not CI-tested either.

Migration: a second socket can no longer bind a UDP port that an rvoip
transport still holds. If you paired a wildcard bind with a specific address
on the same port (for example `0.0.0.0:5060` and `10.0.0.1:5060`), bind each
interface address explicitly instead. If you started a new process on the
port before the old one exited, or rebound a port before closing the
transport that held it, close or stop the old one first.
