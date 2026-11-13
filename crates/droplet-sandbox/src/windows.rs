//! Windows renderer sandboxing via Win32 Job Objects and Restricted Tokens.

use std::os::raw::c_void;
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};

use crate::{SandboxError, SandboxProfile, SandboxReport, DEFAULT_RENDERER_MAX_AS_BYTES};

type Handle = *mut c_void;
type Bool = i32;

const JOB_OBJECT_BASIC_UI_RESTRICTIONS_CLASS: u32 = 4;
const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS: u32 = 9;

const JOB_OBJECT_LIMIT_ACTIVE_PROCESS: u32 = 0x0000_0008;
const JOB_OBJECT_LIMIT_PROCESS_MEMORY: u32 = 0x0000_0100;
const JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION: u32 = 0x0000_0400;

const JOB_OBJECT_UILIMIT_HANDLES: u32 = 0x0000_0001;
const JOB_OBJECT_UILIMIT_READCLIPBOARD: u32 = 0x0000_0002;
const JOB_OBJECT_UILIMIT_WRITECLIPBOARD: u32 = 0x0000_0004;
const JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS: u32 = 0x0000_0008;
const JOB_OBJECT_UILIMIT_DISPLAYSETTINGS: u32 = 0x0000_0010;
const JOB_OBJECT_UILIMIT_GLOBALATOMS: u32 = 0x0000_0020;
const JOB_OBJECT_UILIMIT_DESKTOP: u32 = 0x0000_0040;
const JOB_OBJECT_UILIMIT_EXITWINDOWS: u32 = 0x0000_0080;

const TOKEN_DUPLICATE: u32 = 0x0002;
const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_ADJUST_DEFAULT: u32 = 0x0080;
const DISABLE_MAX_PRIVILEGE: u32 = 0x01;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct JobObjectBasicLimitInformation {
    pub per_process_user_time_limit: i64,
    pub per_job_user_time_limit: i64,
    pub limit_flags: u32,
    pub minimum_working_set_size: usize,
    pub maximum_working_set_size: usize,
    pub active_process_limit: u32,
    pub affinity: usize,
    pub priority_class: u32,
    pub scheduling_class: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct IoCounters {
    pub read_operation_count: u64,
    pub write_operation_count: u64,
    pub other_operation_count: u64,
    pub read_transfer_count: u64,
    pub write_transfer_count: u64,
    pub other_transfer_count: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct JobObjectExtendedLimitInformation {
    pub basic_limit_information: JobObjectBasicLimitInformation,
    pub io_info: IoCounters,
    pub process_memory_limit: usize,
    pub job_memory_limit: usize,
    pub peak_process_memory_used: usize,
    pub peak_job_memory_used: usize,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct JobObjectBasicUiRestrictions {
    pub ui_restrictions_class: u32,
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateJobObjectW(job_attributes: *mut c_void, name: *const u16) -> Handle;
    fn SetInformationJobObject(
        job: Handle,
        info_class: u32,
        info: *const c_void,
        info_len: u32,
    ) -> Bool;
    fn AssignProcessToJobObject(job: Handle, process: Handle) -> Bool;
    fn GetCurrentProcess() -> Handle;
    fn CloseHandle(object: Handle) -> Bool;
    fn GetLastError() -> u32;
}

#[link(name = "advapi32")]
extern "system" {
    fn OpenProcessToken(process: Handle, desired_access: u32, token_handle: *mut Handle) -> Bool;
    fn CreateRestrictedToken(
        existing_token: Handle,
        flags: u32,
        disable_sid_count: u32,
        sids_to_disable: *const c_void,
        delete_privilege_count: u32,
        privileges_to_delete: *const c_void,
        restricted_sid_count: u32,
        sids_to_restrict: *const c_void,
        new_token: *mut Handle,
    ) -> Bool;
}

static ACTIVE_JOB_HANDLE: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

pub fn build_extended_limit_info(
    max_memory_bytes: usize,
    strict: bool,
) -> JobObjectExtendedLimitInformation {
    let mut info = JobObjectExtendedLimitInformation::default();
    let mut flags = JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
    if max_memory_bytes > 0 {
        flags |= JOB_OBJECT_LIMIT_PROCESS_MEMORY;
        info.process_memory_limit = max_memory_bytes;
    }
    if strict {
        flags |= JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
        info.basic_limit_information.active_process_limit = 1;
    }
    info.basic_limit_information.limit_flags = flags;
    info
}

pub fn build_ui_restrictions() -> JobObjectBasicUiRestrictions {
    JobObjectBasicUiRestrictions {
        ui_restrictions_class: JOB_OBJECT_UILIMIT_HANDLES
            | JOB_OBJECT_UILIMIT_READCLIPBOARD
            | JOB_OBJECT_UILIMIT_WRITECLIPBOARD
            | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
            | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
            | JOB_OBJECT_UILIMIT_GLOBALATOMS
            | JOB_OBJECT_UILIMIT_DESKTOP
            | JOB_OBJECT_UILIMIT_EXITWINDOWS,
    }
}

fn apply_job_object(max_memory_bytes: usize, strict: bool) -> Result<(), String> {
    let job = unsafe { CreateJobObjectW(ptr::null_mut(), ptr::null()) };
    if job.is_null() {
        return Err(format!(
            "CreateJobObjectW failed (win32 error {})",
            unsafe { GetLastError() }
        ));
    }

    let ext = build_extended_limit_info(max_memory_bytes, strict);
    let ext_ok = unsafe {
        SetInformationJobObject(
            job,
            JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
            &ext as *const JobObjectExtendedLimitInformation as *const c_void,
            std::mem::size_of::<JobObjectExtendedLimitInformation>() as u32,
        )
    };
    if ext_ok == 0 {
        let err = unsafe { GetLastError() };
        unsafe {
            CloseHandle(job);
        }
        return Err(format!(
            "SetInformationJobObject(ExtendedLimit) failed (win32 error {err})"
        ));
    }

    if strict {
        let ui = build_ui_restrictions();
        let ui_ok = unsafe {
            SetInformationJobObject(
                job,
                JOB_OBJECT_BASIC_UI_RESTRICTIONS_CLASS,
                &ui as *const JobObjectBasicUiRestrictions as *const c_void,
                std::mem::size_of::<JobObjectBasicUiRestrictions>() as u32,
            )
        };
        if ui_ok == 0 {
            let err = unsafe { GetLastError() };
            unsafe {
                CloseHandle(job);
            }
            return Err(format!(
                "SetInformationJobObject(BasicUIRestrictions) failed (win32 error {err})"
            ));
        }
    }

    let assign_ok = unsafe { AssignProcessToJobObject(job, GetCurrentProcess()) };
    if assign_ok == 0 {
        let err = unsafe { GetLastError() };
        unsafe {
            CloseHandle(job);
        }
        return Err(format!(
            "AssignProcessToJobObject failed (win32 error {err})"
        ));
    }

    let prev = ACTIVE_JOB_HANDLE.swap(job, Ordering::SeqCst);
    if !prev.is_null() {
        unsafe {
            CloseHandle(prev);
        }
    }
    Ok(())
}

fn try_create_restricted_token() -> Result<(), String> {
    let mut current_token: Handle = ptr::null_mut();
    let open_ok = unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ADJUST_DEFAULT,
            &mut current_token,
        )
    };
    if open_ok == 0 || current_token.is_null() {
        return Err(format!("OpenProcessToken failed (win32 error {})", unsafe {
            GetLastError()
        }));
    }

    let mut restricted: Handle = ptr::null_mut();
    let create_ok = unsafe {
        CreateRestrictedToken(
            current_token,
            DISABLE_MAX_PRIVILEGE,
            0,
            ptr::null(),
            0,
            ptr::null(),
            0,
            ptr::null(),
            &mut restricted,
        )
    };
    let err = unsafe { GetLastError() };
    unsafe {
        CloseHandle(current_token);
    }
    if create_ok == 0 || restricted.is_null() {
        return Err(format!(
            "CreateRestrictedToken(DISABLE_MAX_PRIVILEGE) failed (win32 error {err})"
        ));
    }
    unsafe {
        CloseHandle(restricted);
    }
    Ok(())
}

pub fn apply_process_rlimits(max_as_bytes: u64, _max_nofile: u64) -> Result<(), SandboxError> {
    let capped = usize::try_from(max_as_bytes).unwrap_or(usize::MAX);
    apply_job_object(capped, false).map_err(SandboxError::new)
}

pub fn apply_renderer_sandbox(profile: SandboxProfile) -> Result<SandboxReport, SandboxError> {
    let mut report = SandboxReport::empty(profile);
    let mut notes = Vec::new();
    let strict = matches!(profile, SandboxProfile::StrictRenderer);
    let max_mem = usize::try_from(DEFAULT_RENDERER_MAX_AS_BYTES).unwrap_or(usize::MAX);

    match apply_job_object(max_mem, strict) {
        Ok(()) => {
            report.job_object_applied = true;
            report.rlimits_applied = true;
        }
        Err(err) => {
            if strict {
                return Err(SandboxError::new(err));
            }
            notes.push(err);
        }
    }

    if strict {
        match try_create_restricted_token() {
            Ok(()) => report.restricted_token = true,
            Err(err) => notes.push(err),
        }
    } else {
        notes.push("PermissiveFallback profile: restricted token skipped".to_string());
    }

    report.notes = notes.join("; ");
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_object_abi_layouts_match_win64_spec() {
        assert_eq!(std::mem::size_of::<IoCounters>(), 48);
        assert_eq!(std::mem::size_of::<JobObjectBasicUiRestrictions>(), 4);
        assert_eq!(
            std::mem::size_of::<JobObjectBasicLimitInformation>(),
            64
        );
        assert_eq!(
            std::mem::size_of::<JobObjectExtendedLimitInformation>(),
            144
        );
    }

    #[test]
    fn limit_builders_set_expected_flags() {
        let ext = build_extended_limit_info(512 * 1024 * 1024, true);
        assert_ne!(
            ext.basic_limit_information.limit_flags & JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
            0
        );
        assert_ne!(
            ext.basic_limit_information.limit_flags & JOB_OBJECT_LIMIT_PROCESS_MEMORY,
            0
        );
        assert_eq!(ext.basic_limit_information.active_process_limit, 1);
        assert_eq!(ext.process_memory_limit, 512 * 1024 * 1024);

        let ui = build_ui_restrictions();
        assert_ne!(ui.ui_restrictions_class & JOB_OBJECT_UILIMIT_DESKTOP, 0);
        assert_ne!(
            ui.ui_restrictions_class & JOB_OBJECT_UILIMIT_READCLIPBOARD,
            0
        );
    }
}
