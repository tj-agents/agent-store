use std::process::Stdio;

use agent_store::domain::session::{Handle, LaunchId, Resume, Wrapper, service};
use agent_store::sqlite::Store;
use chrono::{DateTime, Utc};
use sea_orm::sqlx::{Connection, SqliteConnection, query, sqlite::SqliteConnectOptions};
use serde_json::{Value, json};

use crate::Fixture;

const OLD_TIME: &str = "2026-11-08T10:00:00Z";
const NEW_TIME: &str = "2026-11-09T11:00:00Z";
const OLD_ITEM: &str = "github:github.com/acme/app:1";
const NEW_ITEM: &str = "github:github.com/acme/app:2";

fn input(fixture: &Fixture, request: &Value) -> Value {
    let run = fixture.run_input(&request.to_string());
    assert_eq!(run.code, 0, "{}", run.json);
    run.json["data"].clone()
}

fn launch(fixture: &Fixture, id: &str, host: &str) {
    let source = id == "source";
    input(
        fixture,
        &json!({
            "command": "launch", "launch_id": id, "host": host, "title": id,
            "cwd": fixture.path(&format!("{id} cwd")),
            "worktree": fixture.path(&format!("{id} worktree")),
            "wrapper_pid": if source { 101 } else { 202 },
            "wrapper_started_at": if source { OLD_TIME } else { NEW_TIME },
            "terminal_kind": "tmux", "terminal_id": id,
            "parent": if source { Some("claude:parent") } else { None },
            "item": if source { OLD_ITEM } else { NEW_ITEM },
        }),
    );
}

fn register(fixture: &Fixture, session: &str, launch: Option<&str>) {
    let mut arguments = vec!["register", "--session", session, "--cwd", "original"];
    if let Some(launch) = launch {
        arguments.extend(["--launch-id", launch]);
    }
    fixture.ok(&arguments);
}

fn claim(fixture: &Fixture, session: &str, launch: Option<&str>, name: &str) -> Value {
    let path = fixture.path(name);
    std::fs::create_dir_all(&path).unwrap();
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

fn claims(fixture: &Fixture, session: &str, launch: Option<&str>) -> Value {
    let mut arguments = vec!["worktrees", "--session", session];
    if let Some(launch) = launch {
        arguments.extend(["--launch-id", launch]);
    }
    fixture.ok(&arguments)["worktrees"].clone()
}

fn request(session: &str, destination: &str) -> Value {
    json!({
        "command": "resume", "session": session,
        "from_launch_id": "source", "launch_id": destination,
        "from_wrapper_pid": 101, "from_wrapper_started_at": OLD_TIME,
        "wrapper_pid": 202, "wrapper_started_at": NEW_TIME,
    })
}

fn arguments<'a>(session: &'a str, destination: &'a str) -> Vec<&'a str> {
    vec![
        "resume",
        "--session",
        session,
        "--from-launch-id",
        "source",
        "--launch-id",
        destination,
        "--from-wrapper-pid",
        "101",
        "--from-wrapper-started-at",
        OLD_TIME,
        "--wrapper-pid",
        "202",
        "--wrapper-started-at",
        NEW_TIME,
    ]
}

fn seed() -> Fixture {
    let fixture = Fixture::new();
    launch(&fixture, "source", "codex");
    launch(&fixture, "destination", "codex");
    register(&fixture, "codex:owner", Some("source"));
    claim(&fixture, "codex:owner", Some("source"), "first");
    claim(&fixture, "codex:owner", Some("source"), "second");
    fixture
}

fn snapshot(fixture: &Fixture) -> Value {
    json!({
        "sessions": fixture.ok(&["sessions"]),
        "launches": fixture.ok(&["launches"]),
        "source": claims(fixture, "codex:owner", Some("source")),
        "destination": claims(fixture, "codex:owner", Some("destination")),
    })
}

#[test]
fn resume_moves_only_the_exact_source_claims_and_preserves_history_and_links() {
    for host in ["claude", "codex"] {
        let fixture = Fixture::new();
        let handle = format!("{host}:owner");
        let foreign = format!("{host}:foreign");
        for id in ["source", "destination", "historical"] {
            launch(&fixture, id, host);
        }
        register(&fixture, &handle, None);
        let unmanaged = claim(&fixture, &handle, None, "unmanaged");
        register(&fixture, &handle, Some("historical"));
        let historical = claim(&fixture, &handle, Some("historical"), "historical claim");
        register(&fixture, &foreign, Some("source"));
        let foreign_claim = claim(&fixture, &foreign, Some("source"), "foreign claim");
        fixture.ok(&["end", "--session", &foreign]);
        register(&fixture, &handle, Some("source"));
        fixture.ok(&[
            "link",
            "--session",
            &handle,
            "--item",
            NEW_ITEM,
            "--role",
            "reviewer",
        ]);
        for index in 0..5 {
            claim(
                &fixture,
                &handle,
                Some("source"),
                &format!("repo {index}/worktree"),
            );
        }
        let mut expected = claims(&fixture, &handle, Some("source"));
        for row in expected.as_array_mut().unwrap() {
            row["launch_id"] = json!("destination");
        }
        // Transfer keeps durable claims even when a directory no longer exists.
        std::fs::remove_dir(fixture.path("repo 0/worktree")).unwrap();
        let source_before = fixture.ok(&["show", "--launch-id", "source"])["launch"].clone();
        let session_before = fixture.ok(&["show", &handle])["session"].clone();
        let foreign_before = fixture.ok(&["show", &foreign]);
        let destination = fixture.ok(&["show", "--launch-id", "destination"])["launch"].clone();

        let resumed = fixture.ok(&arguments(&handle, "destination"))["session"].clone();
        assert_eq!(resumed["session"], handle);
        assert_eq!(resumed["launch_id"], "destination");
        assert_eq!(resumed["state"], "active");
        for field in ["cwd", "worktree", "terminal", "wrapper", "title", "parent"] {
            assert_eq!(resumed[field], destination[field], "{field}");
        }
        assert_eq!(resumed["started_at"], session_before["started_at"]);
        assert_eq!(resumed["links"], session_before["links"]);
        assert!(
            resumed["parent"].is_null(),
            "destination clears the previous parent"
        );
        assert_eq!(claims(&fixture, &handle, Some("destination")), expected);
        assert_eq!(claims(&fixture, &handle, Some("source")), json!([]));
        assert_eq!(claims(&fixture, &handle, None), json!([unmanaged]));
        assert_eq!(
            claims(&fixture, &handle, Some("historical")),
            json!([historical])
        );
        assert_eq!(
            claims(&fixture, &foreign, Some("source")),
            json!([foreign_claim])
        );
        assert_eq!(fixture.ok(&["show", &foreign]), foreign_before);
        assert_eq!(
            fixture.ok(&["show", "--launch-id", "source"])["launch"],
            source_before
        );
        assert_eq!(
            fixture.ok(&["show", "--launch-id", "destination"])["launch"]["session"],
            handle
        );
        assert_eq!(
            input(&fixture, &request(&handle, "destination"))["session"],
            resumed
        );
        assert_eq!(claims(&fixture, &handle, Some("destination")), expected);
    }
}

#[tokio::test]
async fn library_resume_accepts_ended_predecessors_and_adds_the_destination_item() {
    let fixture = seed();
    fixture.ok(&["end", "--launch-id", "source", "--reason", "exited"]);
    fixture.ok(&["end", "--session", "codex:owner"]);
    let source = fixture.ok(&["show", "--launch-id", "source"])["launch"].clone();
    let store = Store::open(&fixture.database()).await.unwrap();
    let request = Resume {
        handle: Handle::parse("codex:owner").unwrap(),
        source: LaunchId::parse("source").unwrap(),
        destination: LaunchId::parse("destination").unwrap(),
        source_wrapper: Wrapper {
            pid: 101,
            started_at: OLD_TIME.parse().unwrap(),
        },
        destination_wrapper: Wrapper {
            pid: 202,
            started_at: NEW_TIME.parse().unwrap(),
        },
    };
    let now: DateTime<Utc> = "2026-11-09T12:00:00Z".parse().unwrap();
    let resumed = service::resume(&store, &request, now).await.unwrap();
    assert_eq!(resumed.launch(), Some(&request.destination));
    assert_eq!(resumed.links().len(), 2);
    let added = resumed
        .links()
        .iter()
        .find(|link| link.item.to_string() == NEW_ITEM)
        .unwrap();
    assert_eq!(added.role.as_str(), "owner");
    assert_eq!(added.linked_at, now);
    assert_eq!(
        claims(&fixture, "codex:owner", Some("destination"))
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        fixture.ok(&["show", "--launch-id", "source"])["launch"],
        source
    );
    // Retrying later cannot change the original link timestamp.
    service::resume(&store, &request, now + chrono::Duration::hours(1))
        .await
        .unwrap();
    assert_eq!(
        service::show(&store, &request.handle)
            .await
            .unwrap()
            .links()
            .iter()
            .find(|link| link.item.to_string() == NEW_ITEM)
            .unwrap()
            .linked_at,
        now
    );
}

#[test]
fn resume_rejects_changed_expected_identities_without_writes() {
    for (field, value, code) in [
        ("from_wrapper_pid", json!(999), 4),
        ("from_wrapper_started_at", json!(NEW_TIME), 4),
        ("wrapper_pid", json!(999), 4),
        ("wrapper_started_at", json!(OLD_TIME), 4),
        ("session", json!("codex:missing"), 3),
        ("from_launch_id", json!("missing"), 3),
        ("launch_id", json!("missing"), 3),
        ("launch_id", json!("source"), 2),
        ("wrapper_started_at", json!("invalid"), 2),
        ("unexpected", json!(true), 2),
    ] {
        let fixture = seed();
        let before = snapshot(&fixture);
        let mut resume = request("codex:owner", "destination");
        resume[field] = value;
        let run = fixture.run_input(&resume.to_string());
        assert_eq!(run.code, code, "{field}: {}", run.json);
        assert_eq!(snapshot(&fixture), before, "{field}");
    }
}

#[test]
fn resume_without_claims_adopts_absent_destination_metadata_and_retries() {
    let fixture = Fixture::new();
    launch(&fixture, "source", "codex");
    register(&fixture, "codex:owner", Some("source"));
    input(
        &fixture,
        &json!({
            "command": "launch", "launch_id": "destination", "host": "codex",
            "title": "continued", "cwd": fixture.path("destination"),
            "wrapper_pid": 202, "wrapper_started_at": NEW_TIME,
        }),
    );
    let before = fixture.ok(&["show", "codex:owner"])["session"].clone();
    let resumed = input(&fixture, &request("codex:owner", "destination"))["session"].clone();
    for field in ["worktree", "terminal", "parent"] {
        assert!(resumed[field].is_null(), "{field}");
    }
    assert_eq!(resumed["links"], before["links"]);
    assert_eq!(resumed["title"], "continued");
    assert_eq!(
        claims(&fixture, "codex:owner", Some("destination")),
        json!([])
    );
    assert_eq!(
        input(&fixture, &request("codex:owner", "destination"))["session"],
        resumed
    );
}

#[test]
fn resume_rejects_closing_foreign_and_changed_bindings_without_writes() {
    for case in [
        "source-closing",
        "session-closing",
        "destination-closing",
        "destination-ended",
        "foreign-destination",
        "foreign-source",
        "changed-session-launch",
        "changed-session-wrapper",
        "wrong-destination-host",
        "partial-rebind",
    ] {
        let fixture = seed();
        let mut resume = request("codex:owner", "destination");
        match case {
            "source-closing" => {
                fixture.ok(&["mark-closing", "--launch-id", "source"]);
            }
            "session-closing" => {
                fixture.ok(&["mark-closing", "--session", "codex:owner"]);
            }
            "destination-closing" => {
                fixture.ok(&["mark-closing", "--launch-id", "destination"]);
            }
            "destination-ended" => {
                fixture.ok(&["end", "--launch-id", "destination"]);
            }
            "foreign-destination" => {
                register(&fixture, "codex:foreign", Some("destination"));
            }
            "foreign-source" => {
                fixture.ok(&["end", "--session", "codex:owner"]);
                register(&fixture, "codex:foreign", Some("source"));
            }
            "changed-session-launch" => {
                launch(&fixture, "concurrent", "codex");
                register(&fixture, "codex:owner", Some("concurrent"));
            }
            "changed-session-wrapper" => {
                fixture.ok(&[
                    "register",
                    "--session",
                    "codex:owner",
                    "--cwd",
                    "original",
                    "--launch-id",
                    "source",
                    "--wrapper-pid",
                    "999",
                    "--wrapper-started-at",
                    OLD_TIME,
                ]);
            }
            "wrong-destination-host" => {
                launch(&fixture, "other-host", "claude");
                resume["launch_id"] = json!("other-host");
            }
            "partial-rebind" => {
                register(&fixture, "codex:owner", Some("destination"));
            }
            _ => unreachable!(),
        }
        let before = snapshot(&fixture);
        let run = fixture.run_input(&resume.to_string());
        assert_eq!(run.code, 4, "{case}: {}", run.json);
        assert_eq!(run.json["error"]["code"], "conflict");
        assert_eq!(snapshot(&fixture), before, "{case}");
    }
}

#[test]
fn retry_requires_the_exact_active_destination_and_no_source_claims() {
    for case in [
        "session-ended",
        "session-closing",
        "source-closing",
        "destination-ended",
        "destination-closing",
        "session-wrapper",
        "session-launch",
    ] {
        let fixture = seed();
        input(&fixture, &request("codex:owner", "destination"));
        match case {
            "session-ended" => {
                fixture.ok(&["end", "--session", "codex:owner"]);
            }
            "session-closing" => {
                fixture.ok(&["mark-closing", "--session", "codex:owner"]);
            }
            "source-closing" => {
                fixture.ok(&["mark-closing", "--launch-id", "source"]);
            }
            "destination-ended" => {
                fixture.ok(&["end", "--launch-id", "destination"]);
            }
            "destination-closing" => {
                fixture.ok(&["mark-closing", "--launch-id", "destination"]);
            }
            "session-wrapper" => {
                fixture.ok(&[
                    "register",
                    "--session",
                    "codex:owner",
                    "--cwd",
                    "elsewhere",
                    "--launch-id",
                    "destination",
                    "--wrapper-pid",
                    "999",
                    "--wrapper-started-at",
                    NEW_TIME,
                ]);
            }
            "session-launch" => {
                launch(&fixture, "concurrent", "codex");
                register(&fixture, "codex:owner", Some("concurrent"));
            }
            _ => unreachable!(),
        }
        let before = snapshot(&fixture);
        let run = fixture.run_input(&request("codex:owner", "destination").to_string());
        assert_eq!(run.code, 4, "{case}: {}", run.json);
        assert_eq!(snapshot(&fixture), before, "{case}");
    }
}

#[test]
fn competing_resume_processes_admit_exactly_one_destination() {
    let fixture = seed();
    launch(&fixture, "competitor", "codex");
    let children: Vec<_> = ["destination", "competitor"]
        .into_iter()
        .map(|destination| {
            fixture
                .command(&arguments("codex:owner", destination))
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    let mut codes = Vec::new();
    for child in children {
        let output = child.wait_with_output().unwrap();
        let reply: Value = serde_json::from_slice(&output.stdout).unwrap();
        let code = output.status.code().unwrap();
        assert!(matches!(code, 0 | 4), "{reply}");
        codes.push(code);
    }
    codes.sort_unstable();
    assert_eq!(codes, [0, 4]);
    let session = fixture.ok(&["show", "codex:owner"])["session"].clone();
    let winner = session["launch_id"].as_str().unwrap();
    let loser = if winner == "destination" {
        "competitor"
    } else {
        "destination"
    };
    assert!(fixture.ok(&["show", "--launch-id", loser])["launch"]["session"].is_null());
    assert_eq!(
        claims(&fixture, "codex:owner", Some(winner))
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(claims(&fixture, "codex:owner", Some("source")), json!([]));
    assert_eq!(claims(&fixture, "codex:owner", Some(loser)), json!([]));
}

#[tokio::test]
async fn claim_update_failure_rolls_back_session_destination_links_and_all_claims() {
    let fixture = seed();
    let before = snapshot(&fixture);
    let options = SqliteConnectOptions::new().filename(fixture.database());
    let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
    query(
        "CREATE TRIGGER refuse_transfer BEFORE UPDATE OF launch_id ON worktree_ownerships
        WHEN NEW.launch_id = 'destination'
        BEGIN SELECT RAISE(ABORT, 'injected claim transfer failure'); END",
    )
    .execute(&mut connection)
    .await
    .unwrap();
    let run = fixture.run_input(&request("codex:owner", "destination").to_string());
    assert_eq!(run.code, 5, "{}", run.json);
    assert_eq!(run.json["error"]["code"], "storage");
    assert_eq!(snapshot(&fixture), before);
    query("DROP TRIGGER refuse_transfer")
        .execute(&mut connection)
        .await
        .unwrap();
    input(&fixture, &request("codex:owner", "destination"));
    assert_eq!(
        claims(&fixture, "codex:owner", Some("destination"))
            .as_array()
            .unwrap()
            .len(),
        2
    );
}
