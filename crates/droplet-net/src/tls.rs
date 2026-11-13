#[cfg(target_os = "linux")]
mod imp {
    use std::ffi::{CStr, CString};
    use std::io::{ErrorKind, Read, Write};
    use std::net::{IpAddr, TcpStream};
    use std::os::raw::{c_char, c_int, c_long, c_void};
    use std::os::unix::io::AsRawFd;
    use std::ptr;
    use std::sync::OnceLock;
    use std::time::Instant;

    use crate::dns::strip_ipv6_brackets;
    use crate::{NetError, HTTP_TIMEOUT};

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

            let deadline = Instant::now() + HTTP_TIMEOUT;
            loop {
                let ret = unsafe { (rt.SSL_connect)(stream.ssl) };
                if ret == 1 {
                    break;
                }

                let ssl_err = unsafe { (rt.SSL_get_error)(stream.ssl, ret) };
                if ssl_err == SSL_ERROR_WANT_READ || ssl_err == SSL_ERROR_WANT_WRITE {
                    let os_err = std::io::Error::last_os_error();
                    if matches!(os_err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
                        || Instant::now() >= deadline
                    {
                        return Err(NetError {
                            message: format!("TLS handshake timed out for '{host}'"),
                        });
                    }
                    if os_err.kind() == ErrorKind::Interrupted
                        || os_err.raw_os_error().unwrap_or(0) == 0
                    {
                        continue;
                    }
                    return Err(NetError {
                        message: format!("TLS handshake I/O error for '{host}': {os_err}"),
                    });
                }

                let verify_res = unsafe { (rt.SSL_get_verify_result)(stream.ssl) };
                if verify_res != X509_V_OK {
                    return Err(NetError {
                        message: format!(
                            "TLS certificate verification failed for '{host}' (X509 verify code {verify_res})"
                        ),
                    });
                }

                return Err(NetError {
                    message: format!("TLS handshake failed for '{host}' (SSL error {ssl_err})"),
                });
            }

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
            let deadline = Instant::now() + HTTP_TIMEOUT;

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
                                "TLS read timed out",
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
            let deadline = Instant::now() + HTTP_TIMEOUT;

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
