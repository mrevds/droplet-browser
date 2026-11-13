//! Zero-dependency OS process sandboxing for Droplet's untrusted renderer workers.
//!
//! Provides a unified [`apply_renderer_sandbox`] and [`apply_process_rlimits`] API
//! backed by:
//! - Linux: `setrlimit`, best-effort `unshare(CLONE_NEWUSER | CLONE_NEWNET)` +
//!   `/proc/self/{setgroups,uid_map,gid_map}`, `PR_SET_NO_NEW_PRIVS`, and pure-Rust
//!   `seccomp-bpf` syscall filtering.
//! - Windows: Job Objects (`JOBOBJECT_EXTENDED_LIMIT_INFORMATION` +
//!   `JOBOBJECT_BASIC_UI_RESTRICTIONS`) and Restricted Tokens (`DISABLE_MAX_PRIVILEGE`).
//! - macOS: `setrlimit` and `sandbox_init` (`kSBXProfileNoNetwork`).

use std::fmt;

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;

/// Isolation profile applied to a renderer worker process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxProfile {
    /// Full renderer containment: resource limits, namespace isolation (best effort),
    /// and strict syscall/token/job-object enforcement.
    StrictRenderer,
    /// Resource limits and non-privileged execution without hard syscall/network lockdown.
    PermissiveFallback,
}

/// Summary of OS containment mechanisms actually enforced for the current process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxReport {
    pub profile: SandboxProfile,
    pub user_ns_isolated: bool,
    pub net_ns_isolated: bool,
    pub seccomp_applied: bool,
    pub rlimits_applied: bool,
    pub job_object_applied: bool,
    pub restricted_token: bool,
    pub notes: String,
}

impl SandboxReport {
    pub(crate) fn empty(profile: SandboxProfile) -> Self {
        Self {
            profile,
            user_ns_isolated: false,
            net_ns_isolated: false,
            seccomp_applied: false,
            rlimits_applied: false,
            job_object_applied: false,
            restricted_token: false,
            notes: String::new(),
        }
    }
}

/// Error returned when a mandatory sandbox step fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxError {
    pub message: String,
}

impl SandboxError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SandboxError {}

/// Default address-space limit (1 GiB) for sandboxed renderer processes.
pub const DEFAULT_RENDERER_MAX_AS_BYTES: u64 = 1024 * 1024 * 1024;
/// Default open file descriptor limit for sandboxed renderer processes.
pub const DEFAULT_RENDERER_MAX_NOFILE: u64 = 64;

/// Apply OS process sandboxing for the requested [`SandboxProfile`].
pub fn apply_renderer_sandbox(profile: SandboxProfile) -> Result<SandboxReport, SandboxError> {
    #[cfg(target_os = "linux")]
    {
        linux::apply_renderer_sandbox(profile)
    }
    #[cfg(target_os = "windows")]
    {
        windows::apply_renderer_sandbox(profile)
    }
    #[cfg(target_os = "macos")]
    {
        macos::apply_renderer_sandbox(profile)
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        let mut report = SandboxReport::empty(profile);
        report.notes = "unsupported OS target; no OS sandbox primitives applied".to_string();
        Ok(report)
    }
}

/// Apply resource limits (`RLIMIT_CORE = 0`, `RLIMIT_NOFILE`, and bounded `RLIMIT_AS`)
/// where supported by the host operating system.
pub fn apply_process_rlimits(max_as_bytes: u64, max_nofile: u64) -> Result<(), SandboxError> {
    #[cfg(target_os = "linux")]
    {
        linux::apply_process_rlimits(max_as_bytes, max_nofile)
    }
    #[cfg(target_os = "macos")]
    {
        macos::apply_process_rlimits(max_as_bytes, max_nofile)
    }
    #[cfg(target_os = "windows")]
    {
        windows::apply_process_rlimits(max_as_bytes, max_nofile)
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        let _ = (max_as_bytes, max_nofile);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_error_displays_message_and_implements_error() {
        let err = SandboxError::new("prctl failed");
        assert_eq!(err.to_string(), "prctl failed");
        let err_ref: &dyn std::error::Error = &err;
        assert_eq!(err_ref.to_string(), "prctl failed");
    }

    #[test]
    fn sandbox_report_empty_defaults_to_false() {
        let report = SandboxReport::empty(SandboxProfile::StrictRenderer);
        assert_eq!(report.profile, SandboxProfile::StrictRenderer);
        assert!(!report.user_ns_isolated);
        assert!(!report.net_ns_isolated);
        assert!(!report.seccomp_applied);
        assert!(!report.rlimits_applied);
        assert!(!report.job_object_applied);
        assert!(!report.restricted_token);
        assert!(report.notes.is_empty());
    }
}
