# Cutlery

A cross-platform (Unix and Windows) Rust library for process forking.

## Getting started

```rust
use std::io::{pipe, Read as _, Write as _};
use cutlery::fork_fn;

// create a pipe to communicate with the
// child process
let (mut r, mut w) = pipe()?;

let child = fork_fn(move || {
    // this closure is running inside of the
    // forked process

    // send a message from the child process
    w.write_all(b"hello world").unwrap();
    std::process::exit(42);
})?;

// execution continues in the parent process

// read the message in the parent process
let mut buf = [0u8; 11];
r.read_exact(&mut buf)?;
assert_eq!(&buf, b"hello world");

// retrieve the exit status of the child process
let status = child.wait()?;
assert_eq!(status, 42);
```