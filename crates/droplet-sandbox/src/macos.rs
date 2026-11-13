//! macOS renderer sandboxing via `setrlimit` and Seatbelt (`sandbox_init`).

use std::ffi::CStr;
use std::os::raw::{c_char, c_int};
use std::ptr;

use crate::{
    SandboxError, SandboxProfile, SandboxReport, DEFAULT_RENDERER_MAX_AS_BYTES,
    DEFAULT_RENDERER_MAX_NOFILE,
};

// Resource limit constants (Darwin/macOS ABI)
const RLIMIT_CORE: c_int = 4;
const RLIMIT_AS: c_int = 5;
const RLIMIT_NOFILE: c_int = 8;
const RLIM_INFINITY: u64 = u64::MAX;

const SANDBOX_NAMED: u64 = 0x0001;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct RLimit {
    rlim_cur: u64,
    rlim_max: u64,
}

extern "C" {
    static kSBXProfileNoNetwork: *const c_char;
    fn getrlimit(resource: c_int, rlp: *mut RLimit) -> c_int;
    fn setrlimit(resource: c_int, rlp: *const RLimit) -> c_int;
    fn sandbox_init(profile: *const c_char, flags: u64, errorbuf: *mut *mut c_char) -> c_int;
    fn sandbox_free_error(errorbuf: *mut c_char);
}

fn lower_rlimit(resource: c_int, desired: u64, name: &str) -> Result<(), SandboxError> {
    let mut current = RLimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe { getrlimit(resource, &mut current) } != 0 {
        return Err(SandboxError::new(format!("getrlimit({name}) failed")));
    }
    let capped = if current.rlim_max == RLIM_INFINITY {
        desired
    } else {
        desired.min(current.rlim_max)
    };
    let next = RLimit {
        rlim_cur: capped,
        rlim_max: capped,
    };
    if unsafe { setrlimit(resource, &next) } != 0 {
        return Err(SandboxError::new(format!("setrlimit({name}) failed")));
    }
    Ok(())
}

pub fn apply_process_rlimits(max_as_bytes: u64, max_nofile: u64) -> Result<(), SandboxError> {
    lower_rlimit(RLIMIT_CORE, 0, "RLIMIT_CORE")?;
    if max_nofile > 0 {
        lower_rlimit(RLIMIT_NOFILE, max_nofile, "RLIMIT_NOFILE")?;
    }
    if max_as_bytes > 0 {
        lower_rlimit(RLIMIT_AS, max_as_bytes, "RLIMIT_AS")?;
    }
    Ok(())
}

pub fn apply_renderer_sandbox(profile: SandboxProfile) -> Result<SandboxReport, SandboxError> {
    let mut report = SandboxReport::empty(profile);

    apply_process_rlimits(DEFAULT_RENDERER_MAX_AS_BYTES, DEFAULT_RENDERER_MAX_NOFILE)?;
    report.rlimits_applied = true;

    match profile {
        SandboxProfile::StrictRenderer => {
            let mut err_buf: *mut c_char = ptr::null_mut();
            let rc = unsafe { sandbox_init(kSBXProfileNoNetwork, SANDBOX_NAMED, &mut err_buf) };
            if rc != 0 {
                let reason = if err_buf.is_null() {
                    "sandbox_init(kSBXProfileNoNetwork) failed".to_string()
                } else {
                    let msg = unsafe { CStr::from_ptr(err_buf) }
                        .to_string_lossy()
                        .into_owned();
                    unsafe {
                        sandbox_free_error(err_buf);
                    }
                    format!("sandbox_init(kSBXProfileNoNetwork) failed: {msg}")
                };
                return Err(SandboxError::new(reason));
            }
            report.net_ns_isolated = true;
        }
        SandboxProfile::PermissiveFallback => {
            report.notes = "PermissiveFallback profile: sandbox_init skipped".to_string();
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn darwin_rlimit_struct_size_is_16_bytes() {
        assert_eq!(std::mem::size_of::<RLimit>(), 16);
    }
}
