//! The configuration file `$NGATE_HOME/config.yml`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;

use crate::error::{Error, Result, warn};
use crate::home::Home;

/// Whether to tell again about messages left after a notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Remind {
    Once,
    Never,
}

/// What to do with the extracted message after a notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterDelivery {
    Keep,
    Delete,
}

/// `destinations.<NAME>`.
#[derive(Debug, Clone)]
pub struct DestConfig {
    pub adapter: String,
    pub remind: Option<Remind>,
    pub after_delivery: Option<AfterDelivery>,
    pub with: Value,
    pub template: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct Config {
    /// Seconds between checks (lmm wait's time limit, retries).
    pub interval: f64,
    pub destinations: BTreeMap<String, DestConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            interval: 30.0,
            destinations: BTreeMap::new(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    interval_sec: Option<f64>,
    #[serde(default)]
    destinations: BTreeMap<String, RawDest>,
    /// For `ngate ensure` (not yet).
    links: Option<serde_yaml_ng::Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDest {
    adapter: String,
    remind: Option<String>,
    after_delivery: Option<String>,
    #[serde(default)]
    with: Value,
    template: Option<PathBuf>,
    /// For replies from outside (not yet).
    allow: Option<serde_yaml_ng::Value>,
}

pub fn load(home: &Home) -> Result<Config> {
    let path = home.config();
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(Error::usage(format!("cannot read {}: {e}", path.display()))),
    };
    parse(&text, home).map_err(|e| Error::usage(format!("{}: {e}", path.display())))
}

fn parse(text: &str, home: &Home) -> std::result::Result<Config, String> {
    if text.trim().is_empty() {
        return Ok(Config::default());
    }
    let raw: Raw = serde_yaml_ng::from_str(text).map_err(|e| e.to_string())?;
    let mut cfg = Config::default();
    if let Some(i) = raw.interval_sec {
        if !(i.is_finite() && i > 0.0) {
            return Err("interval_sec must be a positive number".into());
        }
        cfg.interval = i;
    }
    if raw.links.is_some() {
        warn("links (for ngate ensure) are not used yet");
    }
    for (name, d) in raw.destinations {
        if name == "tmac" || name.starts_with("tmac:") || name.is_empty() {
            return Err(format!(
                "destinations.{name}: this name is taken by the tmac adapter"
            ));
        }
        let remind = match d.remind.as_deref() {
            None => None,
            Some("once") => Some(Remind::Once),
            Some("never") => Some(Remind::Never),
            Some(r) => {
                return Err(format!(
                    "destinations.{name}.remind must be once or never, not {r:?}"
                ));
            }
        };
        let after_delivery = match d.after_delivery.as_deref() {
            None => None,
            Some("keep") => Some(AfterDelivery::Keep),
            Some("delete") => Some(AfterDelivery::Delete),
            Some(a) => {
                return Err(format!(
                    "destinations.{name}.after_delivery must be keep or delete, not {a:?}"
                ));
            }
        };
        if d.allow.is_some() {
            warn(format!(
                "destinations.{name}.allow (for replies from outside) is not used yet"
            ));
        }
        // A relative template is relative to the configuration.
        let template = d.template.map(|t| {
            if t.is_absolute() {
                t
            } else {
                home.root.join(t)
            }
        });
        cfg.destinations.insert(
            name,
            DestConfig {
                adapter: d.adapter,
                remind,
                after_delivery,
                with: d.with,
                template,
            },
        );
    }
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> Home {
        Home {
            root: PathBuf::from("/h"),
        }
    }

    #[test]
    fn parsing() {
        let c = parse("", &home()).unwrap();
        assert_eq!(c.interval, 30.0);
        let c = parse(
            "interval_sec: 2\ndestinations:\n  slack-me:\n    adapter: slack\n    remind: never\n    after_delivery: delete\n    with: {channel: C1}\n    template: t.yml\n",
            &home(),
        )
        .unwrap();
        assert_eq!(c.interval, 2.0);
        let d = &c.destinations["slack-me"];
        assert_eq!(d.adapter, "slack");
        assert_eq!(d.after_delivery, Some(AfterDelivery::Delete));
        assert_eq!(d.with["channel"], "C1");
        assert_eq!(
            d.template.as_deref(),
            Some(std::path::Path::new("/h/t.yml"))
        );
        assert!(parse("typo: 1\n", &home()).is_err());
        assert!(
            parse(
                "destinations:\n  x:\n    adapter: a\n    remind: twice\n",
                &home()
            )
            .is_err()
        );
        assert!(parse("destinations:\n  tmac:\n    adapter: a\n", &home()).is_err());
        assert!(parse("interval_sec: 0\n", &home()).is_err());
    }
}
