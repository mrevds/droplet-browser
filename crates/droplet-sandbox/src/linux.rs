//! Linux renderer sandboxing: `setrlimit`, best-effort user/net namespaces,
//! `PR_SET_NO_NEW_PRIVS`, and pure-Rust `seccomp-bpf` syscall filtering.

use std::fs;
use std::os::raw::{c_int, c_ulong};

use crate::{
    SandboxError, SandboxProfile, SandboxReport, DEFAULT_RENDERER_MAX_AS_BYTES,
    DEFAULT_RENDERER_MAX_NOFILE,
};

// Resource limit constants (Linux ABI)
const RLIMIT_CORE: c_int = 4;
const RLIMIT_NOFILE: c_int = 7;
const RLIMIT_AS: c_int = 9;
const RLIM_INFINITY: u64 = u64::MAX;

// Namespace / prctl constants
const CLONE_NEWUSER: c_int = 0x1000_0000;
const CLONE_NEWNET: c_int = 0x4000_0000;
const PR_SET_SECCOMP: c_int = 22;
const PR_SET_NO_NEW_PRIVS: c_int = 38;
const SECCOMP_MODE_FILTER: c_ulong = 2;

// Errno & BPF constants
const EPERM: u32 = 1;
const BPF_LD: u16 = 0x00;
const BPF_JMP: u16 = 0x05;
const BPF_RET: u16 = 0x06;
const BPF_W: u16 = 0x00;
const BPF_ABS: u16 = 0x20;
const BPF_JEQ: u16 = 0x10;
const BPF_K: u16 = 0x00;

const BPF_LD_W_ABS: u16 = BPF_LD | BPF_W | BPF_ABS;
const BPF_JMP_JEQ_K: u16 = BPF_JMP | BPF_JEQ | BPF_K;
const BPF_RET_K: u16 = BPF_RET | BPF_K;

const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
const SECCOMP_RET_ALLOW: u32 = 0x7FFF_0000;
const SECCOMP_RET_ERRNO_EPERM: u32 = SECCOMP_RET_ERRNO | EPERM;

const AUDIT_ARCH_X86_64: u32 = 62 | 0x8000_0000 | 0x4000_0000;
const AUDIT_ARCH_AARCH64: u32 = 183 | 0x8000_0000 | 0x4000_0000;

const SECCOMP_DATA_NR_OFFSET: u32 = 0;
const SECCOMP_DATA_ARCH_OFFSET: u32 = 4;
const MAX_BPF_INSNS: usize = 128;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RLimit {
    rlim_cur: u64,
    rlim_max: u64,
}

/// A single classic BPF instruction (`struct sock_filter`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SockFilter {
    pub code: u16,
    pub jt: u8,
    pub jf: u8,
    pub k: u32,
}

impl SockFilter {
    const fn stmt(code: u16, k: u32) -> Self {
        Self {
            code,
            jt: 0,
            jf: 0,
            k,
        }
    }

    const fn jump(code: u16, k: u32, jt: u8, jf: u8) -> Self {
        Self { code, jt, jf, k }
    }
}

#[repr(C)]
struct SockFprog {
    len: u16,
    filter: *const SockFilter,
}

extern "C" {
    fn getrlimit(resource: c_int, rlim: *mut RLimit) -> c_int;
    fn setrlimit(resource: c_int, rlim: *const RLimit) -> c_int;
    fn unshare(flags: c_int) -> c_int;
    fn geteuid() -> u32;
    fn getegid() -> u32;
    fn prctl(
        option: c_int,
        arg2: c_ulong,
        arg3: c_ulong,
        arg4: c_ulong,
        arg5: c_ulong,
    ) -> c_int;
    fn __errno_location() -> *mut c_int;
}

fn last_os_errno() -> i32 {
    unsafe { *__errno_location() }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BpfArch {
    X86_64,
    Aarch64,
}

impl BpfArch {
    #[cfg(target_arch = "x86_64")]
    pub const fn host() -> Self {
        Self::X86_64
    }

    #[cfg(target_arch = "aarch64")]
    pub const fn host() -> Self {
        Self::Aarch64
    }

    pub const fn audit_arch(self) -> u32 {
        match self {
            Self::X86_64 => AUDIT_ARCH_X86_64,
            Self::Aarch64 => AUDIT_ARCH_AARCH64,
        }
    }

    pub const fn allowed_syscalls(self) -> &'static [u32] {
        match self {
            Self::X86_64 => X86_64_ALLOWED_SYSCALLS,
            Self::Aarch64 => AARCH64_ALLOWED_SYSCALLS,
        }
    }

    pub const fn denied_syscalls(self) -> &'static [u32] {
        match self {
            Self::X86_64 => X86_64_DENIED_SYSCALLS,
            Self::Aarch64 => AARCH64_DENIED_SYSCALLS,
        }
    }
}

// Verified against /usr/include/x86_64-linux-gnu/asm/unistd_64.h
const X86_64_ALLOWED_SYSCALLS: &[u32] = &[
    0,   // read
    1,   // write
    3,   // close
    5,   // fstat
    7,   // poll
    8,   // lseek
    9,   // mmap
    10,  // mprotect
    11,  // munmap
    12,  // brk
    13,  // rt_sigaction
    14,  // rt_sigprocmask
    15,  // rt_sigreturn
    17,  // pread64
    19,  // readv
    20,  // writev
    24,  // sched_yield
    25,  // mremap
    28,  // madvise
    35,  // nanosleep
    39,  // getpid
    60,  // exit
    72,  // fcntl
    102, // getuid
    104, // getgid
    107, // geteuid
    108, // getegid
    131, // sigaltstack
    186, // gettid
    202, // futex
    204, // sched_getaffinity
    219, // restart_syscall
    228, // clock_gettime
    230, // clock_nanosleep
    231, // exit_group
    262, // newfstatat
    271, // ppoll
    273, // set_robust_list
    274, // get_robust_list
    318, // getrandom
    332, // statx
    334, // rseq
];

const X86_64_DENIED_SYSCALLS: &[u32] = &[
    2,   // open
    41,  // socket
    42,  // connect
    43,  // accept
    49,  // bind
    50,  // listen
    53,  // socketpair
    56,  // clone
    57,  // fork
    58,  // vfork
    59,  // execve
    62,  // kill
    101, // ptrace
    257, // openat
    288, // accept4
    322, // execveat
    435, // clone3
    437, // openat2
];

// Verified against /usr/include/asm-generic/unistd.h
const AARCH64_ALLOWED_SYSCALLS: &[u32] = &[
    25,  // fcntl
    57,  // close
    62,  // lseek
    63,  // read
    64,  // write
    65,  // readv
    66,  // writev
    67,  // pread64
    73,  // ppoll
    79,  // newfstatat (__NR3264_fstatat)
    80,  // fstat
    93,  // exit
    94,  // exit_group
    98,  // futex
    99,  // set_robust_list
    100, // get_robust_list
    101, // nanosleep
    113, // clock_gettime
    115, // clock_nanosleep
    123, // sched_getaffinity
    124, // sched_yield
    128, // restart_syscall
    132, // sigaltstack
    134, // rt_sigaction
    135, // rt_sigprocmask
    139, // rt_sigreturn
    172, // getpid
    174, // getuid
    175, // geteuid
    176, // getgid
    177, // getegid
    178, // gettid
    214, // brk
    215, // munmap
    216, // mremap
    222, // mmap
    226, // mprotect
    233, // madvise
    278, // getrandom
    291, // statx
    293, // rseq
];

const AARCH64_DENIED_SYSCALLS: &[u32] = &[
    56,  // openat
    117, // ptrace
    129, // kill
    198, // socket
    199, // socketpair
    200, // bind
    201, // listen
    202, // accept
    203, // connect
    220, // clone
    221, // execve
    242, // accept4
    281, // execveat
    435, // clone3
    437, // openat2
];

/// Stack-allocated compiled `seccomp-bpf` filter program (avoids heap allocations after `fork`).
pub struct CompiledBpfFilter {
    insns: [SockFilter; MAX_BPF_INSNS],
    len: usize,
}

impl CompiledBpfFilter {
    pub fn as_slice(&self) -> &[SockFilter] {
        &self.insns[..self.len]
    }
}

/// Build the renderer `seccomp-bpf` program for the given architecture.
pub fn build_seccomp_filter(arch: BpfArch) -> Result<CompiledBpfFilter, SandboxError> {
    let denied = arch.denied_syscalls();
    let allowed = arch.allowed_syscalls();
    let total = 4 + denied.len() + usize::from(!denied.is_empty()) + allowed.len() + 2;
    if total > MAX_BPF_INSNS {
        return Err(SandboxError::new(format!(
            "seccomp filter exceeds static buffer ({total} > {MAX_BPF_INSNS})"
        )));
    }
    if denied.len() > u8::MAX as usize || allowed.len() > u8::MAX as usize {
        return Err(SandboxError::new(
            "seccomp jump offset exceeds 8-bit BPF limit",
        ));
    }

    let mut prog = CompiledBpfFilter {
        insns: [SockFilter::stmt(0, 0); MAX_BPF_INSNS],
        len: 0,
    };
    let mut push = |insn: SockFilter| {
        prog.insns[prog.len] = insn;
        prog.len += 1;
    };

    // 1. Validate seccomp_data.arch so 32-bit/compat syscall tables cannot bypass the filter.
    push(SockFilter::stmt(BPF_LD_W_ABS, SECCOMP_DATA_ARCH_OFFSET));
    push(SockFilter::jump(BPF_JMP_JEQ_K, arch.audit_arch(), 1, 0));
    push(SockFilter::stmt(BPF_RET_K, SECCOMP_RET_KILL_PROCESS));

    // 2. Load seccomp_data.nr.
    push(SockFilter::stmt(BPF_LD_W_ABS, SECCOMP_DATA_NR_OFFSET));

    // 3. Explicit deny table -> SECCOMP_RET_ERRNO | EPERM.
    if !denied.is_empty() {
        let d = denied.len();
        for (i, &nr) in denied.iter().enumerate() {
            if i + 1 < d {
                let jt = (d - 1 - i) as u8;
                push(SockFilter::jump(BPF_JMP_JEQ_K, nr, jt, 0));
            } else {
                push(SockFilter::jump(BPF_JMP_JEQ_K, nr, 0, 1));
            }
        }
        push(SockFilter::stmt(BPF_RET_K, SECCOMP_RET_ERRNO_EPERM));
    }

    // 4. Explicit allow table -> SECCOMP_RET_ALLOW, default -> SECCOMP_RET_ERRNO | EPERM.
    let a = allowed.len();
    for (i, &nr) in allowed.iter().enumerate() {
        let jt = (a - i) as u8;
        push(SockFilter::jump(BPF_JMP_JEQ_K, nr, jt, 0));
    }
    push(SockFilter::stmt(BPF_RET_K, SECCOMP_RET_ERRNO_EPERM));
    push(SockFilter::stmt(BPF_RET_K, SECCOMP_RET_ALLOW));

    Ok(prog)
}

fn clamp_rlimit_pair(desired: u64, current_max: u64) -> RLimit {
    let capped = if current_max == RLIM_INFINITY {
        desired
    } else {
        desired.min(current_max)
    };
    RLimit {
        rlim_cur: capped,
        rlim_max: capped,
    }
}

fn lower_rlimit(resource: c_int, desired: u64, name: &str) -> Result<(), SandboxError> {
    let mut current = RLimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe { getrlimit(resource, &mut current) } != 0 {
        return Err(SandboxError::new(format!(
            "getrlimit({name}) failed with errno {}",
            last_os_errno()
        )));
    }
    let next = clamp_rlimit_pair(desired, current.rlim_max);
    if unsafe { setrlimit(resource, &next) } != 0 {
        return Err(SandboxError::new(format!(
            "setrlimit({name}) failed with errno {}",
            last_os_errno()
        )));
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

fn write_proc_id_maps(euid: u32, egid: u32) -> Result<(), SandboxError> {
    if let Err(err) = fs::write("/proc/self/setgroups", b"deny\n") {
        // EACCES/ENOENT can occur on older kernels without setgroups; continue mapping if allowed.
        if err.raw_os_error() != Some(2) {
            return Err(SandboxError::new(format!(
                "writing /proc/self/setgroups failed: {err}"
            )));
        }
    }
    let uid_map = format!("{euid} {euid} 1\n");
    fs::write("/proc/self/uid_map", uid_map.as_bytes())
        .map_err(|e| SandboxError::new(format!("writing /proc/self/uid_map failed: {e}")))?;
    let gid_map = format!("{egid} {egid} 1\n");
    fs::write("/proc/self/gid_map", gid_map.as_bytes())
        .map_err(|e| SandboxError::new(format!("writing /proc/self/gid_map failed: {e}")))?;
    Ok(())
}

fn try_isolate_namespaces() -> (bool, bool, Option<String>) {
    let euid = unsafe { geteuid() };
    let egid = unsafe { getegid() };

    if unsafe { unshare(CLONE_NEWUSER | CLONE_NEWNET) } == 0 {
        let map_note = write_proc_id_maps(euid, egid).err().map(|e| e.message);
        return (true, true, map_note);
    }
    let both_errno = last_os_errno();

    if unsafe { unshare(CLONE_NEWUSER) } == 0 {
        let map_note = write_proc_id_maps(euid, egid).err().map(|e| e.message);
        return (
            true,
            false,
            Some(match map_note {
                Some(note) => format!("CLONE_NEWNET skipped (errno {both_errno}); {note}"),
                None => format!("CLONE_NEWNET unavailable (errno {both_errno}); user_ns active"),
            }),
        );
    }

    if unsafe { unshare(CLONE_NEWNET) } == 0 {
        return (
            false,
            true,
            Some(format!(
                "CLONE_NEWUSER unavailable (errno {both_errno}); net_ns active"
            )),
        );
    }

    (
        false,
        false,
        Some(format!(
            "unprivileged user/net namespaces restricted by host (errno {both_errno})"
        )),
    )
}

fn install_seccomp_filter(prog: &CompiledBpfFilter) -> Result<(), SandboxError> {
    let fprog = SockFprog {
        len: prog.len as u16,
        filter: prog.insns.as_ptr(),
    };
    if unsafe {
        prctl(
            PR_SET_SECCOMP,
            SECCOMP_MODE_FILTER,
            &fprog as *const SockFprog as c_ulong,
            0,
            0,
        )
    } != 0
    {
        return Err(SandboxError::new(format!(
            "prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER) failed with errno {}",
            last_os_errno()
        )));
    }
    Ok(())
}

/// Apply Linux renderer containment according to `profile`.
pub fn apply_renderer_sandbox(profile: SandboxProfile) -> Result<SandboxReport, SandboxError> {
    let mut report = SandboxReport::empty(profile);

    apply_process_rlimits(DEFAULT_RENDERER_MAX_AS_BYTES, DEFAULT_RENDERER_MAX_NOFILE)?;
    report.rlimits_applied = true;

    let (user_ns, net_ns, ns_note) = try_isolate_namespaces();
    report.user_ns_isolated = user_ns;
    report.net_ns_isolated = net_ns;

    if unsafe { prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(SandboxError::new(format!(
            "prctl(PR_SET_NO_NEW_PRIVS) failed with errno {}",
            last_os_errno()
        )));
    }

    let mut notes: Vec<String> = Vec::new();
    if let Some(note) = ns_note {
        notes.push(note);
    }

    match profile {
        SandboxProfile::StrictRenderer => {
            #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
            {
                let filter = build_seccomp_filter(BpfArch::host())?;
                install_seccomp_filter(&filter)?;
                report.seccomp_applied = true;
            }
            #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
            {
                notes.push("seccomp-bpf syscall table not defined for this CPU arch".to_string());
            }
        }
        SandboxProfile::PermissiveFallback => {
            notes.push("PermissiveFallback profile: seccomp-bpf skipped".to_string());
        }
    }

    report.notes = notes.join("; ");
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::os::raw::{c_char, c_void};

    const AF_INET: c_int = 2;
    const SOCK_STREAM: c_int = 1;
    const AT_FDCWD: c_int = -100;
    const O_RDONLY: c_int = 0;

    extern "C" {
        fn fork() -> c_int;
        fn _exit(status: c_int) -> !;
        fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int;
        fn pipe(fds: *mut [c_int; 2]) -> c_int;
        fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize;
        fn write(fd: c_int, buf: *const c_void, count: usize) -> isize;
        fn close(fd: c_int) -> c_int;
        fn socket(domain: c_int, ty: c_int, protocol: c_int) -> c_int;
        fn openat(dirfd: c_int, pathname: *const c_char, flags: c_int, mode: u32) -> c_int;
    }

    fn eval_bpf(prog: &[SockFilter], arch: u32, nr: u32) -> u32 {
        let mut a: u32 = 0;
        let mut pc: usize = 0;
        while pc < prog.len() {
            let insn = prog[pc];
            match insn.code {
                BPF_LD_W_ABS => {
                    a = match insn.k {
                        SECCOMP_DATA_NR_OFFSET => nr,
                        SECCOMP_DATA_ARCH_OFFSET => arch,
                        _ => panic!("unexpected BPF load offset {}", insn.k),
                    };
                    pc += 1;
                }
                BPF_JMP_JEQ_K => {
                    let jump = if a == insn.k { insn.jt } else { insn.jf };
                    pc += 1 + jump as usize;
                }
                BPF_RET_K => return insn.k,
                other => panic!("unexpected BPF opcode {other:#x}"),
            }
        }
        panic!("BPF program terminated without RET");
    }

    #[test]
    fn syscall_tables_are_disjoint_and_bpf_evaluates_correctly_for_x86_64_and_aarch64() {
        for arch in [BpfArch::X86_64, BpfArch::Aarch64] {
            let allowed: HashSet<u32> = arch.allowed_syscalls().iter().copied().collect();
            let denied: HashSet<u32> = arch.denied_syscalls().iter().copied().collect();
            assert!(
                allowed.is_disjoint(&denied),
                "allowed and denied tables overlap for {arch:?}"
            );

            let compiled = build_seccomp_filter(arch).expect("filter builds");
            let insns = compiled.as_slice();

            for &nr in arch.allowed_syscalls() {
                assert_eq!(
                    eval_bpf(insns, arch.audit_arch(), nr),
                    SECCOMP_RET_ALLOW,
                    "allowed syscall {nr} denied for {arch:?}"
                );
            }
            for &nr in arch.denied_syscalls() {
                assert_eq!(
                    eval_bpf(insns, arch.audit_arch(), nr),
                    SECCOMP_RET_ERRNO_EPERM,
                    "denied syscall {nr} not blocked for {arch:?}"
                );
            }
            // Unknown syscall falls back to EPERM
            assert_eq!(
                eval_bpf(insns, arch.audit_arch(), 9999),
                SECCOMP_RET_ERRNO_EPERM
            );
            // Wrong ABI arch kills process
            assert_eq!(eval_bpf(insns, 0xDEAD_BEEF, 1), SECCOMP_RET_KILL_PROCESS);
        }
    }

    #[test]
    fn clamp_rlimit_pair_respects_hard_limit_and_infinity() {
        assert_eq!(
            clamp_rlimit_pair(64, RLIM_INFINITY),
            RLimit {
                rlim_cur: 64,
                rlim_max: 64
            }
        );
        assert_eq!(
            clamp_rlimit_pair(1024, 512),
            RLimit {
                rlim_cur: 512,
                rlim_max: 512
            }
        );
        assert_eq!(
            clamp_rlimit_pair(0, 4096),
            RLimit {
                rlim_cur: 0,
                rlim_max: 0
            }
        );
    }

    #[test]
    fn forked_strict_renderer_enforces_seccomp_and_allows_computation_and_pipe_write() {
        let mut fds = [-1 as c_int; 2];
        assert_eq!(unsafe { pipe(&mut fds) }, 0, "pipe creation failed");
        let [read_fd, write_fd] = fds;

        let pid = unsafe { fork() };
        assert!(pid >= 0, "fork failed");

        if pid == 0 {
            unsafe {
                close(read_fd);
            }
            let report = match apply_renderer_sandbox(SandboxProfile::StrictRenderer) {
                Ok(report) => report,
                Err(_) => unsafe { _exit(2) },
            };

            let sock_fd = unsafe { socket(AF_INET, SOCK_STREAM, 0) };
            let sock_eperm = sock_fd == -1 && last_os_errno() == EPERM as i32;
            if sock_fd >= 0 {
                unsafe {
                    close(sock_fd);
                }
            }

            let passwd_path = c"/etc/passwd";
            let open_fd = unsafe { openat(AT_FDCWD, passwd_path.as_ptr(), O_RDONLY, 0) };
            let open_eperm = open_fd == -1 && last_os_errno() == EPERM as i32;
            if open_fd >= 0 {
                unsafe {
                    close(open_fd);
                }
            }

            let computed: u8 = 21u8.wrapping_add(21);
            let payload = [
                u8::from(report.seccomp_applied),
                u8::from(report.rlimits_applied),
                u8::from(sock_eperm),
                u8::from(open_eperm),
                computed,
            ];
            let written = unsafe {
                write(
                    write_fd,
                    payload.as_ptr() as *const c_void,
                    payload.len(),
                )
            };
            unsafe {
                close(write_fd);
                _exit(if written == payload.len() as isize { 0 } else { 3 });
            }
        }

        unsafe {
            close(write_fd);
        }
        let mut buf = [0u8; 5];
        let mut filled = 0usize;
        while filled < buf.len() {
            let n = unsafe {
                read(
                    read_fd,
                    buf[filled..].as_mut_ptr() as *mut c_void,
                    buf.len() - filled,
                )
            };
            if n <= 0 {
                break;
            }
            filled += n as usize;
        }
        unsafe {
            close(read_fd);
        }

        let mut status: c_int = 0;
        let waited = unsafe { waitpid(pid, &mut status, 0) };
        assert_eq!(waited, pid, "waitpid failed");
        let exited_ok = (status & 0x7f) == 0 && ((status >> 8) & 0xff) == 0;
        assert!(exited_ok, "sandboxed child exited abnormally: status={status}");
        assert_eq!(filled, 5, "child did not write full status payload");
        assert_eq!(buf[0], 1, "expected seccomp_applied == true");
        assert_eq!(buf[1], 1, "expected rlimits_applied == true");
        assert_eq!(buf[2], 1, "expected socket() to fail with EPERM");
        assert_eq!(buf[3], 1, "expected openat(/etc/passwd) to fail with EPERM");
        assert_eq!(buf[4], 42, "expected child computation over pipe to succeed");
    }

    #[test]
    fn forked_permissive_fallback_applies_rlimits_without_seccomp() {
        let mut fds = [-1 as c_int; 2];
        assert_eq!(unsafe { pipe(&mut fds) }, 0, "pipe creation failed");
        let [read_fd, write_fd] = fds;

        let pid = unsafe { fork() };
        assert!(pid >= 0, "fork failed");

        if pid == 0 {
            unsafe {
                close(read_fd);
            }
            let report = match apply_renderer_sandbox(SandboxProfile::PermissiveFallback) {
                Ok(report) => report,
                Err(_) => unsafe { _exit(2) },
            };
            let payload = [
                u8::from(report.rlimits_applied),
                u8::from(report.seccomp_applied),
            ];
            let written = unsafe {
                write(
                    write_fd,
                    payload.as_ptr() as *const c_void,
                    payload.len(),
                )
            };
            unsafe {
                close(write_fd);
                _exit(if written == payload.len() as isize { 0 } else { 3 });
            }
        }

        unsafe {
            close(write_fd);
        }
        let mut buf = [0u8; 2];
        let mut filled = 0usize;
        while filled < buf.len() {
            let n = unsafe {
                read(
                    read_fd,
                    buf[filled..].as_mut_ptr() as *mut c_void,
                    buf.len() - filled,
                )
            };
            if n <= 0 {
                break;
            }
            filled += n as usize;
        }
        unsafe {
            close(read_fd);
        }

        let mut status: c_int = 0;
        let waited = unsafe { waitpid(pid, &mut status, 0) };
        assert_eq!(waited, pid);
        let exited_ok = (status & 0x7f) == 0 && ((status >> 8) & 0xff) == 0;
        assert!(exited_ok, "permissive child exited abnormally: status={status}");
        assert_eq!(filled, 2);
        assert_eq!(buf[0], 1, "expected rlimits_applied == true");
        assert_eq!(buf[1], 0, "expected seccomp_applied == false");
    }
}
