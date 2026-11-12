use crate::Fixture;

const ITEM: &str = "github:github.com/tj-agents/core:461";

fn launch(fixture: &Fixture, launch_id: &str, host: &str) -> serde_json::Value {
    fixture.ok(&[
        "launch",
        "--launch-id",
        launch_id,
        "--host",
        host,
        "--title",
        "core 461",
        "--cwd",
        "/work/core.worktrees/Feature-461",
        "--worktree",
        "/work/core.worktrees/Feature-461",
        "--item",
        ITEM,
        "--terminal-kind",
        "windows-terminal",
        "--terminal-id",
        "tab-3",
        "--wrapper-pid",
        "900",
        "--wrapper-started-at",
        "2026-11-08T10:00:00Z",
        "--parent",
        "claude:parent",
    ])["launch"]
        .clone()
}

#[test]
fn a_session_binds_to_its_launch_and_inherits_it() {
    let fixture = Fixture::new();
    let recorded = launch(&fixture, "u-1", "claude");
    assert_eq!(recorded["state"], "active");
    assert_eq!(recorded["item"]["ref"], ITEM);
    assert!(recorded["session"].is_null());

    let pending = fixture.ok(&["show", "--launch-id", "u-1"]);
    assert_eq!(pending["launch"]["wrapper"]["pid"], 900);
    assert!(pending["session"].is_null());

    let registered = fixture.ok(&[
        "register",
        "--session",
        "claude:abc",
        "--cwd",
        "/work/core.worktrees/Feature-461",
        "--launch-id",
        "u-1",
    ])["session"]
        .clone();
    assert_eq!(registered["title"], "core 461");
    assert_eq!(registered["parent"], "claude:parent");
    assert_eq!(registered["launch_id"], "u-1");
    assert_eq!(registered["terminal"]["id"], "tab-3");
    assert_eq!(registered["wrapper"]["pid"], 900);
    assert_eq!(registered["links"][0]["item"]["ref"], ITEM);
    assert_eq!(registered["links"][0]["role"], "owner");

    let bound = fixture.ok(&["show", "--launch-id", "u-1"]);
    assert_eq!(bound["launch"]["session"], "claude:abc");
    assert_eq!(bound["session"]["session"], "claude:abc");

    let titled = fixture.ok(&["sessions", "--title", "core 461"]);
    assert_eq!(titled["sessions"][0]["session"], "claude:abc");
}

#[test]
fn a_resumed_session_moves_to_its_new_launch() {
    let fixture = Fixture::new();
    launch(&fixture, "first", "codex");
    fixture.ok(&[
        "register",
        "--session",
        "codex:t",
        "--cwd",
        "/w",
        "--launch-id",
        "first",
    ]);
    fixture.ok(&[
        "link",
        "--session",
        "codex:t",
        "--item",
        ITEM,
        "--role",
        "reviewer",
    ]);
    fixture.ok(&["end", "--session", "codex:t"]);
    launch(&fixture, "second", "codex");
    let resumed = fixture.ok(&[
        "register",
        "--session",
        "codex:t",
        "--cwd",
        "/w",
        "--launch-id",
        "second",
    ])["session"]
        .clone();
    assert_eq!(resumed["state"], "active");
    assert_eq!(resumed["launch_id"], "second");
    assert_eq!(resumed["links"].as_array().unwrap().len(), 1);
    assert_eq!(resumed["links"][0]["role"], "reviewer");
}

#[test]
fn closing_an_item_reaches_launches_without_a_session() {
    let fixture = Fixture::new();
    launch(&fixture, "waiting", "claude");
    let marked = fixture.ok(&["mark-closing", "--item", ITEM]);
    assert_eq!(marked["sessions"].as_array().unwrap().len(), 0);
    assert_eq!(marked["launches"][0]["launch_id"], "waiting");
    assert_eq!(marked["launches"][0]["state"], "closing");

    let again = fixture.ok(&["mark-closing", "--item", ITEM]);
    assert_eq!(again["launches"].as_array().unwrap().len(), 0);

    let ended = fixture.ok(&["end", "--launch-id", "waiting", "--reason", "merged"]);
    assert_eq!(ended["launch"]["state"], "ended");
    assert_eq!(ended["launch"]["end_reason"], "merged");

    let listed = fixture.ok(&["launches", "--item", ITEM]);
    assert_eq!(listed["launches"][0]["state"], "ended");
    assert_eq!(
        listed["launches"][0]["cwd"],
        "/work/core.worktrees/Feature-461"
    );
    assert_eq!(
        fixture.ok(&["launches"])["launches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn mark_closing_one_launch() {
    let fixture = Fixture::new();
    launch(&fixture, "one", "claude");
    let marked = fixture.ok(&["mark-closing", "--launch-id", "one"]);
    assert_eq!(marked["launches"][0]["state"], "closing");
    let shown = fixture.ok(&["show", "--launch-id", "one"]);
    assert_eq!(shown["launch"]["state"], "closing");
}

#[test]
fn launch_failures_have_stable_codes() {
    let fixture = Fixture::new();
    launch(&fixture, "dup", "claude");
    let cases: [(&[&str], i32, &str); 6] = [
        (
            &[
                "launch",
                "--launch-id",
                "dup",
                "--host",
                "claude",
                "--title",
                "t",
                "--cwd",
                "/",
                "--wrapper-pid",
                "1",
                "--wrapper-started-at",
                "2026-11-08T10:00:00Z",
            ],
            4,
            "conflict",
        ),
        (
            &[
                "register",
                "--session",
                "claude:x",
                "--cwd",
                "/",
                "--launch-id",
                "missing",
            ],
            3,
            "not-found",
        ),
        (
            &[
                "register",
                "--session",
                "codex:x",
                "--cwd",
                "/",
                "--launch-id",
                "dup",
            ],
            2,
            "invalid-input",
        ),
        (&["show", "--launch-id", "missing"], 3, "not-found"),
        (&["sessions", "--title", " "], 2, "invalid-input"),
        (&["mark-closing"], 2, "invalid-input"),
    ];
    for (args, code, name) in cases {
        let run = fixture.run(args);
        assert_eq!(run.code, code, "{args:?} -> {}", run.json);
        assert_eq!(run.json["error"]["code"], name, "{args:?}");
    }
    fixture.ok(&["end", "--launch-id", "dup"]);
    let twice = fixture.run(&["end", "--launch-id", "dup"]);
    assert_eq!(twice.code, 4, "{}", twice.json);
}

#[test]
fn a_launch_records_whether_its_tab_is_open() {
    let fixture = Fixture::new();
    assert_eq!(launch(&fixture, "tab", "claude")["open"], false);

    let opened = fixture.ok(&["set-open", "--launch-id", "tab", "--open", "true"]);
    assert_eq!(opened["launch"]["open"], true);
    assert_eq!(fixture.ok(&["launches"])["launches"][0]["open"], true);

    let ended = fixture.ok(&["end", "--launch-id", "tab", "--reason", "exited"]);
    assert_eq!(ended["launch"]["open"], true);
    let closed = fixture.run_input(r#"{"command": "set-open", "launch_id": "tab", "open": false}"#);
    assert_eq!(closed.code, 0, "{}", closed.json);
    let closed = closed.json["data"].clone();
    assert_eq!(closed["launch"]["open"], false);
    assert_eq!(closed["launch"]["state"], "ended");
    assert_eq!(
        fixture.ok(&["show", "--launch-id", "tab"])["launch"]["open"],
        false
    );

    let missing = fixture.run(&["set-open", "--launch-id", "missing", "--open", "true"]);
    assert_eq!(missing.code, 3, "{}", missing.json);
    assert_eq!(missing.json["error"]["code"], "not-found");
}

#[test]
fn closing_an_item_keeps_its_open_tabs_open() {
    let fixture = Fixture::new();
    launch(&fixture, "tab", "claude");
    fixture.ok(&["set-open", "--launch-id", "tab", "--open", "true"]);
    let marked = fixture.ok(&["mark-closing", "--item", ITEM]);
    assert_eq!(marked["launches"][0]["open"], true);
}
