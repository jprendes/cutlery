use std::io::{Read as _, Write as _, pipe, stdout};
use std::process::exit;
use std::thread::sleep;
use std::time::Duration;

use stdio_utils::StdioOverride;

use crate::{Fork, fork, fork_fn};

#[test]
fn test_fork_basic() {
    match fork().unwrap() {
        Fork::Parent(mut child) => {
            let exit_code = child.wait().unwrap();
            assert_eq!(exit_code, 42);
        }
        Fork::Child => {
            exit(42);
        }
    }
}

#[test]
fn test_pid() {
    let (mut r, mut w) = pipe().unwrap();
    let child = fork_fn(move || {
        w.write_all(&std::process::id().to_ne_bytes()).unwrap();
    })
    .unwrap();

    let mut buf = [0; core::mem::size_of::<u32>()];
    r.read_exact(&mut buf).unwrap();
    let pid = u32::from_ne_bytes(buf);

    assert_eq!(pid, child.id());
}

#[test]
fn test_fork_try_wait() {
    match fork().unwrap() {
        Fork::Parent(mut child) => {
            let status = child.try_wait().unwrap();
            assert_eq!(status, None);

            sleep(Duration::from_secs(2));

            let status = child.try_wait().unwrap();
            assert_eq!(status, Some(42));

            let status = child.wait().unwrap();
            assert_eq!(status, 42);
        }
        Fork::Child => {
            sleep(Duration::from_secs(1));
            exit(42);
        }
    }
}

#[test]
fn test_fork_pipe() {
    let (mut r, mut w) = pipe().unwrap();

    match fork().unwrap() {
        Fork::Child => {
            w.write_all(b"hello world").unwrap();
            exit(42);
        }
        Fork::Parent(mut child) => {
            let mut buf = [0; 11];
            r.read_exact(&mut buf).unwrap();

            assert_eq!(&buf, b"hello world");

            child.wait().unwrap();
        }
    }
}

#[test]
fn test_fork_fn() {
    let (mut r, mut w) = pipe().unwrap();

    let mut child = fork_fn(move || {
        w.write_all(b"hello world").unwrap();
    })
    .unwrap();

    let mut buf = [0u8; 11];
    r.read_exact(&mut buf).unwrap();

    assert_eq!(&buf, b"hello world");

    child.wait().unwrap();
}

#[test]
fn test_fork_stdout() {
    let (mut r, w) = pipe().unwrap();

    let _guard = w.override_stdout().unwrap();
    drop(w);

    let mut child = fork_fn(|| {
        // use stdout instead of println to avoid
        // output capturing by rust's test framework
        stdout().write_all(b"hello from child!").unwrap();
    })
    .unwrap();

    drop(_guard);

    let mut buf = vec![];
    r.read_to_end(&mut buf).unwrap();

    assert_eq!(&buf, b"hello from child!");

    child.wait().unwrap();
}
