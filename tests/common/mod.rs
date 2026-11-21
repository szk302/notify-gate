//! Test harness: a temporary lmm project, a fake tmac and a fake adapter.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

pub struct Env {
    pub tmp: tempfile::TempDir,
    pub work: PathBuf,
    pub ngate_home: PathBuf,
    pub bin: PathBuf,
    path: std::ffi::OsString,
}

/// The lmm to use: `NGATE_TEST_LMM`, else `lmm` in PATH. It must have
/// `lmm config paths`.
fn lmm_dir() -> Option<PathBuf> {
    std::env::var_os("NGATE_TEST_LMM").map(|p| PathBuf::from(p).parent().unwrap().to_path_buf())
}

const FAKE_TMAC: &str = r#"#!/usr/bin/env python3
# A fake tmac: logs its arguments; `status` answers from tmac.state
# ("gone" exits 4), `wake` exits with the code in tmac.wake (default 0).
import json, os, sys
d = os.environ["NGATE_TEST_DIR"]
with open(os.path.join(d, "tmac.calls"), "a") as f:
    f.write(json.dumps(sys.argv[1:]) + "\n")
def read(name, default):
    try:
        with open(os.path.join(d, name)) as f:
            return f.read().strip()
    except OSError:
        return default
if sys.argv[1] == "status":
    state = read("tmac.state", "idle")
    if state == "gone":
        sys.exit(4)
    field = sys.argv[sys.argv.index("--field") + 1]
    print(read("tmac.instance", "i1") if field == "instance" else state)
    sys.exit(0)
if sys.argv[1] == "wake":
    sys.exit(int(read("tmac.wake", "0")))
sys.exit(2)
"#;

const FAKE_ADAPTER: &str = r#"#!/usr/bin/env python3
# A fake adapter: logs each call; exits with the code in adapter.<OP>.code.
import json, os, sys
d = os.environ["NGATE_TEST_DIR"]
op = sys.argv[1]
data = json.loads(sys.stdin.read())
with open(os.path.join(d, "adapter.calls"), "a") as f:
    f.write(json.dumps({"op": op, "input": data}) + "\n")
try:
    with open(os.path.join(d, "adapter." + op + ".code")) as f:
        sys.exit(int(f.read().strip()))
except OSError:
    sys.exit(0)
"#;

fn script(path: &Path, text: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

impl Env {
    pub fn new() -> Env {
        Env::with_config("")
    }

    /// `extra` is appended to the configuration (after `interval_sec: 1`).
    pub fn with_config(extra: &str) -> Env {
        let tmp = tempfile::Builder::new()
            .prefix("ngate-test.")
            .tempdir()
            .unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let work = root.join("work");
        let ngate_home = root.join("ngate");
        let bin = root.join("bin");
        for d in [&work, &ngate_home, &bin, &root.join("home")] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(
            work.join(".lmm.yml"),
            "root: true\ndata_dir: \"{root_dir}/.lmm\"\n",
        )
        .unwrap();
        std::fs::write(
            ngate_home.join("config.yml"),
            format!("interval_sec: 1\n{extra}"),
        )
        .unwrap();
        script(&bin.join("tmac"), FAKE_TMAC);
        script(&bin.join("ngate-adapter-fake"), FAKE_ADAPTER);
        let mut dirs = vec![bin.clone()];
        if let Some(d) = lmm_dir() {
            dirs.push(d);
        }
        dirs.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        let path = std::env::join_paths(dirs).unwrap();
        let env = Env {
            tmp,
            work,
            ngate_home,
            bin,
            path,
        };
        let o = env.lmm(None, &["config", "paths"]);
        assert!(
            o.status.success(),
            "these tests need lmm with `lmm config paths` (set NGATE_TEST_LMM): {}",
            String::from_utf8_lossy(&o.stderr)
        );
        for b in ["me", "sender"] {
            let o = env.lmm(None, &["box", "create", b]);
            assert!(o.status.success(), "{o:?}");
        }
        env
    }

    pub fn root(&self) -> PathBuf {
        self.tmp.path().canonicalize().unwrap()
    }

    fn base(&self, program: &str) -> Command {
        let mut c = Command::new(program);
        for (k, _) in std::env::vars() {
            if k.starts_with("LMM_") || k.starts_with("TMAC_") || k.starts_with("NGATE_") {
                c.env_remove(k);
            }
        }
        c.env("PATH", &self.path)
            .env("HOME", self.root().join("home"))
            .env("NGATE_HOME", &self.ngate_home)
            .env("NGATE_TEST_DIR", self.root())
            .current_dir(&self.work)
            .stdin(Stdio::null());
        c
    }

    pub fn lmm(&self, name: Option<&str>, args: &[&str]) -> Output {
        let mut c = self.base("lmm");
        if let Some(n) = name {
            c.env("LMM_NAME", n);
        }
        c.args(args).output().unwrap()
    }

    pub fn ngate(&self, args: &[&str]) -> Command {
        let mut c = self.base(env!("CARGO_BIN_EXE_ngate"));
        c.args(args);
        c
    }

    pub fn run(&self, args: &[&str]) -> Output {
        self.ngate(args).output().unwrap()
    }

    pub fn expect(&self, args: &[&str], code: i32) -> Output {
        let o = self.run(args);
        assert_eq!(
            o.status.code(),
            Some(code),
            "ngate {args:?}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        o
    }

    /// Start `ngate link` in the foreground as a child.
    pub fn link(&self, args: &[&str]) -> Child {
        let mut a = vec!["link"];
        a.extend_from_slice(args);
        self.ngate(&a)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    /// Send a message from `sender` to `me`; returns its ID.
    pub fn post(&self, subject: &str) -> String {
        let d = self.root().join(format!("draft-{}", uuid_like()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("index.md"), "body\n").unwrap();
        let o = self.lmm(
            Some("sender"),
            &["post", "-t", "me", "-s", subject, d.to_str().unwrap()],
        );
        assert!(o.status.success(), "{o:?}");
        String::from_utf8_lossy(&o.stdout)
            .split_whitespace()
            .next()
            .unwrap()
            .to_string()
    }

    pub fn inbox(&self) -> PathBuf {
        self.work.join(".lmm-inbox/me")
    }

    /// Directories extracted for `me`.
    pub fn extracted(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(self.inbox())
            .map(|e| {
                e.filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| {
                        p.is_dir() && !p.file_name().unwrap().to_string_lossy().starts_with('.')
                    })
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }

    pub fn calls(&self, name: &str) -> Vec<Value> {
        std::fs::read_to_string(self.root().join(name))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    pub fn set(&self, name: &str, value: &str) {
        std::fs::write(self.root().join(name), value).unwrap();
    }

    pub fn unset(&self, name: &str) {
        let _ = std::fs::remove_file(self.root().join(name));
    }

    pub fn list(&self) -> String {
        String::from_utf8_lossy(&self.expect(&["list"], 0).stdout).into_owned()
    }

    /// Wait until `f` holds (up to 20 s).
    pub fn until(&self, what: &str, f: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(20);
        while Instant::now() < end {
            if f() {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
        panic!("timed out waiting for {what}");
    }
}

fn uuid_like() -> String {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    format!(
        "{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

/// Wait for a child to end (up to 20 s); returns its exit code.
pub fn wait_child(child: &mut Child) -> i32 {
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end {
        if let Some(s) = child.try_wait().unwrap() {
            return s.code().unwrap_or(-1);
        }
        thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    panic!("the link did not end");
}
