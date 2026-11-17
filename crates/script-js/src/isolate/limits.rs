//! Per-platform process limits and exit-status decoding.
//!
//! | | memory | lifetime |
//! |---|---|---|
//! | Linux | `RLIMIT_DATA` set by the host on itself before the script runs, plus resident-size polling by the app | the host exits when its input ends |
//! | macOS | resident-size polling by the app (the kernel does not enforce `RLIMIT_DATA`) | the host exits when its input ends |
//! | Windows | resident-size polling by the app only (no job object memory limit, see below) | a job object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, and the host exits when its input ends |
//!
//! On Windows the job object only ends the host with the app; it sets no memory limit.
//! `win32job` has no safe setter for `JOB_OBJECT_LIMIT_PROCESS_MEMORY` and this workspace forbids
//! unsafe code, so memory there is resident-size polling alone, which does not count memory the
//! host committed but has not touched. Follow-up: a small isolated helper crate with the one
//! unsafe `SetInformationJobObject` call, so the system enforces the limit.
//!
//! Stopping a host also stops anything it started, which could otherwise hold its output pipe open
//! (the script host starts nothing; this is for a host that misbehaves): on Unix the host runs in
//! its own process group, which is killed while the host is still unreaped (its group id can't be
//! reused then); on Windows the job object is closed, which ends every process in it.
//!
//! On every platform the host ends when its input does: the app holds the only write end of that
//! pipe, so the system closes it when the app exits or dies, however that happens. A
//! parent-death signal is deliberately not used: on Linux it follows the thread that started the
//! host, not the app, so a run started from a short-lived thread would be killed with it.
//!
//! Everything here is safe code over existing wrappers (`rustix` on Linux, `win32job` on
//! Windows, `sysinfo` for the resident size).

use std::process::{Child, ExitStatus};

/// Host side: cap the memory this process may map (Linux; elsewhere the app polls).
///
/// # Errors
/// The system refused the limit.
pub fn limit_memory(bytes: u64) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        use rustix::process::{Resource, Rlimit, setrlimit};
        setrlimit(Resource::Data, Rlimit { current: Some(bytes), maximum: Some(bytes) }).map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = bytes;
        Ok(())
    }
}

/// App side: what keeps a host from outliving the app beyond its closed input.
pub struct Containment {
    #[cfg(windows)]
    job: std::sync::Mutex<Option<win32job::Job>>,
}

impl Containment {
    /// Reap the host if it has exited, first stopping everything it started. Use this instead of
    /// [`Child::try_wait`]: once the host is reaped its pid, which is its process group's id, can
    /// go to another process, so the group can no longer be killed safely. While the host is a
    /// zombie the id is still reserved.
    ///
    /// # Errors
    /// What `try_wait` returns.
    pub fn try_wait(&self, child: &mut Child) -> std::io::Result<Option<std::process::ExitStatus>> {
        #[cfg(all(unix, not(target_os = "openbsd")))]
        match host_state(child) {
            Host::Running => return Ok(None),
            Host::Exited => kill_group(child),
            Host::Reaped => {}
        }
        let status = child.try_wait()?;
        #[cfg(windows)]
        if status.is_some() {
            self.close_job();
        }
        Ok(status)
    }

    /// Stop the host and everything it started. `child` is the host, locked by the caller.
    pub fn end(&self, child: &mut Child) {
        #[cfg(all(unix, not(target_os = "openbsd")))]
        if host_state(child) != Host::Reaped {
            kill_group(child);
        }
        // Without `waitid`, only a host that is still running is known to be unreaped.
        #[cfg(target_os = "openbsd")]
        if matches!(child.try_wait(), Ok(None)) {
            kill_group(child);
        }
        #[cfg(windows)]
        self.close_job();
        let _ = child.kill();
    }

    #[cfg(windows)]
    fn close_job(&self) {
        // Closing the only handle of a kill-on-close job ends every process in it.
        drop(self.job.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take());
    }
}

#[cfg(all(unix, not(target_os = "openbsd")))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Host {
    Running,
    /// Exited and not reaped yet: a zombie that keeps its pid.
    Exited,
    /// Reaped (or not our child any more): its pid may already be someone else's.
    Reaped,
}

/// Whether the host is running, exited but unreaped, or reaped, without reaping it.
#[cfg(all(unix, not(target_os = "openbsd")))]
fn host_state(child: &Child) -> Host {
    use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};
    let options = WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT;
    match waitid(WaitId::Pid(Pid::from_child(child)), options) {
        Ok(None) => Host::Running,
        Ok(Some(_)) => Host::Exited,
        // ECHILD: `std` already reaped it and cached the status.
        Err(_) => Host::Reaped,
    }
}

/// Kill the host's process group (the host and what it started). Only while the host is
/// unreaped: see [`Containment::try_wait`].
#[cfg(unix)]
fn kill_group(child: &Child) {
    use rustix::process::{Pid, Signal, kill_process_group};
    let _ = kill_process_group(Pid::from_child(child), Signal::KILL);
}

/// App side: put a freshly started host under the platform's lifetime limit (on Windows a
/// kill-on-close job object, with no memory limit). Failing to is not fatal: the host still exits
/// when its input closes.
pub fn contain(child: &Child) -> Containment {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        let mut info = win32job::ExtendedLimitInfo::new();
        info.limit_kill_on_job_close();
        let job = win32job::Job::create_with_limit_info(&info).ok().filter(|job| job.assign_process(child.as_raw_handle() as isize).is_ok());
        Containment { job: std::sync::Mutex::new(job) }
    }
    #[cfg(not(windows))]
    {
        let _ = child;
        Containment {}
    }
}

/// Resident size of a process, polled by the app.
pub struct Meter {
    system: Option<sysinfo::System>,
    pid: sysinfo::Pid,
}

impl Meter {
    pub fn new(pid: u32) -> Self {
        Self { system: sysinfo::System::new().ok(), pid: sysinfo::Pid::from_u32(pid) }
    }

    /// Bytes resident now, or `None` when the platform can't tell.
    pub fn resident(&mut self) -> Option<u64> {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate};
        let s = self.system.as_mut()?;
        s.refresh_processes_specifics(ProcessesToUpdate::Some(&[self.pid]), true, ProcessRefreshKind::nothing().with_memory());
        s.process(self.pid).map(sysinfo::Process::memory)
    }
}

/// What an exit status says about how the host ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exit {
    /// The exit code (on Windows the full 32-bit status, e.g. `0xC00000FD`).
    pub code: Option<u32>,
    /// The signal that ended it (Unix).
    pub signal: Option<i32>,
}

impl Exit {
    pub fn of(status: ExitStatus) -> Self {
        #[cfg(unix)]
        let signal = std::os::unix::process::ExitStatusExt::signal(&status);
        #[cfg(not(unix))]
        let signal = None;
        Self { code: status.code().map(|c| c as u32), signal }
    }
}

/// Windows: the thread overran its stack.
pub const STATUS_STACK_OVERFLOW: u32 = 0xC000_00FD;
/// Windows: an access violation.
pub const STATUS_ACCESS_VIOLATION: u32 = 0xC000_0005;
/// Windows: a fail-fast exit (how Rust aborts there, e.g. after a failed allocation).
pub const STATUS_STACK_BUFFER_OVERRUN: u32 = 0xC000_0409;
