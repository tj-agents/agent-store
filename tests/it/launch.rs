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
fn an_ended_owner_can_be_replaced_without_changing_its_history() {
    let fixture = Fixture::new();
    launch(&fixture, "replacement-launch", "claude");
    fixture.ok(&[
        "register",
        "--session",
        "claude:old",
        "--cwd",
        "/old",
        "--launch-id",
        "replacement-launch",
    ]);
    let ended = fixture.ok(&["end", "--session", "claude:old", "--reason", "clear"]);
    let replacement = fixture.run(&[
        "register",
        "--session",
        "claude:new",
        "--cwd",
        "/new",
        "--launch-id",
        "replacement-launch",
    ]);
    assert_eq!(replacement.code, 0, "{}", replacement.json);
    let bound = fixture.ok(&["show", "--launch-id", "replacement-launch"]);
    assert_eq!(bound["launch"]["session"], "claude:new");
    assert_eq!(bound["session"]["links"][0]["item"]["ref"], ITEM);
    assert_eq!(bound["session"]["terminal"]["id"], "tab-3");
    assert_eq!(fixture.ok(&["show", "claude:old"]), ended);
}

#[test]
fn a_different_native_session_cannot_replace_a_launch_owner() {
    for host in ["claude", "codex"] {
        let fixture = Fixture::new();
        launch(&fixture, "parent-launch", host);
        let parent = format!("{host}:parent");
        let child = format!("{host}:background");
        fixture.ok(&[
            "register",
            "--session",
            &parent,
            "--cwd",
            "/parent",
            "--launch-id",
            "parent-launch",
        ]);
        let before = fixture.ok(&["show", "--launch-id", "parent-launch"]);

        let refused = fixture.run(&[
            "register",
            "--session",
            &child,
            "--cwd",
            "/child",
            "--launch-id",
            "parent-launch",
        ]);

        assert_eq!(refused.code, 4, "{}", refused.json);
        assert_eq!(refused.json["error"]["code"], "conflict");
        assert_eq!(
            fixture.ok(&["show", "--launch-id", "parent-launch"]),
            before
        );
        assert_eq!(fixture.run(&["show", &child]).code, 3);
    }
}

#[test]
fn refused_launch_binding_preserves_an_existing_unbound_session() {
    let fixture = Fixture::new();
    launch(&fixture, "parent-launch", "codex");
    fixture.ok(&[
        "register",
        "--session",
        "codex:parent",
        "--cwd",
        "/parent",
        "--launch-id",
        "parent-launch",
    ]);
    let child = fixture.ok(&[
        "register",
        "--session",
        "codex:background",
        "--cwd",
        "/original-child",
        "--title",
        "child",
    ]);

    let refused = fixture.run(&[
        "register",
        "--session",
        "codex:background",
        "--cwd",
        "/changed-child",
        "--launch-id",
        "parent-launch",
    ]);

    assert_eq!(refused.code, 4, "{}", refused.json);
    assert_eq!(fixture.ok(&["show", "codex:background"]), child);
}

#[test]
fn concurrent_first_bindings_keep_exactly_one_native_owner() {
    for replace_ended in [false, true] {
        let fixture = Fixture::new();
        launch(&fixture, "shared-launch", "codex");
        if replace_ended {
            fixture.ok(&[
                "register",
                "--session",
                "codex:old",
                "--cwd",
                "/old",
                "--launch-id",
                "shared-launch",
            ]);
            fixture.ok(&["end", "--session", "codex:old", "--reason", "resume"]);
        }
        let gate = std::sync::Barrier::new(3);
        let (first, second) = std::thread::scope(|scope| {
            let first = scope.spawn(|| {
                gate.wait();
                fixture.run(&[
                    "register",
                    "--session",
                    "codex:first",
                    "--cwd",
                    "/first",
                    "--launch-id",
                    "shared-launch",
                ])
            });
            let second = scope.spawn(|| {
                gate.wait();
                fixture.run(&[
                    "register",
                    "--session",
                    "codex:second",
                    "--cwd",
                    "/second",
                    "--launch-id",
                    "shared-launch",
                ])
            });
            gate.wait();
            (first.join().unwrap(), second.join().unwrap())
        });
        let (winner, loser) = match (first.code, second.code) {
            (0, 4) => ("codex:first", "codex:second"),
            (4, 0) => ("codex:second", "codex:first"),
            codes => panic!(
                "expected one successful binding: {codes:?}; {}, {}",
                first.json, second.json
            ),
        };
        let bound = fixture.ok(&["show", "--launch-id", "shared-launch"]);
        assert_eq!(bound["launch"]["session"], winner);
        assert_eq!(bound["session"]["session"], winner);
        assert_eq!(fixture.run(&["show", loser]).code, 3);
    }
}

#[test]
fn an_ended_session_does_not_allow_replacing_a_closing_launch() {
    let fixture = Fixture::new();
    launch(&fixture, "closing-launch", "claude");
    fixture.ok(&[
        "register",
        "--session",
        "claude:old",
        "--cwd",
        "/old",
        "--launch-id",
        "closing-launch",
    ]);
    fixture.ok(&["end", "--session", "claude:old", "--reason", "clear"]);
    fixture.ok(&["mark-closing", "--launch-id", "closing-launch"]);
    let before = fixture.ok(&["show", "--launch-id", "closing-launch"]);
    let refused = fixture.run(&[
        "register",
        "--session",
        "claude:new",
        "--cwd",
        "/new",
        "--launch-id",
        "closing-launch",
    ]);
    assert_eq!(refused.code, 4, "{}", refused.json);
    assert_eq!(
        fixture.ok(&["show", "--launch-id", "closing-launch"]),
        before
    );
    assert_eq!(fixture.run(&["show", "claude:new"]).code, 3);
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
