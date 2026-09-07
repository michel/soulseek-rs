//! TCP keepalive for the server connection.
//!
//! A NAT that drops an idle mapping leaves the socket looking connected while
//! the server can no longer reach us, and nothing ever errors. Probes every
//! ten seconds keep the mapping alive and turn a dead path into a read error
//! within about half a minute, which is what Nicotine+ does. The server
//! stopped answering ServerPing (code 32) long ago, so that is no substitute.

use std::io;
use std::net::TcpStream;

#[cfg(unix)]
pub fn set_keepalive(stream: &TcpStream) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;
    let fd = stream.as_raw_fd();
    set_option(fd, libc::SOL_SOCKET, libc::SO_KEEPALIVE, 1)?;
    #[cfg(any(
        target_vendor = "apple",
        target_os = "linux",
        target_os = "android",
        target_os = "freebsd",
        target_os = "netbsd"
    ))]
    {
        set_option(fd, libc::IPPROTO_TCP, IDLE_SECONDS_OPTION, 10)?;
        set_option(fd, libc::IPPROTO_TCP, libc::TCP_KEEPINTVL, 2)?;
        set_option(fd, libc::IPPROTO_TCP, libc::TCP_KEEPCNT, 10)?;
    }
    Ok(())
}

#[cfg(target_vendor = "apple")]
const IDLE_SECONDS_OPTION: libc::c_int = libc::TCP_KEEPALIVE;
#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "freebsd",
    target_os = "netbsd"
))]
const IDLE_SECONDS_OPTION: libc::c_int = libc::TCP_KEEPIDLE;

#[cfg(unix)]
fn set_option(
    fd: libc::c_int,
    level: libc::c_int,
    name: libc::c_int,
    value: libc::c_int,
) -> io::Result<()> {
    // SAFETY: `value` outlives the call and its exact size travels with it.
    let rc = unsafe {
        libc::setsockopt(
            fd,
            level,
            name,
            (&raw const value).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

// ponytail: Windows wants WSAIoctl(SIO_KEEPALIVE_VALS); wire it when the
// daemon ships there.
#[cfg(not(unix))]
pub fn set_keepalive(_stream: &TcpStream) -> io::Result<()> {
    Ok(())
}

#[cfg(all(test, unix))]
pub fn keepalive_enabled(stream: &TcpStream) -> bool {
    use std::os::unix::io::AsRawFd;
    let mut value: libc::c_int = 0;
    let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    // SAFETY: `value` and `len` outlive the call and `len` bounds the write.
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_KEEPALIVE,
            (&raw mut value).cast(),
            &raw mut len,
        )
    };
    rc == 0 && value != 0
}

#[cfg(all(test, unix))]
#[test]
fn keepalive_is_off_until_asked_for() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    assert!(!keepalive_enabled(&stream));
    set_keepalive(&stream).unwrap();
    assert!(keepalive_enabled(&stream));
}
