//! lmm, used through its command line only (notify-gate is one of lmm's
//! users; it does not read lmm's data directory).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;

use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::util::{self, Ran};

fn lmm(cwd: &Path, name: Option<&str>) -> Command {
    let bin = std::env::var_os("NGATE_LMM")
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "lmm".into());
    let mut c = Command::new(bin);
    c.current_dir(cwd);
    if let Some(n) = name {
        c.env("LMM_NAME", n);
    }
    c
}

/// What `lmm config paths` resolves.
#[derive(Debug, Clone)]
pub struct Paths {
    pub name: Option<String>,
    pub data_dir: PathBuf,
    pub receive_dir: Option<PathBuf>,
}

/// Your address (or `name`'s) and the directories, as lmm resolves them in
/// `cwd`.
pub fn paths(cwd: &Path, name: Option<&str>) -> Result<Paths> {
    let out = lmm(cwd, name)
        .args(["config", "paths"])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| Error::runtime(format!("cannot run lmm: {e}")))?;
    let err = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        let hint = if err.contains("unrecognized subcommand") {
            " (notify-gate needs an lmm with `lmm config paths`)"
        } else {
            ""
        };
        return Err(Error::runtime(format!(
            "lmm config paths failed{hint}: {}",
            err.trim()
        )));
    }
    let map: BTreeMap<String, String> = serde_yaml_ng::from_slice(&out.stdout)
        .map_err(|e| Error::runtime(format!("cannot read lmm config paths: {e}")))?;
    let data_dir = map
        .get("data_dir")
        .map(PathBuf::from)
        .ok_or_else(|| Error::runtime("lmm config paths gave no data_dir"))?;
    Ok(Paths {
        name: map.get("name").cloned(),
        data_dir,
        receive_dir: map.get("receive_dir").map(PathBuf::from),
    })
}

/// How `lmm wait` ended.
#[derive(Debug)]
pub enum Waited {
    /// Extracted messages (possibly with some failures, logged as `note`).
    Got(Vec<PathBuf>, Option<String>),
    Timeout,
    Failed(i32, String),
    Stopped,
}

/// Wait for new messages of `name` and extract them all.
pub fn wait(cwd: &Path, name: &str, timeout: f64, stop: &AtomicBool) -> Result<Waited> {
    let mut c = lmm(cwd, Some(name));
    c.args(["wait", "--all", "--timeout", &format!("{timeout}")]);
    let ran = util::run(&mut c, None, stop)
        .map_err(|e| Error::runtime(format!("cannot run lmm wait: {e}")))?;
    Ok(match ran {
        Ran::Stopped => Waited::Stopped,
        Ran::Exited(code, out, err) => {
            let paths: Vec<PathBuf> = String::from_utf8_lossy(&out)
                .lines()
                .filter(|l| !l.is_empty())
                .map(PathBuf::from)
                .collect();
            let err = String::from_utf8_lossy(&err).trim().to_string();
            match code {
                3 => Waited::Timeout,
                // lmm wait prints what it extracted even when some failed.
                _ if !paths.is_empty() => Waited::Got(
                    paths,
                    (code != 0).then(|| format!("lmm wait exit {code}: {err}")),
                ),
                0 => Waited::Got(Vec::new(), None),
                c if c > 128 => Waited::Stopped,
                c => Waited::Failed(c, err),
            }
        }
    })
}

/// A message for adapters: the values of `lmm show --json`, or the path
/// only if lmm cannot show it.
pub fn message(cwd: &Path, name: &str, path: &Path) -> Value {
    let shown = lmm(cwd, Some(name))
        .arg("show")
        .arg(path)
        .arg("--json")
        .stdin(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok());
    let Some(v) = shown else {
        return json!({ "path": path });
    };
    let fm = &v["frontmatter"];
    let s = |k: &str| fm.get(k).cloned().unwrap_or(Value::Null);
    json!({
        "path": v.get("path").cloned().unwrap_or_else(|| json!(path)),
        "id": s("lmm_id"),
        "from": s("lmm_from"),
        "to": s("lmm_to"),
        "subject": s("lmm_subject"),
        "thread_id": s("lmm_thread_id"),
        "in_reply_to": s("lmm_in_reply_to"),
        "created_at": s("lmm_created_at"),
    })
}

/// Whether the mailbox of `name` exists.
pub fn box_exists(cwd: &Path, name: &str) -> Result<bool> {
    let out = lmm(cwd, None)
        .args(["box", "list"])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| Error::runtime(format!("cannot run lmm: {e}")))?;
    if !out.status.success() {
        return Err(Error::runtime(format!(
            "lmm box list failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .any(|l| l.split('\t').next() == Some(name)))
}
