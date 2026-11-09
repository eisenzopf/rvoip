//! Executor v2: `sendmmsg(2)` for packets of one call that leave
//! together on its socket (Linux only).
//!
//! The only `unsafe` here is building the `mmsghdr`/`iovec` arrays and the
//! socket address on the stack and making the one system call. Every pointer
//! handed to the kernel points into those arrays or into the caller's
//! borrowed datagrams, all of which outlive the call; the kernel only reads
//! them (`sendmmsg` writes nothing but each header's `msg_len`).

use std::io;
use std::mem::{size_of, zeroed};
use std::net::SocketAddr;
use std::os::fd::RawFd;

/// Largest batch one call sends. A pacing catch-up is at most eight frames.
pub(crate) const MAX_BATCH: usize = 16;

/// `addr` as a `sockaddr_storage` and its length.
fn socket_address(addr: SocketAddr) -> (libc::sockaddr_storage, libc::socklen_t) {
    // SAFETY: all-zero is a valid `sockaddr_storage`, `sockaddr_in` and
    // `sockaddr_in6` (plain C structs of integers and byte arrays).
    let mut storage: libc::sockaddr_storage = unsafe { zeroed() };
    match addr {
        SocketAddr::V4(v4) => {
            let mut sin: libc::sockaddr_in = unsafe { zeroed() };
            sin.sin_family = libc::AF_INET as libc::sa_family_t;
            sin.sin_port = v4.port().to_be();
            sin.sin_addr = libc::in_addr {
                s_addr: u32::from_ne_bytes(v4.ip().octets()),
            };
            // SAFETY: `sockaddr_storage` is larger than and aligned for
            // `sockaddr_in`; this is the documented way to fill it.
            unsafe {
                std::ptr::write(
                    (&mut storage as *mut libc::sockaddr_storage).cast::<libc::sockaddr_in>(),
                    sin,
                );
            }
            (storage, size_of::<libc::sockaddr_in>() as libc::socklen_t)
        }
        SocketAddr::V6(v6) => {
            let mut sin6: libc::sockaddr_in6 = unsafe { zeroed() };
            sin6.sin6_family = libc::AF_INET6 as libc::sa_family_t;
            sin6.sin6_port = v6.port().to_be();
            sin6.sin6_flowinfo = v6.flowinfo();
            sin6.sin6_addr = libc::in6_addr {
                s6_addr: v6.ip().octets(),
            };
            sin6.sin6_scope_id = v6.scope_id();
            // SAFETY: as above, for `sockaddr_in6`.
            unsafe {
                std::ptr::write(
                    (&mut storage as *mut libc::sockaddr_storage).cast::<libc::sockaddr_in6>(),
                    sin6,
                );
            }
            (storage, size_of::<libc::sockaddr_in6>() as libc::socklen_t)
        }
    }
}

/// Send `datagrams` (at most [`MAX_BATCH`]; the rest are ignored) to `dest`
/// on `fd` in one `sendmmsg` call, in order. Returns how many leading
/// datagrams the kernel accepted (at least one), or the error of the first.
/// A short count means the next datagram would block or failed; the caller
/// retries the remainder one `sendto` at a time to learn which.
pub(crate) fn send_to_many(fd: RawFd, datagrams: &[&[u8]], dest: SocketAddr) -> io::Result<usize> {
    let count = datagrams.len().min(MAX_BATCH);
    if count == 0 {
        return Ok(0);
    }
    let (mut storage, address_len) = socket_address(dest);
    // SAFETY: all-zero `iovec`/`mmsghdr` values are valid (null pointers,
    // zero lengths); every used entry is filled in below.
    let mut iovecs: [libc::iovec; MAX_BATCH] = unsafe { zeroed() };
    let mut messages: [libc::mmsghdr; MAX_BATCH] = unsafe { zeroed() };
    for (iovec, datagram) in iovecs.iter_mut().zip(datagrams.iter()).take(count) {
        // The kernel only reads through `iov_base`.
        iovec.iov_base = datagram.as_ptr() as *mut libc::c_void;
        iovec.iov_len = datagram.len();
    }
    let name = (&mut storage as *mut libc::sockaddr_storage).cast::<libc::c_void>();
    for (message, iovec) in messages.iter_mut().zip(iovecs.iter_mut()).take(count) {
        message.msg_hdr.msg_name = name;
        message.msg_hdr.msg_namelen = address_len;
        message.msg_hdr.msg_iov = iovec as *mut libc::iovec;
        // `size_t` on glibc, `c_int` on musl.
        message.msg_hdr.msg_iovlen = 1 as _;
    }
    // SAFETY: `messages[..count]` are initialised headers whose name, iovec
    // and data pointers stay valid for the duration of the call (`storage`,
    // `iovecs` and `datagrams` are all alive and not moved); `fd` is the
    // caller's open socket.
    let sent = unsafe { libc::sendmmsg(fd, messages.as_mut_ptr(), count as libc::c_uint, 0) };
    if sent < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(sent as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;

    #[test]
    fn one_call_delivers_every_datagram_in_order() {
        let sender = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let receiver = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        receiver
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let payloads: Vec<Vec<u8>> = (0..5u8).map(|i| vec![i; 20 + usize::from(i)]).collect();
        let views: Vec<&[u8]> = payloads.iter().map(Vec::as_slice).collect();
        let sent =
            send_to_many(sender.as_raw_fd(), &views, receiver.local_addr().unwrap()).unwrap();
        assert_eq!(sent, 5);
        let mut buffer = [0u8; 64];
        for expected in &payloads {
            let (size, from) = receiver.recv_from(&mut buffer).unwrap();
            assert_eq!(&buffer[..size], expected.as_slice());
            assert_eq!(from, sender.local_addr().unwrap());
        }
    }

    #[test]
    fn ipv6_destinations_are_encoded() {
        let Ok(sender) = std::net::UdpSocket::bind("[::1]:0") else {
            return; // no IPv6 loopback on this host
        };
        let receiver = std::net::UdpSocket::bind("[::1]:0").unwrap();
        receiver
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let sent = send_to_many(
            sender.as_raw_fd(),
            &[b"six".as_slice(), b"seven".as_slice()],
            receiver.local_addr().unwrap(),
        )
        .unwrap();
        assert_eq!(sent, 2);
        let mut buffer = [0u8; 16];
        let (size, _) = receiver.recv_from(&mut buffer).unwrap();
        assert_eq!(&buffer[..size], b"six");
    }

    #[test]
    fn an_empty_batch_sends_nothing() {
        let sender = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        assert_eq!(
            send_to_many(sender.as_raw_fd(), &[], "127.0.0.1:9".parse().unwrap()).unwrap(),
            0
        );
    }
}
