use crate::{Fixture, git_repository};

const ITEM: &str = "github:github.com/tj-agents/core:461";

fn register(fixture: &Fixture, session: &str) {
    fixture.ok(&["register", "--session", session, "--cwd", "/work/core"]);
}

#[test]
fn register_link_show_round_trip() {
    let fixture = Fixture::new();
    let data = fixture.ok(&[
        "register",
        "--session",
        "claude:abc",
        "--cwd",
        "/work/core",
        "--worktree",
        "/work/core.worktrees/Feature-X",
        "--terminal-kind",
        "windows-terminal",
        "--terminal-id",
        "tab-7",
        "--wrapper-pid",
        "4242",
        "--wrapper-started-at",
        "2026-11-08T10:00:00Z",
        "--started-at",
        "2026-11-08T10:00:01Z",
    ]);
    assert_eq!(data["session"]["state"], "active");
    assert_eq!(data["session"]["terminal"]["id"], "tab-7");

    fixture.ok(&["link", "--session", "claude:abc", "--item", ITEM]);
    let shown = fixture.ok(&["show", "claude:abc"])["session"].clone();
    assert_eq!(shown["session"], "claude:abc");
    assert_eq!(shown["worktree"], "/work/core.worktrees/Feature-X");
    assert_eq!(shown["wrapper"]["pid"], 4242);
    assert_eq!(shown["wrapper"]["started_at"], "2026-11-08T10:00:00Z");
    assert_eq!(shown["started_at"], "2026-11-08T10:00:01Z");
    assert_eq!(shown["links"][0]["item"]["ref"], ITEM);
    assert_eq!(shown["links"][0]["role"], "owner");
}

#[test]
fn an_item_lists_every_linked_agent() {
    let fixture = Fixture::new();
    register(&fixture, "claude:owner");
    register(&fixture, "codex:second");
    fixture.ok(&["link", "--session", "claude:owner", "--item", ITEM]);
    fixture.ok(&[
        "link",
        "--session",
        "codex:second",
        "--item",
        "https://github.com/tj-agents/core/issues/461",
        "--role",
        "reviewer",
    ]);

    let sessions = fixture.ok(&["sessions", "--item", ITEM]);
    let handles: Vec<&str> = sessions["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["session"].as_str().unwrap())
        .collect();
    assert_eq!(handles, ["claude:owner", "codex:second"]);

    let items = fixture.ok(&["items"]);
    assert_eq!(items["items"].as_array().unwrap().len(), 1);
    assert_eq!(items["items"][0]["sessions"][1]["role"], "reviewer");
}

#[test]
fn mark_closing_then_end() {
    let fixture = Fixture::new();
    register(&fixture, "claude:a");
    register(&fixture, "claude:b");
    register(&fixture, "claude:other");
    for session in ["claude:a", "claude:b"] {
        fixture.ok(&["link", "--session", session, "--item", ITEM]);
    }
    fixture.ok(&["end", "--session", "claude:b"]);

    let marked = fixture.ok(&["mark-closing", "--item", ITEM]);
    assert_eq!(marked["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(marked["sessions"][0]["state"], "closing");
    assert_eq!(
        fixture.ok(&["show", "claude:other"])["session"]["state"],
        "active"
    );

    let ended = fixture.ok(&["end", "--session", "claude:a", "--reason", "handed_off"]);
    assert_eq!(ended["session"]["state"], "ended");
    assert_eq!(ended["session"]["end_reason"], "handed_off");

    let again = fixture.run(&["end", "--session", "claude:a"]);
    assert_eq!(again.code, 4);
    assert_eq!(again.json["error"]["code"], "conflict");
}

#[test]
fn resuming_reactivates_an_ended_session() {
    let fixture = Fixture::new();
    register(&fixture, "codex:r");
    fixture.ok(&["link", "--session", "codex:r", "--item", ITEM]);
    fixture.ok(&["end", "--session", "codex:r"]);
    let resumed = fixture.ok(&["register", "--session", "codex:r", "--cwd", "/elsewhere"]);
    assert_eq!(resumed["session"]["state"], "active");
    assert_eq!(resumed["session"]["cwd"], "/elsewhere");
    assert_eq!(resumed["session"]["links"].as_array().unwrap().len(), 1);
}

#[test]
fn json_request_on_stdin() {
    let fixture = Fixture::new();
    let run = fixture.run_input(
        r#"{"command":"register","session":"claude:j","cwd":"/w","terminal_kind":"tmux","terminal_id":"%3"}"#,
    );
    assert_eq!(run.code, 0, "{}", run.json);
    let run = fixture.run_input(&format!(
        r#"{{"command":"link","session":"claude:j","item":"{ITEM}"}}"#
    ));
    assert_eq!(run.json["data"]["session"]["links"][0]["role"], "owner");
    let run = fixture.run_input(r#"{"command":"items","surprise":1}"#);
    assert_eq!(run.code, 0, "{}", run.json);
}

#[test]
fn bare_ids_resolve_against_the_origin_remote() {
    let fixture = Fixture::new();
    git_repository(fixture.dir.path(), "git@github.com:tj-agents/core.git");
    register(&fixture, "claude:bare");
    let linked = fixture.ok(&["link", "--session", "claude:bare", "--item", "#461"]);
    assert_eq!(linked["session"]["links"][0]["item"]["ref"], ITEM);
}

#[test]
fn failures_are_json_with_stable_codes() {
    let fixture = Fixture::new();
    let cases: [(&[&str], i32, &str); 5] = [
        (&["show", "claude:missing"], 3, "not-found"),
        (&["show", "no-colon"], 2, "invalid-input"),
        (
            &["link", "--session", "claude:x", "--item", "nonsense"],
            2,
            "invalid-input",
        ),
        (&["frobnicate"], 2, "invalid-input"),
        (
            &["register", "--session", "Claude:x", "--cwd", "/"],
            2,
            "invalid-input",
        ),
    ];
    for (args, code, name) in cases {
        let run = fixture.run(args);
        assert_eq!(run.code, code, "{args:?} -> {}", run.json);
        assert_eq!(run.json["ok"], false);
        assert_eq!(run.json["error"]["code"], name, "{args:?}");
    }
    let bare = fixture.run(&["sessions", "--item", "461"]);
    assert_eq!(bare.code, 2);
    assert!(
        bare.json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("origin")
    );
}

#[test]
fn unwritable_database_is_a_storage_failure() {
    let fixture = Fixture::new();
    let blocker = fixture.path("file");
    std::fs::write(&blocker, "").unwrap();
    let database = blocker.join("agent-store.db");
    let run = fixture.run(&["--database", database.to_str().unwrap(), "items"]);
    assert_eq!(run.code, 5, "{}", run.json);
    assert_eq!(run.json["error"]["code"], "storage");
}
