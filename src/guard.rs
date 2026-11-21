//! `ngate guard` (§8.3 of the design): for tmac's relay of Stop. If
//! extracted messages are left, stop the agent from ending its reply:
//! exit code 2 with the reason on standard error (tmac gives both to the
//! CLI as they are). Once per reply: when the stop already continues one
//! that was stopped (`TMAC_STOP_ACTIVE=true`), let it end.

use std::path::PathBuf;

use crate::error::Result;
use crate::lmm;
use crate::template::{Templates, Which};

pub fn run(address: Option<&str>, template: Option<&std::path::Path>) -> Result<i32> {
    if std::env::var("TMAC_STOP_ACTIVE").is_ok_and(|v| v == "true") {
        return Ok(0);
    }
    let templates = Templates::load(template)?;
    let cwd = std::env::current_dir()?;
    let paths = lmm::paths(&cwd, address)?;
    let Some(address) = address.map(str::to_string).or(paths.name) else {
        // Not taking part in lmm here.
        return Ok(0);
    };
    let Some(dir) = paths.receive_dir else {
        return Ok(0);
    };
    let mut left: Vec<PathBuf> = match std::fs::read_dir(&dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.path())
            .collect(),
        Err(_) => return Ok(0),
    };
    if left.is_empty() {
        return Ok(0);
    }
    left.sort();
    eprintln!("{}", templates.render(Which::Guard, &left, &address));
    Ok(2)
}
