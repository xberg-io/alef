//! Windows job objects: the platform's stand-in for a killable process group.
//!
//! Unix addresses a subprocess tree by process group id and tears the whole thing down with one
//! `kill(-pgid)`. Windows has no equivalent signal, and `TerminateProcess` -- what `Child::kill`
//! calls -- ends exactly one process. A `cmd /C` wrapper therefore dies while everything it
//! started keeps running, keeps alef's stdout and stderr pipes open, and stalls the bounded drain
//! in [`super::capture`] for its full grace period rather than ending at the deadline. A job
//! object is the closest analogue Windows offers: every process created by a job member joins the
//! job, so `TerminateJobObject` reaches the whole tree at once. ~keep

use std::os::windows::io::AsRawHandle as _;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW, TerminateJobObject};

/// An anonymous job object holding one child and everything that child goes on to create.
///
/// The handle is held for as long as the tree may still need killing; nothing else keeps the job
/// alive, and a closed handle on a job with no limits set simply detaches, leaving its members
/// running. That is deliberate: the tree is torn down when a deadline expires, never merely
/// because alef stopped watching it -- matching the Unix side, which also only signals the group
/// on expiry. ~keep
pub(crate) struct JobObject(HANDLE);

// SAFETY: a job object handle is a kernel handle with no thread affinity; every call below is
// safe to make from any thread, and `JobObject` owns the handle exclusively.
unsafe impl Send for JobObject {}
// SAFETY: as above -- the handle is only read, never mutated, through `&self`.
unsafe impl Sync for JobObject {}

impl JobObject {
    /// Creates a job object and puts `child` in it.
    ///
    /// `None` when the job cannot be created or the child cannot be assigned -- an already-exited
    /// child is the ordinary reason -- and the caller then falls back to killing the direct child
    /// alone.
    ///
    /// Assignment happens after `CreateProcess` has already returned, so a child that manages to
    /// spawn a descendant before this call lands leaves that one descendant outside the job. The
    /// window is one syscall wide against a child that has not yet been scheduled, and closing it
    /// would mean `CREATE_SUSPENDED` plus a thread handle `std::process` does not expose. ~keep
    pub(crate) fn holding(child: &std::process::Child) -> Option<Self> {
        // SAFETY: a null security descriptor and a null name request the default anonymous job.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return None;
        }
        let job = Self(handle);
        // SAFETY: both handles are open across the call -- `job` owns the first, `child` the
        // second -- and neither is used after this value is dropped.
        let assigned = unsafe { AssignProcessToJobObject(job.0, child.as_raw_handle()) };
        (assigned != 0).then_some(job)
    }

    /// Terminates every process in the job, returning whether the kernel accepted the request.
    pub(crate) fn terminate(&self) -> bool {
        // SAFETY: `self.0` is open until `Drop` runs.
        unsafe { TerminateJobObject(self.0, 1) != 0 }
    }
}

impl Drop for JobObject {
    fn drop(&mut self) {
        // SAFETY: the handle is owned by this value and closed exactly once.
        unsafe {
            CloseHandle(self.0);
        }
    }
}
