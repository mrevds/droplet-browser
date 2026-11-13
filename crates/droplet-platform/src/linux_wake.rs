//! Cross-thread wake-up for the Linux event loops (X11, Wayland), DROP-110.
//!
//! A connected `UnixStream` pair acts as a self-pipe: the [`Waker`] writes one byte from
//! any thread, the event loop `poll()`s the read end next to the display connection fd.

use std::io::{ErrorKind, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::raw::{c_int, c_short, c_ulong};
use std::os::unix::net::UnixStream;
use std::sync::Arc;

use crate::Waker;

const POLLIN: c_short = 0x001;

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: c_short,
    revents: c_short,
}

extern "C" {
    // libc is always linked by std on Linux; no extra crate or dlopen needed.
    fn poll(fds: *mut PollFd, nfds: c_ulong, timeout: c_int) -> c_int;
}

/// Blocks until one of `fds` is readable (or has an error/hang-up). Returns one flag per fd.
pub(crate) fn wait_readable<const N: usize>(fds: [RawFd; N]) -> [bool; N] {
    let mut pfds = fds.map(|fd| PollFd {
        fd,
        events: POLLIN,
        revents: 0,
    });
    loop {
        let rc = unsafe { poll(pfds.as_mut_ptr(), N as c_ulong, -1) };
        if rc >= 0 {
            break;
        }
        if std::io::Error::last_os_error().kind() != ErrorKind::Interrupted {
            // Treat a poll failure as "everything ready" so the caller re-checks and
            // never deadlocks waiting for an event that can no longer be observed.
            return [true; N];
        }
    }
    pfds.map(|p| p.revents != 0)
}

pub(crate) struct WakePipe {
    read: UnixStream,
    write: Arc<UnixStream>,
}

impl WakePipe {
    pub(crate) fn new() -> std::io::Result<Self> {
        let (read, write) = UnixStream::pair()?;
        read.set_nonblocking(true)?;
        write.set_nonblocking(true)?;
        Ok(Self {
            read,
            write: Arc::new(write),
        })
    }

    pub(crate) fn read_fd(&self) -> RawFd {
        self.read.as_raw_fd()
    }

    /// Callback for other threads. A full socket buffer already means "wake pending",
    /// so `WouldBlock` is ignored.
    pub(crate) fn waker(&self) -> Waker {
        let write = Arc::clone(&self.write);
        Arc::new(move || {
            let _ = (&*write).write(&[1]);
        })
    }

    /// Consumes all pending wake bytes; returns whether there were any.
    pub(crate) fn drain(&self) -> bool {
        let mut buf = [0u8; 64];
        let mut woke = false;
        loop {
            match (&self.read).read(&mut buf) {
                Ok(0) => return woke,
                Ok(_) => woke = true,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(_) => return woke, // WouldBlock: drained
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waker_from_another_thread_unblocks_poll_and_drains() {
        let pipe = WakePipe::new().unwrap();
        assert!(!pipe.drain());
        let waker = pipe.waker();
        let t = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            for _ in 0..1000 {
                waker(); // many wakes coalesce, never block the caller
            }
        });
        let ready = wait_readable([pipe.read_fd()]);
        assert!(ready[0]);
        t.join().unwrap();
        assert!(pipe.drain());
        assert!(!pipe.drain());
    }
}
