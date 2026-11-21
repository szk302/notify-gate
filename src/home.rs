//! The state directory: `$NGATE_HOME`, else `~/.ngate`.

use std::path::PathBuf;

use crate::error::{Error, Result};

#[derive(Debug, Clone)]
pub struct Home {
    pub root: PathBuf,
}

impl Home {
    pub fn resolve() -> Result<Home> {
        let root = match std::env::var_os("NGATE_HOME").filter(|v| !v.is_empty()) {
            Some(p) => std::path::absolute(PathBuf::from(p))?,
            None => {
                let home = std::env::var_os("HOME")
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| Error::usage("HOME is not set; set NGATE_HOME"))?;
                PathBuf::from(home).join(".ngate")
            }
        };
        Ok(Home { root })
    }

    /// The configuration file.
    pub fn config(&self) -> PathBuf {
        self.root.join("config.yml")
    }

    /// The directory of every link's state.
    pub fn links(&self) -> PathBuf {
        self.root.join("links")
    }
}
