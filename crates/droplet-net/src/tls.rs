#[cfg(target_os = "linux")]
mod imp {
    use std::ffi::{CStr, CString};
    use std::io::{ErrorKind, Read, Write};
    use std::net::{IpAddr, TcpStream};
    use std::os::raw::{c_char, c_int, c_long, c_void};
    use std::os::unix::io::AsRawFd;
    use std::ptr;
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    use crate::dns::strip_ipv6_brackets;
    use crate::{NetError, HANDSHAKE_TIMEOUT, READ_TIMEOUT};

    const RTLD_LAZY: c_int = 1;
    const SSL_VERIFY_PEER: c_int = 1;
    const SSL_CTRL_SET_TLSEXT_HOSTNAME: c_int = 55;
    #[allow(non_upper_case_globals)]
    const TLSEXT_NAMETYPE_host_name: c_long = 0;
    const X509_V_OK: c_long = 0;

    const SSL_ERROR_WANT_READ: c_int = 2;
    const SSL_ERROR_WANT_WRITE: c_int = 3;
    const SSL_ERROR_SYSCALL: c_int = 5;
    const SSL_ERROR_ZERO_RETURN: c_int = 6;

    #[link(name = "c")]
    extern "C" {
        fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn dlclose(handle: *mut c_void) -> c_int;
    }

    #[allow(non_snake_case)]
    struct OpenSslRuntime {
        _handle: *mut c_void,
        ctx: *mut c_void,
        SSL_new: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
        SSL_free: unsafe extern "C" fn(*mut c_void),
        SSL_set_fd: unsafe extern "C" fn(*mut c_void, c_int) -> c_int,
        SSL_ctrl: unsafe extern "C" fn(*mut c_void, c_int, c_long, *mut c_void) -> c_long,
        SSL_set1_host: unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int,
        SSL_connect: unsafe extern "C" fn(*mut c_void) -> c_int,
        SSL_get_verify_result: unsafe extern "C" fn(*const c_void) -> c_long,
        SSL_read: unsafe extern "C" fn(*mut c_void, *mut c_void, c_int) -> c_int,
        SSL_write: unsafe extern "C" fn(*mut c_void, *const c_void, c_int) -> c_int,
        SSL_get_error: unsafe extern "C" fn(*const c_void, c_int) -> c_int,
    }

    unsafe impl Send for OpenSslRuntime {}
    unsafe impl Sync for OpenSslRuntime {}

    static TLS_RUNTIME: OnceLock<Result<OpenSslRuntime, String>> = OnceLock::new();

    unsafe fn load_sym<T: Copy>(handle: *mut c_void, name: &CStr) -> Result<T, String> {
        let ptr = dlsym(handle, name.as_ptr());
        if ptr.is_null() {
            return Err(format!(
                "Failed to resolve OpenSSL symbol '{}'",
                name.to_string_lossy()
            ));
        }
        debug_assert_eq!(std::mem::size_of::<T>(), std::mem::size_of::<*mut c_void>());
        Ok(std::mem::transmute_copy::<*mut c_void, T>(&ptr))
    }

    #[allow(non_snake_case)]
    impl OpenSslRuntime {
        fn load() -> Result<Self, String> {
            let handle = unsafe {
                let h4 = dlopen(c"libssl.so.4".as_ptr(), RTLD_LAZY);
                if !h4.is_null() {
                    h4
                } else {
                    let h3 = dlopen(c"libssl.so.3".as_ptr(), RTLD_LAZY);
                    if !h3.is_null() {
                        h3
                    } else {
                        dlopen(c"libssl.so".as_ptr(), RTLD_LAZY)
                    }
                }
            };
            if handle.is_null() {
                return Err(
                    "Could not dynamically load libssl.so.4, libssl.so.3, or libssl.so".into(),
                );
            }

            let init_res = unsafe { Self::init_with_handle(handle) };
            if init_res.is_err() {
                unsafe {
                    dlclose(handle);
                }
            }
            init_res
        }

        unsafe fn init_with_handle(handle: *mut c_void) -> Result<Self, String> {
            let OPENSSL_init_ssl: unsafe extern "C" fn(u64, *const c_void) -> c_int =
                load_sym(handle, c"OPENSSL_init_ssl")?;
            let TLS_client_method: unsafe extern "C" fn() -> *const c_void =
                load_sym(handle, c"TLS_client_method")?;
            let SSL_CTX_new: unsafe extern "C" fn(*const c_void) -> *mut c_void =
                load_sym(handle, c"SSL_CTX_new")?;
            let SSL_CTX_free: unsafe extern "C" fn(*mut c_void) =
                load_sym(handle, c"SSL_CTX_free")?;
            let SSL_CTX_set_default_verify_paths: unsafe extern "C" fn(*mut c_void) -> c_int =
                load_sym(handle, c"SSL_CTX_set_default_verify_paths")?;
            let SSL_CTX_set_verify: unsafe extern "C" fn(*mut c_void, c_int, *const c_void) =
                load_sym(handle, c"SSL_CTX_set_verify")?;

            let ssl_new = load_sym(handle, c"SSL_new")?;
            let ssl_free = load_sym(handle, c"SSL_free")?;
            let ssl_set_fd = load_sym(handle, c"SSL_set_fd")?;
            let ssl_ctrl = load_sym(handle, c"SSL_ctrl")?;
            let ssl_set1_host = load_sym(handle, c"SSL_set1_host")?;
            let ssl_connect = load_sym(handle, c"SSL_connect")?;
            let ssl_get_verify_result = load_sym(handle, c"SSL_get_verify_result")?;
            let ssl_read = load_sym(handle, c"SSL_read")?;
            let ssl_write = load_sym(handle, c"SSL_write")?;
            let ssl_get_error = load_sym(handle, c"SSL_get_error")?;

            OPENSSL_init_ssl(0, ptr::null());
            let method = TLS_client_method();
            if method.is_null() {
                return Err("OpenSSL TLS_client_method() returned null".into());
            }

            let ctx = SSL_CTX_new(method);
            if ctx.is_null() {
                return Err("OpenSSL SSL_CTX_new() returned null".into());
            }

            if SSL_CTX_set_default_verify_paths(ctx) != 1 {
                SSL_CTX_free(ctx);
                return Err("Failed to load system CA verify paths in OpenSSL".into());
            }
            SSL_CTX_set_verify(ctx, SSL_VERIFY_PEER, ptr::null());

            Ok(Self {
                _handle: handle,
                ctx,
                SSL_new: ssl_new,
                SSL_free: ssl_free,
                SSL_set_fd: ssl_set_fd,
                SSL_ctrl: ssl_ctrl,
                SSL_set1_host: ssl_set1_host,
                SSL_connect: ssl_connect,
                SSL_get_verify_result: ssl_get_verify_result,
                SSL_read: ssl_read,
                SSL_write: ssl_write,
                SSL_get_error: ssl_get_error,
            })
        }
    }

    fn runtime() -> Result<&'static OpenSslRuntime, NetError> {
        let res = TLS_RUNTIME.get_or_init(OpenSslRuntime::load);
        match res {
            Ok(rt) => Ok(rt),
            Err(msg) => Err(NetError {
                message: format!("HTTPS unavailable: {msg}"),
            }),
        }
    }

    pub struct TlsStream {
        ssl: *mut c_void,
        _tcp: TcpStream,
    }

    unsafe impl Send for TlsStream {}

    impl Drop for TlsStream {
        fn drop(&mut self) {
            self.close();
        }
    }

    impl TlsStream {
        pub fn close(&mut self) {
            if !self.ssl.is_null() {
                if let Ok(rt) = runtime() {
                    unsafe {
                        (rt.SSL_free)(self.ssl);
                    }
                }
                self.ssl = ptr::null_mut();
            }
        }

        pub fn connect(tcp: TcpStream, host: &str) -> Result<Self, NetError> {
            Self::connect_within(tcp, host, HANDSHAKE_TIMEOUT)
        }

        /// TLS handshake bounded by one total `budget`. The socket is blocking, so OpenSSL
        /// reports `WANT_READ`/`WANT_WRITE` only when a single socket wait timed out (or was
        /// interrupted); that is not a failure while budget remains (DROP-111). Before every
        /// attempt the socket timeouts are shrunk to the remaining budget, and restored after.
        pub(crate) fn connect_within(
            tcp: TcpStream,
            host: &str,
            budget: Duration,
        ) -> Result<Self, NetError> {
            let rt = runtime()?;
            let clean_host = strip_ipv6_brackets(host);
            let c_host = CString::new(clean_host).map_err(|_| NetError {
                message: format!("Invalid hostname containing NUL byte: '{host}'"),
            })?;

            let ssl = unsafe { (rt.SSL_new)(rt.ctx) };
            if ssl.is_null() {
                return Err(NetError {
                    message: format!("OpenSSL SSL_new() failed for '{host}'"),
                });
            }

            let stream = Self { ssl, _tcp: tcp };
            let fd = stream._tcp.as_raw_fd();
            if unsafe { (rt.SSL_set_fd)(stream.ssl, fd as c_int) } != 1 {
                return Err(NetError {
                    message: format!("OpenSSL SSL_set_fd() failed for '{host}'"),
                });
            }

            if clean_host.parse::<IpAddr>().is_err() {
                let sni_ok = unsafe {
                    (rt.SSL_ctrl)(
                        stream.ssl,
                        SSL_CTRL_SET_TLSEXT_HOSTNAME,
                        TLSEXT_NAMETYPE_host_name,
                        c_host.as_ptr() as *mut c_void,
                    )
                };
                if sni_ok <= 0 {
                    return Err(NetError {
                        message: format!("Failed setting TLS SNI hostname '{host}'"),
                    });
                }
            }

            if unsafe { (rt.SSL_set1_host)(stream.ssl, c_host.as_ptr()) } != 1 {
                return Err(NetError {
                    message: format!("Failed setting TLS verification host '{host}'"),
                });
            }

            let start = Instant::now();
            let deadline = start + budget;
            let idle_read = stream._tcp.read_timeout().ok().flatten();
            let idle_write = stream._tcp.write_timeout().ok().flatten();
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(NetError {
                        message: format!(
                            "TLS handshake with '{host}' timed out after {:.1} s",
                            start.elapsed().as_secs_f64()
                        ),
                    });
                }
                // Zero would mean "block forever" to the OS; keep a 1 ms floor.
                let slice = remaining.max(Duration::from_millis(1));
                let _ = stream._tcp.set_read_timeout(Some(slice));
                let _ = stream._tcp.set_write_timeout(Some(slice));

                let ret = unsafe { (rt.SSL_connect)(stream.ssl) };
                if ret == 1 {
                    break;
                }

                let ssl_err = unsafe { (rt.SSL_get_error)(stream.ssl, ret) };
                if ssl_err == SSL_ERROR_WANT_READ || ssl_err == SSL_ERROR_WANT_WRITE {
                    // A socket wait expired or was interrupted: retry until the deadline.
                    continue;
                }

                let verify_res = unsafe { (rt.SSL_get_verify_result)(stream.ssl) };
                if verify_res != X509_V_OK {
                    return Err(NetError {
                        message: format!(
                            "TLS certificate verification failed for '{host}' (X509 verify code {verify_res})"
                        ),
                    });
                }

                let detail = if ssl_err == SSL_ERROR_SYSCALL {
                    let os_err = std::io::Error::last_os_error();
                    if os_err.raw_os_error().unwrap_or(0) == 0 {
                        "peer closed the connection".to_string()
                    } else {
                        os_err.to_string()
                    }
                } else {
                    format!("SSL error {ssl_err}")
                };
                return Err(NetError {
                    message: format!("TLS handshake with '{host}' failed: {detail}"),
                });
            }
            let _ = stream._tcp.set_read_timeout(idle_read);
            let _ = stream._tcp.set_write_timeout(idle_write);

            let verify_res = unsafe { (rt.SSL_get_verify_result)(stream.ssl) };
            if verify_res != X509_V_OK {
                return Err(NetError {
                    message: format!(
                        "TLS certificate verification failed for '{host}' (X509 verify code {verify_res})"
                    ),
                });
            }

            Ok(stream)
        }
    }

    impl Read for TlsStream {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if buf.is_empty() {
                return Ok(0);
            }
            let rt = runtime().map_err(|e| std::io::Error::other(e.message))?;
            let max_len = buf.len().min(c_int::MAX as usize) as c_int;
            let deadline = Instant::now() + READ_TIMEOUT;

            loop {
                let ret =
                    unsafe { (rt.SSL_read)(self.ssl, buf.as_mut_ptr() as *mut c_void, max_len) };
                if ret > 0 {
                    return Ok(ret as usize);
                }

                let ssl_err = unsafe { (rt.SSL_get_error)(self.ssl, ret) };
                match ssl_err {
                    SSL_ERROR_ZERO_RETURN => return Ok(0),
                    SSL_ERROR_WANT_READ | SSL_ERROR_WANT_WRITE => {
                        let os_err = std::io::Error::last_os_error();
                        if matches!(os_err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
                            || Instant::now() >= deadline
                        {
                            return Err(std::io::Error::new(
                                ErrorKind::TimedOut,
                                "TLS read timed out (no data)",
                            ));
                        }
                        if os_err.kind() == ErrorKind::Interrupted
                            || os_err.raw_os_error().unwrap_or(0) == 0
                        {
                            continue;
                        }
                        return Err(os_err);
                    }
                    SSL_ERROR_SYSCALL => {
                        let os_err = std::io::Error::last_os_error();
                        if os_err.raw_os_error().unwrap_or(0) == 0 {
                            return Ok(0);
                        }
                        return Err(os_err);
                    }
                    other => {
                        return Err(std::io::Error::other(format!(
                            "OpenSSL SSL_read error ({other})"
                        )));
                    }
                }
            }
        }
    }

    impl Write for TlsStream {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if buf.is_empty() {
                return Ok(0);
            }
            let rt = runtime().map_err(|e| std::io::Error::other(e.message))?;
            let max_len = buf.len().min(c_int::MAX as usize) as c_int;
            let deadline = Instant::now() + READ_TIMEOUT;

            loop {
                let ret =
                    unsafe { (rt.SSL_write)(self.ssl, buf.as_ptr() as *const c_void, max_len) };
                if ret > 0 {
                    return Ok(ret as usize);
                }

                let ssl_err = unsafe { (rt.SSL_get_error)(self.ssl, ret) };
                match ssl_err {
                    SSL_ERROR_ZERO_RETURN => {
                        return Err(std::io::Error::new(
                            ErrorKind::BrokenPipe,
                            "TLS connection closed",
                        ));
                    }
                    SSL_ERROR_WANT_READ | SSL_ERROR_WANT_WRITE => {
                        let os_err = std::io::Error::last_os_error();
                        if matches!(os_err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
                            || Instant::now() >= deadline
                        {
                            return Err(std::io::Error::new(
                                ErrorKind::TimedOut,
                                "TLS write timed out",
                            ));
                        }
                        if os_err.kind() == ErrorKind::Interrupted
                            || os_err.raw_os_error().unwrap_or(0) == 0
                        {
                            continue;
                        }
                        return Err(os_err);
                    }
                    SSL_ERROR_SYSCALL => {
                        let os_err = std::io::Error::last_os_error();
                        if os_err.raw_os_error().unwrap_or(0) == 0 {
                            return Err(std::io::Error::new(
                                ErrorKind::BrokenPipe,
                                "TLS syscall EOF on write",
                            ));
                        }
                        return Err(os_err);
                    }
                    other => {
                        return Err(std::io::Error::other(format!(
                            "OpenSSL SSL_write error ({other})"
                        )));
                    }
                }
            }
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::net::TcpListener;

        /// Server that accepts one connection, waits `delay`, optionally writes `reply`, then
        /// holds the socket open until the client goes away.
        fn stalling_server(
            delay: Duration,
            reply: Option<&'static [u8]>,
        ) -> (TcpStream, std::thread::JoinHandle<()>) {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let handle = std::thread::spawn(move || {
                let (mut sock, _) = listener.accept().unwrap();
                std::thread::sleep(delay);
                if let Some(bytes) = reply {
                    let _ = sock.write_all(bytes);
                }
                let mut sink = [0u8; 4096];
                let _ = sock.set_read_timeout(Some(Duration::from_secs(2)));
                while matches!(sock.read(&mut sink), Ok(n) if n > 0) {}
            });
            (TcpStream::connect(addr).unwrap(), handle)
        }

        fn openssl_available() -> bool {
            runtime().is_ok()
        }

        #[test]
        fn silent_server_hits_the_total_handshake_budget() {
            if !openssl_available() {
                return;
            }
            let (tcp, server) = stalling_server(Duration::from_millis(1200), None);
            // The socket's idle timeout (3 s) is longer than the handshake budget (600 ms):
            // the handshake must shrink socket waits to the remaining budget, not overshoot.
            tcp.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let start = Instant::now();
            let err = match TlsStream::connect_within(tcp, "example.test", Duration::from_millis(600)) {
                Err(err) => err,
                Ok(_) => panic!("silent server cannot complete a handshake"),
            };
            let took = start.elapsed();
            assert!(err.message.contains("handshake") && err.message.contains("timed out"), "{}", err.message);
            assert!(took >= Duration::from_millis(550), "gave up early: {took:?}");
            assert!(took < Duration::from_millis(1400), "overshot budget: {took:?}");
            server.join().unwrap();
        }

        #[test]
        fn stall_longer_than_one_socket_wait_is_not_a_timeout() {
            if !openssl_available() {
                return;
            }
            // The peer answers after 500 ms with bytes that are not TLS. The old loop treated the
            // first 100 ms socket timeout as "handshake timed out"; now we must keep waiting,
            // receive the bytes, and fail for the real reason instead.
            let (tcp, server) =
                stalling_server(Duration::from_millis(500), Some(b"HTTP/1.1 400 Bad Request\r\n\r\n"));
            tcp.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
            let start = Instant::now();
            let err = match TlsStream::connect_within(tcp, "example.test", Duration::from_secs(5)) {
                Err(err) => err,
                Ok(_) => panic!("plain-text peer cannot complete a handshake"),
            };
            let took = start.elapsed();
            assert!(took >= Duration::from_millis(450), "gave up before the peer spoke: {took:?}");
            assert!(!err.message.contains("timed out"), "{}", err.message);
            assert!(err.message.contains("TLS handshake with 'example.test' failed"), "{}", err.message);
            server.join().unwrap();
        }
    }
}

#[cfg(target_os = "windows")]
mod imp {
    pub(crate) use crate::tls_schannel::TlsStream;
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod imp {
    use std::io::{ErrorKind, Read, Write};
    use std::net::TcpStream;

    use crate::NetError;

    pub struct TlsStream;

    impl TlsStream {
        pub fn connect(_tcp: TcpStream, host: &str) -> Result<Self, NetError> {
            Err(NetError {
                message: format!(
                    "HTTPS to '{host}' is not yet enabled on this OS (macOS will use a system TLS backend)"
                ),
            })
        }

        #[allow(dead_code)]
        pub fn close(&mut self) {}
    }

    impl Read for TlsStream {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                ErrorKind::Unsupported,
                "Non-Linux TLS is not enabled",
            ))
        }
    }

    impl Write for TlsStream {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                ErrorKind::Unsupported,
                "Non-Linux TLS is not enabled",
            ))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}

pub(crate) use imp::TlsStream;
