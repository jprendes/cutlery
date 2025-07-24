#[cfg(all(target_os = "linux", feature = "pidfd"))]
use core::mem::zeroed;
use std::io::{Error, ErrorKind, Result};
use std::os::fd::OwnedFd;
#[cfg(all(target_os = "linux", feature = "pidfd"))]
use std::os::fd::{AsRawFd, FromRawFd as _};
#[cfg(all(target_os = "linux", feature = "pidfd"))]
use std::ptr::null;

use super::{Child, Fork};

pub(super) type OwnedFileDescriptor = Option<OwnedFd>;

#[cfg(all(target_os = "linux", feature = "pidfd"))]
fn open_pidfd(pid: u32) -> Result<OwnedFd> {
    match unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) } {
        ..0 => Err(Error::last_os_error()),
        pidfd => Ok(unsafe { OwnedFd::from_raw_fd(pidfd as i32) }),
    }
}

#[cfg(not(all(target_os = "linux", feature = "pidfd")))]
fn open_pidfd(_pid: u32) -> Result<OwnedFd> {
    Err(Error::other("Unsupported"))
}

pub(super) fn fork() -> Result<Fork> {
    match cvt(unsafe { libc::fork() })? {
        0 => Ok(Fork::Child),
        pid => {
            let pid = pid as u32;
            let descriptor = open_pidfd(pid).ok();
            let status = None;
            Ok(Fork::Parent(Child {
                pid,
                descriptor,
                status,
            }))
        }
    }
}

fn wait_impl<const FLAGS: libc::c_int>(child: &Child) -> Result<Option<i32>> {
    #[cfg(all(target_os = "linux", feature = "pidfd"))]
    if let Some(pidfd) = &child.descriptor {
        let mut info = unsafe { zeroed::<libc::siginfo_t>() };
        cvt_r(|| unsafe {
            libc::waitid(
                libc::P_PIDFD,
                pidfd.as_raw_fd() as _,
                &mut info,
                libc::WEXITED | FLAGS,
            )
        })?;
        if info.si_code == 0 {
            // would block, so return None
            return Ok(None);
        }
        let status = unsafe { info.si_status() };
        return Ok(Some(status));
    }
    let mut status = 0;
    let pid = cvt_r(|| unsafe { libc::waitpid(child.pid as _, &mut status as *mut _, FLAGS) })?;
    if pid == 0 {
        return Ok(None);
    }
    if libc::WIFEXITED(status) {
        Ok(Some(libc::WEXITSTATUS(status)))
    } else if libc::WIFSIGNALED(status) {
        Ok(Some(libc::WTERMSIG(status)))
    } else {
        Ok(Some(-1))
    }
}

pub(super) fn wait(child: &Child) -> Result<i32> {
    wait_impl::<0>(child).map(|status| status.unwrap_or(-1))
}

pub(super) fn try_wait(child: &Child) -> Result<Option<i32>> {
    wait_impl::<{ libc::WNOHANG }>(child)
}

fn kill_impl(child: &Child) -> Result<()> {
    #[cfg(all(target_os = "linux", feature = "pidfd"))]
    if let Some(pidfd) = &child.descriptor {
        cvt(unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                pidfd.as_raw_fd(),
                libc::SIGKILL,
                null::<usize>(),
                0,
            )
        })?;
        return Ok(());
    }
    cvt(unsafe { libc::kill(child.pid as _, libc::SIGKILL) })?;
    Ok(())
}

pub(super) fn kill(child: &Child) -> Result<()> {
    match kill_impl(child) {
        Ok(()) => Ok(()),
        Err(err) if err.raw_os_error() == Some(libc::ESRCH) => Ok(()), // Process already exited
        Err(err) => Err(err),
    }
}

trait IsMinusOne {
    fn is_minus_one(&self) -> bool;
}

macro_rules! impl_is_minus_one {
    ($($t:ident)*) => ($(impl IsMinusOne for $t {
        fn is_minus_one(&self) -> bool {
            *self == -1
        }
    })*)
}

impl_is_minus_one! { i8 i16 i32 i64 isize }

/// Converts native return values to Result using the *-1 means error is in `errno`*  convention.
/// Non-error values are `Ok`-wrapped.
fn cvt<T: IsMinusOne>(t: T) -> Result<T> {
    if t.is_minus_one() {
        Err(Error::last_os_error())
    } else {
        Ok(t)
    }
}

/// `-1` → look at `errno` → retry on `EINTR`. Otherwise `Ok()`-wrap the closure return value.
fn cvt_r<T: IsMinusOne>(mut f: impl FnMut() -> T) -> Result<T> {
    loop {
        match cvt(f()) {
            Err(ref e) if is_interrupted(e) => {}
            other => return other,
        }
    }
}

fn is_interrupted(err: &Error) -> bool {
    err.kind() == ErrorKind::Interrupted || err.raw_os_error() == Some(libc::EINTR)
}
