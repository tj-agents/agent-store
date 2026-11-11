use std::path::Path;
use std::process::Stdio;

use crate::Fixture;

fn register(fixture: &Fixture, session: &str, launch: Option<&str>) {
    let mut arguments = vec!["register", "--session", session, "--cwd", "."];
    if let Some(launch) = launch {
        arguments.extend(["--launch-id", launch]);
    }
    fixture.ok(&arguments);
}

fn launch(fixture: &Fixture, id: &str, host: &str) {
    fixture.ok(&[
        "launch",
        "--launch-id",
        id,
        "--host",
        host,
        "--title",
        "ownership test",
        "--cwd",
        ".",
        "--wrapper-pid",
        "900",
        "--wrapper-started-at",
        "2026-11-11T00:00:00Z",
    ]);
}

fn claim(fixture: &Fixture, session: &str, launch: Option<&str>, path: &Path) -> serde_json::Value {
    let mut arguments = vec![
        "claim-worktree",
        "--session",
        session,
        "--path",
        path.to_str().unwrap(),
    ];
    if let Some(launch) = launch {
        arguments.extend(["--launch-id", launch]);
    }
    fixture.ok(&arguments)["worktree"].clone()
}

#[test]
fn worktree_ownership_retains_multiple_paths_across_registration() {
    let fixture = Fixture::new();
    register(&fixture, "codex:owner", None);
    let first = fixture.path("repository-a/worktree");
    let second = fixture.path("repository-b/worktree");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    let first_claim = claim(&fixture, "codex:owner", None, &first);
    register(&fixture, "codex:owner", None);
    let second_claim = claim(&fixture, "codex:owner", None, &second);
    assert_eq!(claim(&fixture, "codex:owner", None, &first), first_claim);
    let listed = fixture.ok(&["worktrees", "--session", "codex:owner"]);
    let owned = listed["worktrees"].as_array().unwrap();
    assert_eq!(owned.len(), 2);
    assert!(owned.contains(&first_claim));
    assert!(owned.contains(&second_claim));
    assert!(fixture.ok(&["show", "codex:owner"])["session"]["worktree"].is_null());
}

#[test]
fn worktree_ownership_rejects_competitors_until_explicit_release() {
    let fixture = Fixture::new();
    for session in ["claude:first", "codex:second"] {
        register(&fixture, session, None);
    }
    let path = fixture.path("owned");
    std::fs::create_dir(&path).unwrap();
    let owned = claim(&fixture, "claude:first", None, &path);
    let stored_path = owned["path"].as_str().unwrap();
    let alias = path.join(".");
    let refused = fixture.run(&[
        "claim-worktree",
        "--session",
        "codex:second",
        "--path",
        alias.to_str().unwrap(),
    ]);
    assert_eq!(refused.code, 4, "{}", refused.json);
    if cfg!(windows) {
        let casing = stored_path.to_uppercase();
        let refused = fixture.run(&[
            "claim-worktree",
            "--session",
            "codex:second",
            "--path",
            &casing,
        ]);
        assert_eq!(refused.code, 4, "{}", refused.json);
    }
    let refused = fixture.run(&[
        "release-worktree",
        "--session",
        "codex:second",
        "--path",
        stored_path,
    ]);
    assert_eq!(refused.code, 4, "{}", refused.json);
    fixture.ok(&["end", "--session", "claude:first"]);
    let ended = fixture.run(&[
        "claim-worktree",
        "--session",
        "claude:first",
        "--path",
        stored_path,
    ]);
    assert_eq!(ended.code, 4, "{}", ended.json);
    assert_eq!(
        fixture.ok(&["worktrees", "--session", "claude:first"])["worktrees"][0],
        owned
    );
    let refused = fixture.run(&[
        "claim-worktree",
        "--session",
        "codex:second",
        "--path",
        stored_path,
    ]);
    assert_eq!(refused.code, 4, "{}", refused.json);
    std::fs::remove_dir(&path).unwrap();
    fixture.ok(&[
        "release-worktree",
        "--session",
        "claude:first",
        "--path",
        stored_path,
    ]);
    assert_eq!(
        fixture.ok(&["worktrees", "--session", "claude:first"])["worktrees"],
        serde_json::json!([])
    );
    std::fs::create_dir(&path).unwrap();
    assert_eq!(
        claim(&fixture, "codex:second", None, &path)["session"],
        "codex:second"
    );
}

#[test]
fn worktree_ownership_requires_the_exact_active_launch_binding() {
    let fixture = Fixture::new();
    launch(&fixture, "first-launch", "claude");
    launch(&fixture, "other-launch", "claude");
    register(&fixture, "claude:owner", Some("first-launch"));
    register(&fixture, "claude:other", Some("other-launch"));
    let path = fixture.path("owned");
    std::fs::create_dir(&path).unwrap();
    for arguments in [
        vec![
            "claim-worktree",
            "--session",
            "claude:owner",
            "--path",
            path.to_str().unwrap(),
        ],
        vec![
            "claim-worktree",
            "--session",
            "claude:owner",
            "--launch-id",
            "other-launch",
            "--path",
            path.to_str().unwrap(),
        ],
    ] {
        let refused = fixture.run(&arguments);
        assert_eq!(refused.code, 4, "{}", refused.json);
    }
    let owned = claim(&fixture, "claude:owner", Some("first-launch"), &path);
    assert_eq!(
        fixture.ok(&["worktrees", "--session", "claude:owner"])["worktrees"],
        serde_json::json!([])
    );
    assert_eq!(
        fixture.ok(&[
            "worktrees",
            "--session",
            "claude:owner",
            "--launch-id",
            "first-launch"
        ])["worktrees"][0],
        owned
    );
    fixture.ok(&["mark-closing", "--launch-id", "first-launch"]);
    let next_path = fixture.path("next");
    std::fs::create_dir(&next_path).unwrap();
    let refused = fixture.run(&[
        "claim-worktree",
        "--session",
        "claude:owner",
        "--launch-id",
        "first-launch",
        "--path",
        next_path.to_str().unwrap(),
    ]);
    assert_eq!(refused.code, 4, "{}", refused.json);
    fixture.ok(&["end", "--session", "claude:owner"]);
    let stored_path = owned["path"].as_str().unwrap();
    let refused = fixture.run(&[
        "release-worktree",
        "--session",
        "claude:owner",
        "--path",
        stored_path,
    ]);
    assert_eq!(refused.code, 4, "{}", refused.json);
    fixture.ok(&[
        "release-worktree",
        "--session",
        "claude:owner",
        "--launch-id",
        "first-launch",
        "--path",
        stored_path,
    ]);
}

#[test]
fn worktree_ownership_serializes_competing_processes() {
    let fixture = Fixture::new();
    for session in ["claude:first", "codex:second"] {
        register(&fixture, session, None);
    }
    let path = fixture.path("owned");
    std::fs::create_dir(&path).unwrap();
    let first = fixture
        .command(&[
            "claim-worktree",
            "--session",
            "claude:first",
            "--path",
            path.to_str().unwrap(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let second = fixture
        .command(&[
            "claim-worktree",
            "--session",
            "codex:second",
            "--path",
            path.to_str().unwrap(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut codes = [
        first.wait_with_output().unwrap().status.code().unwrap(),
        second.wait_with_output().unwrap().status.code().unwrap(),
    ];
    codes.sort_unstable();
    assert_eq!(codes, [0, 4]);
    let first = fixture.ok(&["worktrees", "--session", "claude:first"]);
    let second = fixture.ok(&["worktrees", "--session", "codex:second"]);
    assert_eq!(
        first["worktrees"].as_array().unwrap().len()
            + second["worktrees"].as_array().unwrap().len(),
        1
    );
}

#[test]
fn worktree_ownership_json_contract_and_invalid_input() {
    let fixture = Fixture::new();
    register(&fixture, "codex:owner", None);
    let path = fixture.path("owned");
    std::fs::create_dir(&path).unwrap();
    let request =
        serde_json::json!({"command": "claim-worktree", "session": "codex:owner", "path": path});
    let claimed = fixture.run_input(&request.to_string());
    assert_eq!(claimed.code, 0, "{}", claimed.json);
    let stored_path = claimed.json["data"]["worktree"]["path"].as_str().unwrap();
    assert_eq!(
        stored_path,
        std::fs::canonicalize(&path).unwrap().to_str().unwrap()
    );
    assert!(claimed.json["data"]["worktree"]["launch_id"].is_null());
    let missing = fixture.run(&[
        "claim-worktree",
        "--session",
        "codex:missing",
        "--path",
        stored_path,
    ]);
    assert_eq!(missing.code, 3, "{}", missing.json);
    let absent_path = fixture.path("absent");
    let missing = fixture.run(&[
        "claim-worktree",
        "--session",
        "codex:owner",
        "--path",
        absent_path.to_str().unwrap(),
    ]);
    assert_eq!(missing.code, 2, "{}", missing.json);
    let relative = fixture.run(&[
        "release-worktree",
        "--session",
        "codex:owner",
        "--path",
        "owned",
    ]);
    assert_eq!(relative.code, 2, "{}", relative.json);
    let request = serde_json::json!({"command": "release-worktree", "session": "codex:owner", "path": stored_path});
    assert_eq!(fixture.run_input(&request.to_string()).code, 0);
    assert_eq!(
        fixture.ok(&["worktrees", "--session", "codex:owner"])["worktrees"],
        serde_json::json!([])
    );
}
