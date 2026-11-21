//! Links (§5, §7.1 of the design): one recipient (an lmm address) and one
//! destination. A link waits for new messages with `lmm wait`, and tells
//! the destination about them until the destination is gone.
//!
//! State: `$NGATE_HOME/links/<ADDRESS>-<HASH>/` per lmm data directory and
//! address (HASH: of the data directory), with `link.yml`, `lock` (held by
//! the running link) and `log`.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapter::{Destination, Outcome};
use crate::config;
use crate::error::{Error, Result};
use crate::home::Home;
use crate::lmm::{self, Waited};
use crate::template::Which;
use crate::util;

/// `link.yml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkFile {
    pub address: String,
    pub destination: String,
    pub data_dir: String,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
}

/// The state directory of the link of `address` in `data_dir`.
pub fn link_dir(home: &Home, address: &str, data_dir: &Path) -> PathBuf {
    home.links().join(format!(
        "{address}-{}",
        util::short_hash(&data_dir.to_string_lossy())
    ))
}

fn read_link(dir: &Path) -> Option<LinkFile> {
    serde_yaml_ng::from_str(&fs::read_to_string(dir.join("link.yml")).ok()?).ok()
}

fn write_link(dir: &Path, f: &LinkFile) -> Result<()> {
    let text = serde_yaml_ng::to_string(f).map_err(|e| Error::runtime(e.to_string()))?;
    let tmp = dir.join(".link.yml.tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, dir.join("link.yml"))?;
    Ok(())
}

fn open_lock(dir: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("lock"))?)
}

/// Whether a link runs (holds its lock).
fn running(dir: &Path) -> bool {
    match File::open(dir.join("lock")) {
        Ok(f) => !util::try_lock(&f).unwrap_or(true),
        Err(_) => false,
    }
}

struct Log(PathBuf);

impl Log {
    fn line(&self, msg: impl std::fmt::Display) {
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&self.0) {
            let _ = writeln!(f, "{} {msg}", util::now_stamp());
        }
    }
}

pub struct LinkArgs {
    pub address: String,
    pub destination: String,
    pub detach: bool,
    pub template: Option<PathBuf>,
}

/// `ngate link`.
pub fn run(a: LinkArgs) -> Result<()> {
    if !util::valid_address(&a.address) {
        return Err(Error::usage(format!("invalid address {:?}", a.address)));
    }
    let cwd = std::env::current_dir()?;
    let home = Home::resolve()?;
    let cfg = config::load(&home)?;
    // Check everything before detaching, so that the caller sees mistakes.
    let mut dest = Destination::resolve(&a.destination, &cfg, a.template.as_deref())?;
    let paths = lmm::paths(&cwd, Some(&a.address))?;
    if a.detach {
        let mut c = Command::new(std::env::current_exe()?);
        c.arg("link");
        if let Some(t) = &a.template {
            c.arg("--template").arg(std::path::absolute(t)?);
        }
        c.arg(&a.address)
            .arg(&a.destination)
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // SAFETY: setsid is async-signal-safe; the new session makes the link
        // the leader of its process group, which `ngate stop` signals.
        unsafe {
            c.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        c.spawn()
            .map_err(|e| Error::runtime(format!("cannot start the link: {e}")))?;
        return Ok(());
    }
    let dir = link_dir(&home, &a.address, &paths.data_dir);
    fs::create_dir_all(&dir)?;
    let lock = open_lock(&dir)?;
    if !util::try_lock(&lock)? {
        // One link per data directory and address: this one already runs.
        return Ok(());
    }
    let mut info = LinkFile {
        address: a.address.clone(),
        destination: a.destination.clone(),
        data_dir: paths.data_dir.to_string_lossy().into_owned(),
        cwd: cwd.to_string_lossy().into_owned(),
        pid: Some(std::process::id()),
        started_at: Some(util::now_stamp()),
        ended_at: None,
        end: None,
    };
    write_link(&dir, &info)?;
    let log = Log(dir.join("log"));
    log.line(format!("start {} -> {}", a.address, a.destination));
    let stop = util::signal_flag();
    let end = serve(&cwd, &a.address, &mut dest, cfg.interval, &log, &stop);
    log.line(format!("end: {end}"));
    info.pid = None;
    info.ended_at = Some(util::now_stamp());
    info.end = Some(end);
    write_link(&dir, &info)?;
    drop(lock);
    Ok(())
}

/// The link's loop. Returns why it ended.
fn serve(
    cwd: &Path,
    address: &str,
    dest: &mut Destination,
    interval: f64,
    log: &Log,
    stop: &AtomicBool,
) -> String {
    let pause = Duration::from_secs_f64(interval);
    match dest.start(stop) {
        Outcome::Gone => return "the destination is gone".into(),
        Outcome::Stopped => return "stopped".into(),
        _ => {}
    }
    // Extracted messages already told about: false once, true after the
    // reminder (one reminder per message at most).
    let mut told: BTreeMap<PathBuf, bool> = BTreeMap::new();
    loop {
        match dest.alive(address, stop) {
            Outcome::Gone => return "the destination is gone".into(),
            Outcome::Stopped => return "stopped".into(),
            Outcome::Later(r) => log.line(format!("alive: {r}")),
            Outcome::Delivered => {}
        }
        let paths = match lmm::wait(cwd, address, interval, stop) {
            Err(e) => {
                log.line(e);
                if !util::sleep(pause, stop) {
                    return "stopped".into();
                }
                continue;
            }
            Ok(Waited::Stopped) => return "stopped".into(),
            Ok(Waited::Timeout) => continue,
            Ok(Waited::Failed(code, err)) => {
                log.line(format!("lmm wait exit {code}: {err}"));
                if matches!(lmm::box_exists(cwd, address), Ok(false)) {
                    return format!("the mailbox of {address} is gone");
                }
                if !util::sleep(pause, stop) {
                    return "stopped".into();
                }
                continue;
            }
            Ok(Waited::Got(paths, note)) => {
                if let Some(n) = note {
                    log.line(n);
                }
                paths
            }
        };
        if paths.is_empty() {
            continue;
        }
        log.line(format!("received {}", paths.len()));
        match notify(cwd, address, dest, "new", &paths, interval, log, stop) {
            Outcome::Delivered => {}
            Outcome::Gone => return "the destination is gone".into(),
            _ => return "stopped".into(),
        }
        if dest.delete_after {
            for p in &paths {
                if let Err(e) = fs::remove_dir_all(p) {
                    log.line(format!("cannot delete {}: {e}", p.display()));
                }
            }
        }
        if !dest.remind {
            continue;
        }
        for p in paths {
            told.insert(p, false);
        }
        // Tell once more about messages still left (the agent may have
        // stopped without dealing with them).
        told.retain(|p, _| p.is_dir());
        let left: Vec<PathBuf> = told
            .iter_mut()
            .filter(|(_, reminded)| !**reminded)
            .map(|(p, reminded)| {
                *reminded = true;
                p.clone()
            })
            .collect();
        if !left.is_empty() {
            log.line(format!("still left {}", left.len()));
            match notify(cwd, address, dest, "reminder", &left, interval, log, stop) {
                Outcome::Delivered => {}
                Outcome::Gone => return "the destination is gone".into(),
                _ => return "stopped".into(),
            }
        }
    }
}

/// Deliver until delivered, gone or stopped; retry after `interval`.
#[allow(clippy::too_many_arguments)]
fn notify(
    cwd: &Path,
    address: &str,
    dest: &Destination,
    kind: &str,
    paths: &[PathBuf],
    interval: f64,
    log: &Log,
    stop: &AtomicBool,
) -> Outcome {
    let messages: Vec<Value> = paths
        .iter()
        .map(|p| lmm::message(cwd, address, p))
        .collect();
    let which = if kind == "new" {
        Which::New
    } else {
        Which::Reminder
    };
    let text = dest.templates.render(which, paths, address);
    loop {
        match dest.deliver(kind, address, &messages, &text, stop) {
            Outcome::Later(r) => {
                log.line(format!("{kind}: {r}; trying again"));
                if !util::sleep(Duration::from_secs_f64(interval), stop) {
                    return Outcome::Stopped;
                }
            }
            o => {
                log.line(format!("{kind}: {o:?}"));
                return o;
            }
        }
    }
}

/// `ngate stop ADDRESS`: stop the link of the address in this directory's
/// lmm data directory.
pub fn stop(address: &str) -> Result<()> {
    if !util::valid_address(address) {
        return Err(Error::usage(format!("invalid address {address:?}")));
    }
    let cwd = std::env::current_dir()?;
    let home = Home::resolve()?;
    let paths = lmm::paths(&cwd, Some(address))?;
    let dir = link_dir(&home, address, &paths.data_dir);
    if !running(&dir) {
        return Ok(());
    }
    let Some(pid) = read_link(&dir).and_then(|l| l.pid) else {
        return Err(Error::runtime(format!(
            "the link of {address} runs but its pid is unknown ({})",
            dir.display()
        )));
    };
    let signal = |sig: i32| {
        // The link and its children (lmm wait, the adapter) form a process
        // group when detached; otherwise signal the link alone.
        // SAFETY: plain signals.
        unsafe {
            if libc::kill(-(pid as i32), sig) != 0 {
                libc::kill(pid as i32, sig);
            }
        }
    };
    signal(libc::SIGTERM);
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        if !running(&dir) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    signal(libc::SIGKILL);
    Ok(())
}

/// `ngate list`.
pub fn list() -> Result<()> {
    let home = Home::resolve()?;
    let mut rows = Vec::new();
    let entries = match fs::read_dir(home.links()) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    for e in entries {
        let dir = e?.path();
        let Some(l) = read_link(&dir) else { continue };
        let on = running(&dir);
        rows.push([
            l.address,
            l.destination,
            if on { "running" } else { "stopped" }.to_string(),
            l.pid
                .filter(|_| on)
                .map(|p| p.to_string())
                .unwrap_or_else(|| "-".into()),
            l.cwd,
        ]);
    }
    rows.sort();
    let header = ["ADDRESS", "DESTINATION", "STATE", "PID", "CWD"].map(String::from);
    let mut widths = header.clone().map(|h| h.chars().count());
    for r in &rows {
        for (i, c) in r.iter().enumerate() {
            widths[i] = widths[i].max(c.chars().count());
        }
    }
    for r in std::iter::once(&header).chain(rows.iter()) {
        let line: Vec<String> = r
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{c:<w$}", w = widths[i]))
            .collect();
        println!("{}", line.join("  ").trim_end());
    }
    Ok(())
}
