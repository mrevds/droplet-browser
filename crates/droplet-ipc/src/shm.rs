use crate::IpcError;

pub const MAX_SHM_PIXELS: usize = 64_000_000;

enum Backing {
    #[cfg(target_os = "linux")]
    LinuxMemfd {
        fd: i32,
        ptr: *mut u32,
        byte_len: usize,
    },
    #[cfg(target_os = "windows")]
    WinMapping {
        handle: *mut core::ffi::c_void,
        view: *mut u32,
        name: String,
    },
    OwnedVec(Vec<u32>),
}

pub struct ShmFrameBuffer {
    width: usize,
    height: usize,
    len_pixels: usize,
    backing: Backing,
}

impl std::fmt::Debug for ShmFrameBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShmFrameBuffer")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("len_pixels", &self.len_pixels)
            .field("fd_raw", &self.fd_raw())
            .finish()
    }
}

fn validate_dimensions(width: usize, height: usize) -> Result<(usize, usize), IpcError> {
    if width == 0 || height == 0 {
        return Err(IpcError::InvalidDimensions { width, height });
    }
    let len_pixels = width
        .checked_mul(height)
        .ok_or(IpcError::InvalidDimensions { width, height })?;
    if len_pixels > MAX_SHM_PIXELS {
        return Err(IpcError::InvalidDimensions { width, height });
    }
    let byte_len = len_pixels
        .checked_mul(std::mem::size_of::<u32>())
        .ok_or(IpcError::InvalidDimensions { width, height })?;
    Ok((len_pixels, byte_len))
}

impl ShmFrameBuffer {
    pub fn new(width: usize, height: usize) -> Result<Self, IpcError> {
        #[cfg(target_os = "linux")]
        {
            Self::new_linux_memfd(width, height)
        }
        #[cfg(target_os = "windows")]
        {
            Self::new_windows_mapping(width, height)
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            Self::from_vec(width, height)
        }
    }

    pub fn from_vec(width: usize, height: usize) -> Result<Self, IpcError> {
        let (len_pixels, _) = validate_dimensions(width, height)?;
        Ok(Self {
            width,
            height,
            len_pixels,
            backing: Backing::OwnedVec(vec![0u32; len_pixels]),
        })
    }

    #[cfg(target_os = "linux")]
    pub fn from_raw_fd(fd: i32, width: usize, height: usize) -> Result<Self, IpcError> {
        if fd < 0 {
            return Err(IpcError::Shm(format!("invalid file descriptor: {fd}")));
        }
        let (len_pixels, byte_len) = validate_dimensions(width, height)?;
        // SAFETY: `fd` is non-negative and validated via `lseek` before mapping.
        unsafe {
            let size = linux_ffi::lseek(fd, 0, linux_ffi::SEEK_END);
            if size < 0 {
                return Err(IpcError::Shm(format!(
                    "lseek failed on SHM fd {fd}: {}",
                    std::io::Error::last_os_error()
                )));
            }
            if (size as usize) < byte_len {
                return Err(IpcError::Shm(format!(
                    "SHM fd {fd} size {size} is smaller than required {byte_len} bytes"
                )));
            }
            let _ = linux_ffi::lseek(fd, 0, linux_ffi::SEEK_SET);
            let ptr = linux_ffi::mmap(
                core::ptr::null_mut(),
                byte_len,
                linux_ffi::PROT_READ | linux_ffi::PROT_WRITE,
                linux_ffi::MAP_SHARED,
                fd,
                0,
            );
            if ptr == linux_ffi::map_failed() || ptr.is_null() {
                return Err(IpcError::Shm(format!(
                    "mmap failed on SHM fd {fd}: {}",
                    std::io::Error::last_os_error()
                )));
            }
            Ok(Self {
                width,
                height,
                len_pixels,
                backing: Backing::LinuxMemfd {
                    fd,
                    ptr: ptr.cast::<u32>(),
                    byte_len,
                },
            })
        }
    }

    #[cfg(not(target_os = "linux"))]
    pub fn from_raw_fd(_fd: i32, _width: usize, _height: usize) -> Result<Self, IpcError> {
        Err(IpcError::Shm(
            "from_raw_fd is only supported on Linux".to_string(),
        ))
    }

    pub fn from_env_token(token: &str, width: usize, height: usize) -> Result<Self, IpcError> {
        #[cfg(target_os = "linux")]
        {
            let fd: i32 = token
                .trim()
                .parse()
                .map_err(|_| IpcError::Shm(format!("invalid SHM fd token: {token:?}")))?;
            Self::from_raw_fd(fd, width, height)
        }
        #[cfg(target_os = "windows")]
        {
            Self::open_windows_mapping(token, width, height)
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            let _ = token;
            Self::from_vec(width, height)
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn byte_len(&self) -> usize {
        self.len_pixels * std::mem::size_of::<u32>()
    }

    pub fn fd_raw(&self) -> Option<i32> {
        match &self.backing {
            #[cfg(target_os = "linux")]
            Backing::LinuxMemfd { fd, .. } => Some(*fd),
            _ => None,
        }
    }

    pub fn pixels(&self) -> &[u32] {
        match &self.backing {
            #[cfg(target_os = "linux")]
            Backing::LinuxMemfd { ptr, .. } => {
                // SAFETY: `ptr` points to `len_pixels` valid mapped `u32` elements for `self`'s lifetime.
                unsafe { core::slice::from_raw_parts(*ptr, self.len_pixels) }
            }
            #[cfg(target_os = "windows")]
            Backing::WinMapping { view, .. } => {
                // SAFETY: `view` points to `len_pixels` valid mapped `u32` elements for `self`'s lifetime.
                unsafe { core::slice::from_raw_parts(*view, self.len_pixels) }
            }
            Backing::OwnedVec(vec) => vec,
        }
    }

    pub fn pixels_mut(&mut self) -> &mut [u32] {
        match &mut self.backing {
            #[cfg(target_os = "linux")]
            Backing::LinuxMemfd { ptr, .. } => {
                // SAFETY: `self` holds `&mut` access and `ptr` is valid for `len_pixels` `u32`s.
                unsafe { core::slice::from_raw_parts_mut(*ptr, self.len_pixels) }
            }
            #[cfg(target_os = "windows")]
            Backing::WinMapping { view, .. } => {
                // SAFETY: `self` holds `&mut` access and `view` is valid for `len_pixels` `u32`s.
                unsafe { core::slice::from_raw_parts_mut(*view, self.len_pixels) }
            }
            Backing::OwnedVec(vec) => vec,
        }
    }

    pub fn dup_inheritable_fd(&self) -> Result<i32, IpcError> {
        #[cfg(target_os = "linux")]
        {
            let Backing::LinuxMemfd { fd, .. } = &self.backing else {
                return Err(IpcError::Shm("buffer is not backed by memfd".to_string()));
            };
            // SAFETY: `*fd` is an open file descriptor owned by `self`.
            let duped = unsafe { linux_ffi::fcntl(*fd, linux_ffi::F_DUPFD, 3) };
            if duped < 0 {
                return Err(IpcError::Shm(format!(
                    "fcntl(F_DUPFD) failed: {}",
                    std::io::Error::last_os_error()
                )));
            }
            Ok(duped)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(IpcError::Shm(
                "dup_inheritable_fd is only supported on Linux".to_string(),
            ))
        }
    }

    pub fn prepare_fd_for_child(&self) -> Result<String, IpcError> {
        match &self.backing {
            #[cfg(target_os = "linux")]
            Backing::LinuxMemfd { fd, .. } => {
                // SAFETY: `*fd` is a valid open memfd; clearing FD_CLOEXEC allows `exec` inheritance.
                unsafe {
                    let flags = linux_ffi::fcntl(*fd, linux_ffi::F_GETFD, 0);
                    if flags < 0 {
                        return Err(IpcError::Shm(format!(
                            "fcntl(F_GETFD) failed: {}",
                            std::io::Error::last_os_error()
                        )));
                    }
                    if linux_ffi::fcntl(*fd, linux_ffi::F_SETFD, flags & !linux_ffi::FD_CLOEXEC) < 0 {
                        return Err(IpcError::Shm(format!(
                            "fcntl(F_SETFD) failed: {}",
                            std::io::Error::last_os_error()
                        )));
                    }
                }
                Ok(fd.to_string())
            }
            #[cfg(target_os = "windows")]
            Backing::WinMapping { name, .. } => Ok(name.clone()),
            Backing::OwnedVec(_) => Err(IpcError::Shm(
                "OwnedVec framebuffer cannot be shared across processes".to_string(),
            )),
        }
    }

    #[cfg(target_os = "linux")]
    fn new_linux_memfd(width: usize, height: usize) -> Result<Self, IpcError> {
        let (len_pixels, byte_len) = validate_dimensions(width, height)?;
        let name = b"droplet-ipc-fb\0";
        // SAFETY: null-terminated static byte string and valid `MFD_CLOEXEC` flag.
        unsafe {
            let fd = linux_ffi::memfd_create(
                name.as_ptr().cast::<core::ffi::c_char>(),
                linux_ffi::MFD_CLOEXEC,
            );
            if fd < 0 {
                return Err(IpcError::Shm(format!(
                    "memfd_create failed: {}",
                    std::io::Error::last_os_error()
                )));
            }
            if linux_ffi::ftruncate(fd, byte_len as i64) < 0 {
                let err = std::io::Error::last_os_error();
                linux_ffi::close(fd);
                return Err(IpcError::Shm(format!("ftruncate failed: {err}")));
            }
            let ptr = linux_ffi::mmap(
                core::ptr::null_mut(),
                byte_len,
                linux_ffi::PROT_READ | linux_ffi::PROT_WRITE,
                linux_ffi::MAP_SHARED,
                fd,
                0,
            );
            if ptr == linux_ffi::map_failed() || ptr.is_null() {
                let err = std::io::Error::last_os_error();
                linux_ffi::close(fd);
                return Err(IpcError::Shm(format!("mmap failed: {err}")));
            }
            Ok(Self {
                width,
                height,
                len_pixels,
                backing: Backing::LinuxMemfd {
                    fd,
                    ptr: ptr.cast::<u32>(),
                    byte_len,
                },
            })
        }
    }

    #[cfg(target_os = "windows")]
    fn new_windows_mapping(width: usize, height: usize) -> Result<Self, IpcError> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);

        let (len_pixels, byte_len) = validate_dimensions(width, height)?;
        let seq = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        // SAFETY: `GetCurrentProcessId` takes no parameters and cannot fail.
        let pid = unsafe { win_ffi::GetCurrentProcessId() };
        let name = format!("Local\\droplet-ipc-fb-{pid}-{seq}");
        let wide = win_ffi::to_wide(&name);

        let size_u64 = byte_len as u64;
        let high = (size_u64 >> 32) as u32;
        let low = size_u64 as u32;

        // SAFETY: valid mapping parameters and null-terminated UTF-16 name.
        unsafe {
            let handle = win_ffi::CreateFileMappingW(
                win_ffi::invalid_handle(),
                core::ptr::null(),
                win_ffi::PAGE_READWRITE,
                high,
                low,
                wide.as_ptr(),
            );
            if handle.is_null() {
                return Err(IpcError::Shm(format!(
                    "CreateFileMappingW failed: {}",
                    std::io::Error::last_os_error()
                )));
            }
            let view = win_ffi::MapViewOfFile(handle, win_ffi::FILE_MAP_ALL_ACCESS, 0, 0, byte_len);
            if view.is_null() {
                let err = std::io::Error::last_os_error();
                win_ffi::CloseHandle(handle);
                return Err(IpcError::Shm(format!("MapViewOfFile failed: {err}")));
            }
            Ok(Self {
                width,
                height,
                len_pixels,
                backing: Backing::WinMapping {
                    handle,
                    view: view.cast::<u32>(),
                    name,
                },
            })
        }
    }

    #[cfg(target_os = "windows")]
    fn open_windows_mapping(name: &str, width: usize, height: usize) -> Result<Self, IpcError> {
        let (len_pixels, byte_len) = validate_dimensions(width, height)?;
        let wide = win_ffi::to_wide(name);
        // SAFETY: valid access flags and null-terminated UTF-16 mapping name.
        unsafe {
            let handle = win_ffi::OpenFileMappingW(win_ffi::FILE_MAP_ALL_ACCESS, 0, wide.as_ptr());
            if handle.is_null() {
                return Err(IpcError::Shm(format!(
                    "OpenFileMappingW failed for {name:?}: {}",
                    std::io::Error::last_os_error()
                )));
            }
            let view = win_ffi::MapViewOfFile(handle, win_ffi::FILE_MAP_ALL_ACCESS, 0, 0, byte_len);
            if view.is_null() {
                let err = std::io::Error::last_os_error();
                win_ffi::CloseHandle(handle);
                return Err(IpcError::Shm(format!("MapViewOfFile failed: {err}")));
            }
            Ok(Self {
                width,
                height,
                len_pixels,
                backing: Backing::WinMapping {
                    handle,
                    view: view.cast::<u32>(),
                    name: name.to_string(),
                },
            })
        }
    }
}

impl Drop for ShmFrameBuffer {
    fn drop(&mut self) {
        match &mut self.backing {
            #[cfg(target_os = "linux")]
            Backing::LinuxMemfd { fd, ptr, byte_len } => unsafe {
                if !ptr.is_null() && *byte_len > 0 {
                    let _ = linux_ffi::munmap(ptr.cast::<core::ffi::c_void>(), *byte_len);
                    *ptr = core::ptr::null_mut();
                }
                if *fd >= 0 {
                    let _ = linux_ffi::close(*fd);
                    *fd = -1;
                }
            },
            #[cfg(target_os = "windows")]
            Backing::WinMapping { handle, view, .. } => unsafe {
                if !view.is_null() {
                    let _ = win_ffi::UnmapViewOfFile(view.cast::<core::ffi::c_void>());
                    *view = core::ptr::null_mut();
                }
                if !handle.is_null() {
                    let _ = win_ffi::CloseHandle(*handle);
                    *handle = core::ptr::null_mut();
                }
            },
            Backing::OwnedVec(_) => {}
        }
    }
}

#[cfg(target_os = "linux")]
mod linux_ffi {
    use core::ffi::{c_char, c_int, c_void};

    pub const MFD_CLOEXEC: u32 = 0x0001;
    pub const PROT_READ: c_int = 1;
    pub const PROT_WRITE: c_int = 2;
    pub const MAP_SHARED: c_int = 1;
    pub const SEEK_SET: c_int = 0;
    pub const SEEK_END: c_int = 2;
    pub const F_DUPFD: c_int = 0;
    pub const F_GETFD: c_int = 1;
    pub const F_SETFD: c_int = 2;
    pub const FD_CLOEXEC: c_int = 1;

    pub fn map_failed() -> *mut c_void {
        usize::MAX as *mut c_void
    }

    extern "C" {
        pub fn memfd_create(name: *const c_char, flags: u32) -> c_int;
        pub fn ftruncate(fd: c_int, length: i64) -> c_int;
        pub fn lseek(fd: c_int, offset: i64, whence: c_int) -> i64;
        pub fn mmap(
            addr: *mut c_void,
            len: usize,
            prot: c_int,
            flags: c_int,
            fd: c_int,
            offset: i64,
        ) -> *mut c_void;
        pub fn munmap(addr: *mut c_void, len: usize) -> c_int;
        pub fn close(fd: c_int) -> c_int;
        pub fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
    }
}

#[cfg(target_os = "windows")]
mod win_ffi {
    use core::ffi::c_void;

    pub const PAGE_READWRITE: u32 = 0x04;
    pub const FILE_MAP_ALL_ACCESS: u32 = 0x000F_001F;

    pub fn invalid_handle() -> *mut c_void {
        (-1isize) as *mut c_void
    }

    pub fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(core::iter::once(0)).collect()
    }

    #[link(name = "kernel32")]
    extern "system" {
        pub fn GetCurrentProcessId() -> u32;
        pub fn CreateFileMappingW(
            hFile: *mut c_void,
            lpFileMappingAttributes: *const c_void,
            flProtect: u32,
            dwMaximumSizeHigh: u32,
            dwMaximumSizeLow: u32,
            lpName: *const u16,
        ) -> *mut c_void;
        pub fn OpenFileMappingW(
            dwDesiredAccess: u32,
            bInheritHandle: i32,
            lpName: *const u16,
        ) -> *mut c_void;
        pub fn MapViewOfFile(
            hFileMappingObject: *mut c_void,
            dwDesiredAccess: u32,
            dwFileOffsetHigh: u32,
            dwFileOffsetLow: u32,
            dwNumberOfBytesToMap: usize,
        ) -> *mut c_void;
        pub fn UnmapViewOfFile(lpBaseAddress: *const c_void) -> i32;
        pub fn CloseHandle(hObject: *mut c_void) -> i32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_and_oversized_dimensions() {
        assert_eq!(
            ShmFrameBuffer::new(0, 100).unwrap_err(),
            IpcError::InvalidDimensions {
                width: 0,
                height: 100
            }
        );
        assert_eq!(
            ShmFrameBuffer::new(100, 0).unwrap_err(),
            IpcError::InvalidDimensions {
                width: 100,
                height: 0
            }
        );
        assert_eq!(
            ShmFrameBuffer::new(10_000, 10_000).unwrap_err(),
            IpcError::InvalidDimensions {
                width: 10_000,
                height: 10_000
            }
        );
    }

    #[test]
    fn from_vec_allows_reading_and_writing_pixels() {
        let mut fb = ShmFrameBuffer::from_vec(4, 3).expect("from_vec");
        assert_eq!(fb.width(), 4);
        assert_eq!(fb.height(), 3);
        assert_eq!(fb.byte_len(), 48);
        fb.pixels_mut()[0] = 0xFF11_2233;
        fb.pixels_mut()[11] = 0xFFAA_BBCC;
        assert_eq!(fb.pixels()[0], 0xFF11_2233);
        assert_eq!(fb.pixels()[11], 0xFFAA_BBCC);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_memfd_shares_pixels_across_fork_and_pipe() {
        extern "C" {
            fn fork() -> i32;
            fn pipe(fds: *mut i32) -> i32;
            fn read(fd: i32, buf: *mut core::ffi::c_void, count: usize) -> isize;
            fn write(fd: i32, buf: *const core::ffi::c_void, count: usize) -> isize;
            fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
            fn _exit(status: i32) -> !;
        }

        let fb = ShmFrameBuffer::new(16, 16).expect("memfd create");
        let token = fb.prepare_fd_for_child().expect("prepare fd");
        let raw_fd = fb.fd_raw().expect("linux memfd fd");
        assert_eq!(token, raw_fd.to_string());

        let dup_fd = fb.dup_inheritable_fd().expect("dup inheritable fd");
        assert!(dup_fd >= 3);

        let mut pipe_fds = [0i32; 2];
        assert_eq!(unsafe { pipe(pipe_fds.as_mut_ptr()) }, 0);

        let pid = unsafe { fork() };
        assert!(pid >= 0);

        if pid == 0 {
            unsafe {
                linux_ffi::close(pipe_fds[0]);
            }
            let attached = ShmFrameBuffer::from_raw_fd(dup_fd, 16, 16);
            let ok = match attached {
                Ok(mut child_fb) => {
                    child_fb.pixels_mut()[0] = 0xFFDE_ADBE;
                    child_fb.pixels_mut()[255] = 0xFFCA_FEBA;
                    1u8
                }
                Err(_) => 0u8,
            };
            unsafe {
                write(pipe_fds[1], (&ok as *const u8).cast(), 1);
                linux_ffi::close(pipe_fds[1]);
                _exit(0);
            }
        }

        unsafe {
            linux_ffi::close(pipe_fds[1]);
            linux_ffi::close(dup_fd);
        }
        let mut ack = 0u8;
        let n = unsafe { read(pipe_fds[0], (&mut ack as *mut u8).cast(), 1) };
        unsafe {
            linux_ffi::close(pipe_fds[0]);
        }
        let mut status = 0i32;
        unsafe { waitpid(pid, &mut status, 0) };

        assert_eq!(n, 1);
        assert_eq!(ack, 1);
        assert_eq!(fb.pixels()[0], 0xFFDE_ADBE);
        assert_eq!(fb.pixels()[255], 0xFFCA_FEBA);
    }
}
