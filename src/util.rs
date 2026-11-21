//! Small helpers: time, hashes, signals, child processes and locks.

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

/// Now as RFC 3339 in UTC, to the second.
pub fn now_stamp() -> String {
    let f = time::macros::format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z");
    time::OffsetDateTime::now_utc()
        .format(&f)
        .unwrap_or_default()
}

/// The first 12 hex digits of the SHA-256 of a text.
pub fn short_hash(s: &str) -> String {
    let d = Sha256::digest(s.as_bytes());
    d.iter().take(6).map(|b| format!("{b:02x}")).collect()
}

/// A flag set by SIGTERM, SIGINT or SIGHUP.
pub fn signal_flag() -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    for sig in [
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGHUP,
    ] {
        let _ = signal_hook::flag::register(sig, Arc::clone(&flag));
    }
    flag
}

/// Sleep, waking up early when `stop` is set. Returns false if stopped.
pub fn sleep(d: Duration, stop: &AtomicBool) -> bool {
    let end = Instant::now() + d;
    while Instant::now() < end {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        thread::sleep(Duration::from_millis(50).min(end - Instant::now()));
    }
    !stop.load(Ordering::Relaxed)
}

/// How a child process ended.
#[derive(Debug)]
pub enum Ran {
    /// Exit code (128 + signal if killed by one), standard output and
    /// standard error.
    Exited(i32, Vec<u8>, Vec<u8>),
    /// `stop` was set: the child got SIGTERM (then SIGKILL) and was reaped.
    Stopped,
}

/// Run a command with `input` on standard input, collecting its output.
/// When `stop` is set, the child is ended.
pub fn run(cmd: &mut Command, input: Option<&[u8]>, stop: &AtomicBool) -> std::io::Result<Ran> {
    cmd.stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let writer = child.stdin.take().map(|mut s| {
        let data = input.unwrap_or_default().to_vec();
        thread::spawn(move || {
            let _ = s.write_all(&data);
        })
    });
    let read = |r: Option<Box<dyn Read + Send>>| {
        thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut r) = r {
                let _ = r.read_to_end(&mut buf);
            }
            buf
        })
    };
    let out = read(
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    );
    let err = read(
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    );
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break Some(s);
        }
        if stop.load(Ordering::Relaxed) {
            // SAFETY: a plain signal to our own child.
            unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
            let grace = Instant::now() + Duration::from_secs(5);
            while Instant::now() < grace {
                if child.try_wait()?.is_some() {
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        thread::sleep(Duration::from_millis(50));
    };
    if let Some(w) = writer {
        let _ = w.join();
    }
    let out = out.join().unwrap_or_default();
    let err = err.join().unwrap_or_default();
    Ok(match status {
        None => Ran::Stopped,
        Some(s) => {
            use std::os::unix::process::ExitStatusExt;
            let code = s.code().unwrap_or_else(|| 128 + s.signal().unwrap_or(0));
            Ran::Exited(code, out, err)
        }
    })
}

/// Take an exclusive lock without waiting. Returns false if another
/// process holds it.
pub fn try_lock(file: &File) -> std::io::Result<bool> {
    // SAFETY: flock on a file descriptor we own.
    let r = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if r == 0 {
        return Ok(true);
    }
    let e = std::io::Error::last_os_error();
    if e.raw_os_error() == Some(libc::EWOULDBLOCK) {
        Ok(false)
    } else {
        Err(e)
    }
}

/// Whether an address follows lmm's rules: 1-64 of a-z 0-9 . _ -,
/// starting with a-z or 0-9, and not `lmm`.
pub fn valid_address(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'_' | b'-')
        })
        && s != "lmm"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert!(valid_address("todo-coder"));
        assert!(valid_address("a.b_c-1"));
        assert!(!valid_address("Bad"));
        assert!(!valid_address("-x"));
        assert!(!valid_address("lmm"));
        assert!(!valid_address(""));
    }

    #[test]
    fn hashes_and_stamps() {
        assert_eq!(short_hash("x").len(), 12);
        assert_ne!(short_hash("a"), short_hash("b"));
        assert!(now_stamp().ends_with('Z'));
    }
}
