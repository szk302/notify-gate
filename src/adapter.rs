//! Destinations and adapters (§6, §8 of the design).
//!
//! The built-in tmac adapter types the notice into a tmac agent with
//! `tmac wake`. Other adapters are external commands
//! `ngate-adapter-<NAME> <OP>` that read one JSON object on standard input
//! and answer with an exit code: 0 done (alive), 3 not now (try later),
//! 4 the destination is gone, 5 the operation is not supported, anything
//! else a failure (try later).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;

use serde_json::{Value, json};

use crate::config::{AfterDelivery, Config, Remind};
use crate::error::{Error, Result};
use crate::template::Templates;
use crate::util::{self, Ran};

/// What a delivery did.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Delivered,
    /// Not now: try again later (with the reason).
    Later(String),
    /// The destination is gone: end the link.
    Gone,
    /// `stop` was set.
    Stopped,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// A tmac agent `PROJECT/NAME`, and the instance the link belongs to.
    Tmac {
        agent: String,
        instance: Option<String>,
    },
    External {
        adapter: String,
        program: PathBuf,
        with: Value,
    },
}

#[derive(Debug, Clone)]
pub struct Destination {
    /// As given on the command line (`tmac`, `tmac:P/N` or a name).
    pub spec: String,
    pub target: Target,
    pub remind: bool,
    pub delete_after: bool,
    pub templates: Templates,
}

fn which(name: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|d| !d.as_os_str().is_empty())
        .map(|d| d.join(name))
        .find(|p| {
            std::fs::metadata(p)
                .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        })
}

fn tmac() -> Command {
    let bin = std::env::var_os("NGATE_TMAC")
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "tmac".into());
    Command::new(bin)
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.is_empty())
}

impl Destination {
    /// Resolve a destination and check it before anything starts.
    pub fn resolve(spec: &str, cfg: &Config, template: Option<&Path>) -> Result<Destination> {
        let (target, remind, after, dest_template) = if spec == "tmac" {
            // Started from tmac's relay: the agent is in the environment.
            let (Some(p), Some(n), Some(i)) =
                (env("TMAC_PROJECT"), env("TMAC_AGENT"), env("TMAC_INSTANCE"))
            else {
                return Err(Error::usage(
                    "the tmac destination needs TMAC_PROJECT, TMAC_AGENT and TMAC_INSTANCE \
                     (run it from a tmac relay, or give tmac:PROJECT/NAME)",
                ));
            };
            (
                Target::Tmac {
                    agent: format!("{p}/{n}"),
                    instance: Some(i),
                },
                None,
                None,
                None,
            )
        } else if let Some(agent) = spec.strip_prefix("tmac:") {
            let ok = agent
                .split_once('/')
                .is_some_and(|(p, n)| !p.is_empty() && !n.is_empty() && !n.contains('/'));
            if !ok {
                return Err(Error::usage(format!(
                    "give the agent as tmac:PROJECT/NAME, not {spec:?}"
                )));
            }
            (
                Target::Tmac {
                    agent: agent.to_string(),
                    instance: None,
                },
                None,
                None,
                None,
            )
        } else {
            let d = cfg.destinations.get(spec).ok_or_else(|| {
                Error::usage(format!(
                    "unknown destination {spec:?}: use tmac, tmac:PROJECT/NAME or a name in \
                     destinations of the configuration"
                ))
            })?;
            let program = format!("ngate-adapter-{}", d.adapter);
            let program = which(&program)
                .ok_or_else(|| Error::usage(format!("adapter {program} is not in PATH")))?;
            (
                Target::External {
                    adapter: d.adapter.clone(),
                    program,
                    with: d.with.clone(),
                },
                d.remind,
                d.after_delivery,
                d.template.clone(),
            )
        };
        let tmac = matches!(target, Target::Tmac { .. });
        // tmac agents get a reminder once by default; people do not.
        let remind = remind.unwrap_or(if tmac { Remind::Once } else { Remind::Never });
        let after = after.unwrap_or(AfterDelivery::Keep);
        if after == AfterDelivery::Delete {
            if tmac {
                return Err(Error::usage(
                    "the tmac destination keeps the extracted messages (the agent deletes them \
                     when done); after_delivery: delete is not allowed",
                ));
            }
            if remind == Remind::Once {
                return Err(Error::usage(format!(
                    "destination {spec}: remind: once needs the extracted messages, so it \
                     cannot go with after_delivery: delete"
                )));
            }
        }
        let templates = Templates::load(template.or(dest_template.as_deref()))?;
        Ok(Destination {
            spec: spec.to_string(),
            target,
            remind: remind == Remind::Once,
            delete_after: after == AfterDelivery::Delete,
            templates,
        })
    }

    /// Before the first wait: a tmac agent given by name gets its current
    /// instance, so that a restarted agent ends the link.
    pub fn start(&mut self, stop: &AtomicBool) -> Outcome {
        if let Target::Tmac { agent, instance } = &mut self.target
            && instance.is_none()
        {
            match tmac_field(agent, "instance", stop) {
                Field::Value(v) => *instance = Some(v),
                Field::Gone => return Outcome::Gone,
                Field::Stopped => return Outcome::Stopped,
            }
        }
        Outcome::Delivered
    }

    /// Whether the destination is still there. `Later` for unknown.
    pub fn alive(&self, recipient: &str, stop: &AtomicBool) -> Outcome {
        match &self.target {
            Target::Tmac { agent, instance } => {
                match tmac_field(agent, "instance", stop) {
                    Field::Value(v) if Some(&v) == instance.as_ref() => {}
                    Field::Stopped => return Outcome::Stopped,
                    // Restarted (another instance) or gone.
                    _ => return Outcome::Gone,
                }
                match tmac_field(agent, "state", stop) {
                    Field::Value(s) if s != "exited" => Outcome::Delivered,
                    Field::Stopped => Outcome::Stopped,
                    _ => Outcome::Gone,
                }
            }
            Target::External { .. } => {
                let input = json!({
                    "recipient": recipient,
                    "destination": self.destination_json(),
                });
                match self.call("alive", &input, stop) {
                    // Not supported: take it as alive.
                    Outcome::Later(r) if r == "unsupported" => Outcome::Delivered,
                    o => o,
                }
            }
        }
    }

    fn destination_json(&self) -> Value {
        match &self.target {
            Target::Tmac { agent, .. } => json!({"adapter": "tmac", "with": {"agent": agent}}),
            Target::External { adapter, with, .. } => json!({"adapter": adapter, "with": with}),
        }
    }

    /// Deliver a notice about `messages` (`kind`: `new` or `reminder`).
    pub fn deliver(
        &self,
        kind: &str,
        recipient: &str,
        messages: &[Value],
        text: &str,
        stop: &AtomicBool,
    ) -> Outcome {
        match &self.target {
            Target::Tmac { agent, .. } => {
                let mut c = tmac();
                c.arg("wake").arg(agent).arg(text);
                match util::run(&mut c, None, stop) {
                    Err(e) => Outcome::Later(format!("cannot run tmac: {e}")),
                    Ok(Ran::Stopped) => Outcome::Stopped,
                    Ok(Ran::Exited(code, _, err)) => match code {
                        0 => Outcome::Delivered,
                        4 => Outcome::Gone,
                        // Typed but not confirmed: tmac holds the agent for a
                        // person to look at; do not type it again.
                        6 => Outcome::Delivered,
                        c => Outcome::Later(format!(
                            "tmac wake exit {c}: {}",
                            String::from_utf8_lossy(&err).trim()
                        )),
                    },
                }
            }
            Target::External { .. } => {
                let input = json!({
                    "kind": kind,
                    "recipient": recipient,
                    "destination": self.destination_json(),
                    "messages": messages,
                    "text": text,
                });
                match self.call("deliver", &input, stop) {
                    Outcome::Later(r) if r == "unsupported" => {
                        Outcome::Later("the adapter does not support deliver".into())
                    }
                    o => o,
                }
            }
        }
    }

    fn call(&self, op: &str, input: &Value, stop: &AtomicBool) -> Outcome {
        let Target::External { program, .. } = &self.target else {
            unreachable!("external adapters only");
        };
        let mut c = Command::new(program);
        c.arg(op);
        let data = input.to_string();
        match util::run(&mut c, Some(data.as_bytes()), stop) {
            Err(e) => Outcome::Later(format!("cannot run {}: {e}", program.display())),
            Ok(Ran::Stopped) => Outcome::Stopped,
            Ok(Ran::Exited(code, _, err)) => match code {
                0 => Outcome::Delivered,
                4 => Outcome::Gone,
                5 => Outcome::Later("unsupported".into()),
                c => Outcome::Later(format!(
                    "{op} exit {c}: {}",
                    String::from_utf8_lossy(&err).trim()
                )),
            },
        }
    }
}

enum Field {
    Value(String),
    Gone,
    Stopped,
}

/// `tmac status AGENT --field FIELD`. Any failure counts as gone, like a
/// missing agent (exit code 4): a link must not outlive its agent.
fn tmac_field(agent: &str, field: &str, stop: &AtomicBool) -> Field {
    let mut c = tmac();
    c.args(["status", agent, "--field", field]);
    match util::run(&mut c, None, stop) {
        Ok(Ran::Stopped) => Field::Stopped,
        Ok(Ran::Exited(0, out, _)) => {
            Field::Value(String::from_utf8_lossy(&out).trim().to_string())
        }
        _ => Field::Gone,
    }
}
