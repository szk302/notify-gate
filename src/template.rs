//! Texts of the notices (§7.1 of the design): the defaults only say how
//! many messages there are and where; how to deal with them belongs to the
//! recipient's own rules. A YAML file with `new`, `reminder` and `guard`
//! changes them. Placeholders: `{count}`, `{paths}` (one `- PATH` line
//! each), `{address}`; `{{` and `}}` for braces.

use std::path::Path;

use serde::Deserialize;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    New,
    Reminder,
    Guard,
}

#[derive(Debug, Clone)]
pub struct Templates {
    new: String,
    reminder: String,
    guard: String,
}

impl Default for Templates {
    fn default() -> Self {
        Templates {
            new: "{count} new lmm message(s) for {address}:\n{paths}".into(),
            reminder: "{count} lmm message(s) for {address} are still left:\n{paths}".into(),
            guard: "lmm messages you have not dealt with are left:\n{paths}".into(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    new: Option<String>,
    reminder: Option<String>,
    guard: Option<String>,
}

fn render(
    t: &str,
    count: usize,
    paths: &str,
    address: &str,
) -> std::result::Result<String, String> {
    let mut out = String::new();
    let mut rest = t;
    while let Some(i) = rest.find(['{', '}']) {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if let Some(r) = tail.strip_prefix("{{") {
            out.push('{');
            rest = r;
        } else if let Some(r) = tail.strip_prefix("}}") {
            out.push('}');
            rest = r;
        } else if tail.starts_with('{') {
            let end = tail.find('}').ok_or("an unclosed {")?;
            match &tail[1..end] {
                "count" => out.push_str(&count.to_string()),
                "paths" => out.push_str(paths),
                "address" => out.push_str(address),
                other => return Err(format!("unknown placeholder {{{other}}}")),
            }
            rest = &tail[end + 1..];
        } else {
            return Err("a lone } (write }} for a brace)".into());
        }
    }
    out.push_str(rest);
    Ok(out)
}

impl Templates {
    /// The defaults, changed by a template file if given.
    pub fn load(file: Option<&Path>) -> Result<Templates> {
        let mut t = Templates::default();
        let Some(f) = file else { return Ok(t) };
        let text = std::fs::read_to_string(f)
            .map_err(|e| Error::usage(format!("cannot read the template {}: {e}", f.display())))?;
        let raw: Raw = serde_yaml_ng::from_str(&text)
            .map_err(|e| Error::usage(format!("{}: {e}", f.display())))?;
        for (slot, v) in [
            (&mut t.new, raw.new),
            (&mut t.reminder, raw.reminder),
            (&mut t.guard, raw.guard),
        ] {
            if let Some(v) = v {
                render(&v, 1, "- x", "a")
                    .map_err(|e| Error::usage(format!("{}: {e}", f.display())))?;
                *slot = v;
            }
        }
        Ok(t)
    }

    pub fn render(&self, which: Which, paths: &[impl AsRef<Path>], address: &str) -> String {
        let t = match which {
            Which::New => &self.new,
            Which::Reminder => &self.reminder,
            Which::Guard => &self.guard,
        };
        let lines: Vec<String> = paths
            .iter()
            .map(|p| format!("- {}", p.as_ref().display()))
            .collect();
        // Checked when loaded.
        render(t, paths.len(), &lines.join("\n"), address).unwrap_or_else(|_| t.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendering() {
        assert_eq!(
            render("{count} for {address} {{x}}:\n{paths}", 2, "- a\n- b", "me").unwrap(),
            "2 for me {x}:\n- a\n- b"
        );
        assert!(render("{nope}", 1, "", "").is_err());
        assert!(render("{count", 1, "", "").is_err());
        assert!(render("a } b", 1, "", "").is_err());
        let t = Templates::default();
        let text = t.render(Which::New, &["/x/1"], "me");
        assert_eq!(text, "1 new lmm message(s) for me:\n- /x/1");
    }

    #[test]
    fn files() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("t.yml");
        std::fs::write(&f, "new: \"N={count}\\n{paths}\"\n").unwrap();
        let t = Templates::load(Some(&f)).unwrap();
        assert_eq!(t.render(Which::New, &["/p"], "me"), "N=1\n- /p");
        assert!(
            t.render(Which::Guard, &["/p"], "me")
                .starts_with("lmm messages")
        );
        std::fs::write(&f, "new: \"{oops}\"\n").unwrap();
        assert!(Templates::load(Some(&f)).is_err());
        std::fs::write(&f, "typo: x\n").unwrap();
        assert!(Templates::load(Some(&f)).is_err());
    }
}
