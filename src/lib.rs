#[cfg_attr(unix, path = "impl/unix.rs")]
#[cfg_attr(windows, path = "impl/win.rs")]
mod r#impl;

use std::io::Result;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::process::{ExitCode, Termination, exit};

/// Representation of a running or exited child process,
/// similar to `std::process::Child`.
///
/// This structure is used to represent and manage child processes.
/// A child process is created via the `fork` or `fork_fn` functions.
///
/// There is no implementation of [`Drop`](std::ops::Drop) for child
/// processes, so if you do not ensure the Child has exited then it
/// will continue to run, even after the Child handle to the child
/// process has gone out of scope.
///
/// Calling [`wait`](Child::wait) will make the parent process wait
/// until the child has actually exited before continuing.
#[derive(Debug)]
pub struct Child {
    pid: u32,
    #[allow(dead_code)]
    descriptor: r#impl::OwnedFileDescriptor,
    status: Option<i32>,
}

/// Result of a [`fork`] operation.
/// In the parent process [`fork`] will return [`Fork::Parent`],
/// while in the child it will return [`Fork::Child`].
///
/// The `Parent` variant holds a struct that can be used to
/// wait for the child process to exit.
///
#[derive(Debug)]
pub enum Fork {
    Parent(Child),
    Child,
}

/// Forks the current process, creating a new child process
/// that inherits the state of the parent process.
///
/// # Notes
/// Executing code after creating a fork needs to be done
/// carefully. It is recommended that the process is single
/// threaded at the time of forking, or that extra care has
/// been taken about the other threads in the process before
/// forking.
///
/// The new process will contain a copy of the process state.
/// This allows the child process to continue directly where
/// the parent left off.
/// But the child process will only contain a copy of the
/// calling thread. All other threads in the process will not
/// be reflected in the child process. This is particularly
/// relevant when one of the (non-calling) threads is holding
/// a mutex. That mutex will never be released in the child
/// process. This is even true is don't use mutexes yourself,
/// as some of these mutexes might be part of the system
/// libraries (even by the libraries that `cutlery` uses
/// internally), e.g., the malloc implementation could use a
/// mutex.
///
/// File descriptors will be shared form the parent to the
/// child. This allows you to share pipes between the two
/// processes for communication. But care needs to be taken
/// as operations on those file descriptors can trigger
/// deadlocks or race conditions between the parent and the
/// child.
///
/// ## Example
/// ```rust
/// # use cutlery::*;
/// match fork()? {
///     Fork::Parent(mut child) => {
///         let status = child.wait()?;
///         assert_eq!(status, 42);
///     }
///     Fork::Child => {
///         std::process::exit(42);
///     }
/// }
/// # std::io::Result::Ok(())
/// ```
pub fn fork() -> Result<Fork> {
    r#impl::fork()
}

/// Run a function inside of a fork of the process.
/// The child process will exit after executing the function.
///
/// All the same considerations from [`fork`] also apply to
/// [`fork_fn`].
///
/// ## Example
/// ```rust
/// # use cutlery::*;
/// let mut child = fork_fn(move || {
///     println!("hello from child!");
/// })?;
/// let status = child.wait()?;
/// assert_eq!(status, 0);
/// # std::io::Result::Ok(())
/// ```
pub fn fork_fn<T: Termination>(f: impl FnOnce() -> T) -> Result<Child> {
    let f = move || catch_unwind(AssertUnwindSafe(f));
    match fork()? {
        Fork::Parent(child) => Ok(child),
        Fork::Child => match f().report() {
            ExitCode::SUCCESS => exit(0),
            _ => exit(1),
        },
    }
}

impl Child {
    /// Returns the OS-assigned process identifier associated with this child.
    pub fn id(&self) -> u32 {
        self.pid
    }

    /// Waits for the child to exit completely,
    /// returning the status that it exited with.
    /// This function will continue to have the
    /// same return value after it has been called
    /// at least once.
    pub fn wait(&mut self) -> Result<i32> {
        match self.status {
            Some(status) => Ok(status),
            None => {
                let status = r#impl::wait(self)?;
                self.status = Some(status);
                Ok(status)
            }
        }
    }

    /// Attempts to collect the exit status of the
    /// child if it has already exited.
    ///
    /// This function will not block the calling thread
    /// and will only check to see if the child process
    /// has exited or not. If the child has exited then
    /// on Unix the process ID is reaped. This function
    /// is guaranteed to repeatedly return a successful
    /// exit status so long as the child has already
    /// exited.
    ///
    /// If the child has exited, then Ok(Some(status))
    /// is returned. If the exit status is not available
    /// at this time then Ok(None) is returned. If an
    /// error occurs, then that error is returned.
    pub fn try_wait(&mut self) -> Result<Option<i32>> {
        match self.status {
            Some(status) => Ok(Some(status)),
            None => {
                self.status = r#impl::try_wait(self)?;
                Ok(self.status)
            }
        }
    }

    /// Forces the child process to exit. If the child has already exited, Ok(()) is returned.
    ///
    /// This is equivalent to sending a SIGKILL on Unix platforms.
    pub fn kill(&mut self) -> Result<()> {
        // If we've already waited on this process then
        // the pid can be recycled and used for another
        // process, and we probably shouldn't be killing
        // random processes, so return Ok because the
        // process has exited already.
        match self.status {
            Some(_) => Ok(()),
            None => r#impl::kill(self),
        }
    }
}

#[cfg(test)]
mod tests;
