//! Per-platform process limits and exit-status decoding.
//!
//! | | memory | lifetime |
//! |---|---|---|
//! | every platform | the host's heap cap ([`HostAllocator`]): an allocation that would take the heap past the limit fails. Where the engine can handle the failure the script gets an error; where it can't, the host ends at once with a failed allocation | |
//! | Linux | also `RLIMIT_DATA`, set by the host on itself before the script runs | the host exits when its input ends |
//! | macOS | the heap cap (the kernel enforces neither `RLIMIT_DATA` nor `RLIMIT_AS`) | the host exits when its input ends |
//! | Windows | the heap cap | a job object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, and the host exits when its input ends |
//!
//! The heap cap counts every allocation the host makes through Rust's allocator, which is where
//! the engine keeps every script value, at the moment it is made: there is no window in which a
//! fast allocation gets past it, and memory committed but not yet touched counts too. The script
//! thread's stack ([`super::STACK`]) is outside the heap and is subtracted from the limit. The
//! app also polls the host's resident size, as a second line for memory the allocator does not
//! see. On Windows a job object memory limit would need unsafe code (`win32job` has no setter for
//! `JOB_OBJECT_LIMIT_PROCESS_MEMORY`), which this workspace forbids; the heap cap does not.
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
//! Everything here is safe code over existing wrappers (`cap` for the heap, `rustix` on Linux,
//! `win32job` on Windows, `sysinfo` for the resident size).

use std::process::{Child, ExitStatus};

use super::heap::HostAllocator;

/// Host side: cap the memory this process may use at `bytes`: its heap at `bytes` less the
/// script thread's stack, and on Linux also the memory it may map.
///
/// # Errors
/// The limit is below what the host already uses, or the system refused it.
pub fn limit_memory(bytes: u64, heap: &HostAllocator) -> Result<(), String> {
    super::heap::cap_heap(heap, bytes.saturating_sub(super::STACK as u64))?;
    // A debug build can leave the system limit off, so a test sees the heap cap on its own.
    if cfg!(debug_assertions) && std::env::var_os("PHOTOCRAFT_TEST_NO_RLIMIT").is_some() {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        use rustix::process::{Resource, Rlimit, setrlimit};
        setrlimit(Resource::Data, Rlimit { current: Some(bytes), maximum: Some(bytes) }).map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "linux"))]
    Ok(())
}

/// App side: what keeps a host from outliving the app beyond its closed input.
pub struct Containment {
    #[cfg(windows)]
    job: std::sync::Mutex<Option<win32job::Job>>,
    /// Whether [`Containment::try_wait`] has reaped the host: until then its
    /// pid, which is its group's id, is still reserved.
    #[cfg(unix)]
    reaped: std::sync::atomic::AtomicBool,
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
        // Once reaped, the host's pid may be another process's, even another child of this app:
        // it is not probed or signalled again, and `std` answers from the status it kept.
        #[cfg(unix)]
        if self.is_reaped() {
            return child.try_wait();
        }
        #[cfg(all(unix, not(target_os = "openbsd")))]
        match host_state(child) {
            Host::Running => return Ok(None),
            Host::Exited => kill_group(child),
            Host::Reaped => {}
        }
        let status = child.try_wait()?;
        #[cfg(all(unix, not(target_os = "openbsd")))]
        if status.is_some() {
            self.reaped.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        // Without `waitid` the host is only known to have exited once it is reaped. What it
        // started is still in its group then, and a group id is not given to a new process while
        // the group has members, so the group is stopped right after. A group left empty is
        // signalled once by an id that is free again: OpenBSD does not hand a freed pid out at
        // once, but that is no guarantee. Watching the exit without reaping (a kqueue
        // `EVFILT_PROC`) would close that; it is not done here, where it can't be built or tested.
        #[cfg(target_os = "openbsd")]
        if status.is_some() && !self.reaped.swap(true, std::sync::atomic::Ordering::SeqCst) {
            kill_group(child);
        }
        #[cfg(windows)]
        if status.is_some() {
            self.close_job();
        }
        Ok(status)
    }

    /// Wait for the host to exit and reap it, as after [`Self::end`]. It is then known as reaped:
    /// its pid, its group's id, may go to another process, and its group is not signalled again.
    ///
    /// # Errors
    /// What `wait` returns.
    pub fn wait(&self, child: &mut Child) -> std::io::Result<std::process::ExitStatus> {
        let status = child.wait()?;
        #[cfg(unix)]
        self.reaped.store(true, std::sync::atomic::Ordering::SeqCst);
        #[cfg(windows)]
        self.close_job();
        Ok(status)
    }

    /// Stop the host and everything it started. `child` is the host, locked by the caller.
    pub fn end(&self, child: &mut Child) {
        #[cfg(all(unix, not(target_os = "openbsd")))]
        if !self.is_reaped() && host_state(child) != Host::Reaped {
            kill_group(child);
        }
        // Without `waitid`: a host this has not reaped keeps its pid (as a zombie if it exited),
        // so its group can still be killed safely; then it is reaped.
        #[cfg(target_os = "openbsd")]
        if !self.reaped.load(std::sync::atomic::Ordering::SeqCst) {
            kill_group(child);
        }
        #[cfg(windows)]
        self.close_job();
        let _ = child.kill();
    }

    #[cfg(unix)]
    fn is_reaped(&self) -> bool {
        self.reaped.load(std::sync::atomic::Ordering::SeqCst)
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
    loop {
        return match waitid(WaitId::Pid(Pid::from_child(child)), options) {
            Ok(None) => Host::Running,
            Ok(Some(_)) => Host::Exited,
            Err(rustix::io::Errno::INTR) => continue,
            // `std` already reaped it and cached the status.
            Err(rustix::io::Errno::CHILD) => Host::Reaped,
            // Any other failure says nothing about the host; only ECHILD means its pid is free.
            // Taken as running: nothing is reaped or killed on its account.
            Err(_) => Host::Running,
        };
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
    #[cfg(unix)]
    {
        let _ = child;
        Containment { reaped: std::sync::atomic::AtomicBool::new(false) }
    }
    #[cfg(not(any(windows, unix)))]
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

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::io::BufRead;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    /// Wait until `done` holds, polling, for at most 10 s: `false` if it never did.
    fn within(mut done: impl FnMut() -> bool) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done() {
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        true
    }

    /// Running, not a zombie waiting for whoever adopted it.
    fn alive(pid: i32) -> bool {
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else { return false };
        stat.rsplit_once(')').is_some_and(|(_, rest)| !rest.trim_start().starts_with('Z'))
    }

    /// A host reaped by a plain wait (a run that failed to start) counts as reaped too, so its
    /// group, whose id is free again, is not signalled afterwards.
    #[test]
    fn a_host_reaped_by_waiting_is_known_as_reaped() {
        let mut host = Command::new("true").process_group(0).spawn().expect("spawn");
        let containment = contain(&host);
        containment.end(&mut host);
        assert!(containment.wait(&mut host).is_ok());
        assert!(containment.is_reaped());
    }

    #[test]
    fn a_reaped_host_is_never_probed_or_its_group_signalled_again() {
        // The host: exits at once and is reaped.
        let mut host = Command::new("true").process_group(0).spawn().expect("spawn");
        let containment = contain(&host);
        let reaped = within(|| containment.try_wait(&mut host).expect("wait").is_some());
        if !reaped {
            let _ = host.kill();
            let _ = host.wait();
        }
        assert!(reaped, "the host was never seen to exit");
        // Its pid may now be another child's: one that exited, leaving its own group behind.
        let mut other = Command::new("sh").args(["-c", "sleep 30 & echo $!"]).process_group(0).stdout(Stdio::piped()).spawn().expect("spawn");
        let mut line = String::new();
        std::io::BufReader::new(other.stdout.take().expect("stdout")).read_line(&mut line).expect("read");
        let left: i32 = line.trim().parse().expect("pid");
        let kill_left = || {
            let _ = rustix::process::kill_process(rustix::process::Pid::from_raw(left).expect("pid"), rustix::process::Signal::KILL);
        };
        if !within(|| host_state(&other) == Host::Exited) {
            let _ = other.kill();
            let _ = other.wait();
            kill_left();
            panic!("the second host was never seen to exit");
        }
        let _ = containment.try_wait(&mut other);
        containment.end(&mut other);
        // A kill takes effect a moment after it is sent.
        std::thread::sleep(std::time::Duration::from_millis(300));
        let survived = alive(left);
        kill_left();
        assert!(survived, "the group of a process that only shares the reaped host's pid was killed");
    }
}
