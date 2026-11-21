//! End-to-end tests with the real lmm, a fake tmac and a fake adapter.

mod common;

use common::{Env, wait_child};

fn wakes(env: &Env) -> Vec<String> {
    env.calls("tmac.calls")
        .iter()
        .filter(|c| c[0] == "wake")
        .map(|c| c[2].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn tmac_agents_are_woken_and_reminded_once() {
    let env = Env::new();
    let mut link = env.link(&["me", "tmac:proj/agent"]);
    env.until("the link to run", || env.list().contains("running"));
    env.post("hello");
    env.until("a notice and a reminder", || wakes(&env).len() == 2);
    let extracted = env.extracted();
    assert_eq!(extracted.len(), 1);
    let w = wakes(&env);
    assert!(
        w[0].starts_with("1 new lmm message(s) for me:\n- "),
        "{w:?}"
    );
    assert!(w[0].contains(&extracted[0].display().to_string()), "{w:?}");
    assert!(w[1].contains("still left"), "{w:?}");
    // The agent is woken with its name; the message stays (kept).
    let call = env
        .calls("tmac.calls")
        .into_iter()
        .find(|c| c[0] == "wake")
        .unwrap();
    assert_eq!(call[1], "proj/agent");
    assert!(extracted[0].is_dir());

    // A message the agent deals with (deletes) is not told about again.
    std::fs::remove_dir_all(&extracted[0]).unwrap();
    env.post("second");
    env.until("the second notice", || wakes(&env).len() == 4);
    assert!(wakes(&env)[2].starts_with("1 new"));

    // The agent ends: so does the link.
    env.set("tmac.state", "exited");
    assert_eq!(wait_child(&mut link), 0);
    assert!(env.list().contains("stopped"));
    let log = std::fs::read_dir(env.ngate_home.join("links"))
        .unwrap()
        .map(|e| std::fs::read_to_string(e.unwrap().path().join("log")).unwrap())
        .collect::<String>();
    assert!(log.contains("end: the destination is gone"), "{log}");
}

#[test]
fn a_restarted_tmac_agent_ends_the_link() {
    let env = Env::new();
    let mut link = env
        .ngate(&["link", "me", "tmac"])
        .env("TMAC_PROJECT", "proj")
        .env("TMAC_AGENT", "agent")
        .env("TMAC_INSTANCE", "i1")
        .spawn()
        .unwrap();
    env.until("the link to run", || env.list().contains("running"));
    env.set("tmac.instance", "i2");
    assert_eq!(wait_child(&mut link), 0);
}

#[test]
fn external_adapters_get_json_and_templates() {
    let env = Env::with_config(
        "destinations:\n  ext:\n    adapter: fake\n    after_delivery: delete\n    template: tpl.yml\n    with: {channel: C1}\n",
    );
    std::fs::write(
        env.ngate_home.join("tpl.yml"),
        "new: \"N={count} for {address}\\n{paths}\"\n",
    )
    .unwrap();
    // The first delivery is refused for now (exit 3), then accepted.
    env.set("adapter.deliver.code", "3");
    let mut link = env.link(&["me", "ext"]);
    env.until("the link to run", || env.list().contains("running"));
    let id = env.post("hello");
    let delivers = || {
        env.calls("adapter.calls")
            .into_iter()
            .filter(|c| c["op"] == "deliver")
            .collect::<Vec<_>>()
    };
    env.until("the first try", || !delivers().is_empty());
    env.unset("adapter.deliver.code");
    env.until("the second try", || delivers().len() >= 2);
    let input = &delivers()[1]["input"];
    assert_eq!(input["kind"], "new");
    assert_eq!(input["recipient"], "me");
    assert_eq!(input["destination"]["adapter"], "fake");
    assert_eq!(input["destination"]["with"]["channel"], "C1");
    let m = &input["messages"][0];
    assert_eq!(m["subject"], "hello");
    assert_eq!(m["from"], "sender");
    assert_eq!(m["id"], id.as_str());
    assert!(
        input["text"]
            .as_str()
            .unwrap()
            .starts_with("N=1 for me\n- ")
    );
    // Deleted after the delivery (the original stays in lmm).
    env.until("the deletion", || env.extracted().is_empty());
    assert!(
        env.calls("adapter.calls")
            .iter()
            .any(|c| c["op"] == "alive"),
        "alive is asked"
    );
    // ngate stop ends the link.
    env.expect(&["stop", "me"], 0);
    assert_eq!(wait_child(&mut link), 0);
    assert!(env.list().contains("stopped"));
}

#[test]
fn one_link_per_address_and_detach() {
    let env = Env::new();
    env.expect(&["link", "--detach", "me", "tmac:proj/agent"], 0);
    env.until("the link to run", || env.list().contains("running"));
    // A second one does nothing.
    env.expect(&["link", "--detach", "me", "tmac:proj/agent"], 0);
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert_eq!(env.list().matches("running").count(), 1);
    env.expect(&["stop", "me"], 0);
    assert!(env.list().contains("stopped"));
    // Stopping a stopped link is fine.
    env.expect(&["stop", "me"], 0);
}

#[test]
fn guard_stops_the_reply_while_messages_are_left() {
    let env = Env::new();
    env.expect(&["guard", "me"], 0);
    env.post("hello");
    let o = env.lmm(Some("me"), &["receive"]);
    assert!(o.status.success(), "{o:?}");
    let left = env.extracted();
    assert_eq!(left.len(), 1);
    let o = env.expect(&["guard", "me"], 2);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains(&left[0].display().to_string()), "{err}");
    // Once per reply: a stop that continues a stopped one is let through.
    let o = env
        .ngate(&["guard", "me"])
        .env("TMAC_STOP_ACTIVE", "true")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(0));
    // The address can come from lmm (LMM_NAME here), the text from a file.
    std::fs::write(env.root().join("g.yml"), "guard: \"left: {count}\"\n").unwrap();
    let o = env
        .ngate(&[
            "guard",
            "--template",
            env.root().join("g.yml").to_str().unwrap(),
        ])
        .env("LMM_NAME", "me")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(String::from_utf8_lossy(&o.stderr).trim(), "left: 1");
    std::fs::remove_dir_all(&left[0]).unwrap();
    env.expect(&["guard", "me"], 0);
}

#[test]
fn usage_errors() {
    let env = Env::with_config(
        "destinations:\n  both:\n    adapter: fake\n    after_delivery: delete\n    remind: once\n  missing:\n    adapter: nowhere\n",
    );
    // tmac without the agent in the environment.
    env.expect(&["link", "me", "tmac"], 2);
    env.expect(&["link", "me", "tmac:noslash"], 2);
    env.expect(&["link", "Bad", "tmac:p/a"], 2);
    env.expect(&["link", "me", "nope"], 2);
    env.expect(&["link", "me", "both"], 2);
    env.expect(&["link", "me", "missing"], 2);
    env.expect(&["stop", "Bad"], 2);
    let broken = Env::with_config("destinations:\n  x:\n    adapter: fake\n    remind: twice\n");
    broken.expect(&["link", "me", "x"], 2);
}
