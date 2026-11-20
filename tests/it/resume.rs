use std::process::Stdio;

use agent_store::domain::session::{Handle, LaunchId, Resume, Wrapper, service};
use agent_store::sqlite::Store;
use chrono::{DateTime, Utc};
use sea_orm::sqlx::{
    Connection, SqliteConnection, query, query_scalar, sqlite::SqliteConnectOptions,
};
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
fn guarded_registration_retries_same_owner_and_refuses_a_different_launch() {
    let fixture = seed();
    let before = snapshot(&fixture);
    let register = |launch| {
        fixture.run_input(
            &json!({
                "command": "register", "session": "codex:owner", "cwd": "original",
                "launch_id": launch, "require_unowned": true,
            })
            .to_string(),
        )
    };
    let conflict = register("destination");
    assert_eq!(
        conflict.json["error"]["code"], "conflict",
        "{}",
        conflict.json
    );
    assert_eq!(snapshot(&fixture), before);
    assert_eq!(register("source").code, 0);
    assert_eq!(
        claims(&fixture, "codex:owner", Some("source")),
        before["source"]
    );
}

#[test]
fn competing_guarded_registrations_bind_exactly_one_launch() {
    let fixture = Fixture::new();
    let handle = "codex:new";
    let register = |id| {
        fixture
            .command(&[
                "register",
                "--session",
                handle,
                "--cwd",
                "original",
                "--launch-id",
                id,
                "--require-unowned",
            ])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap()
    };
    launch(&fixture, "source", "codex");
    launch(&fixture, "destination", "codex");
    let first = register("source");
    let second = register("destination");
    let first = first.wait_with_output().unwrap();
    let second = second.wait_with_output().unwrap();
    let mut codes = [first.status.code().unwrap(), second.status.code().unwrap()];
    codes.sort_unstable();
    assert_eq!(codes, [0, 4]);
    let (winner, loser) = if first.status.success() {
        ("source", "destination")
    } else {
        ("destination", "source")
    };
    assert_eq!(
        fixture.ok(&["show", handle])["session"]["launch_id"],
        winner
    );
    assert_eq!(
        fixture.ok(&["show", "--launch-id", winner])["launch"]["session"],
        handle
    );
    assert!(fixture.ok(&["show", "--launch-id", loser])["launch"]["session"].is_null());
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
fn ordinary_registration_without_claims_cannot_masquerade_as_a_resume_retry() {
    for host in ["claude", "codex"] {
        for difference in ["none", "cwd", "parent"] {
            let fixture = Fixture::new();
            let handle = format!("{host}:owner");
            launch(&fixture, "source", host);
            register(&fixture, &handle, Some("source"));
            let destination = input(
                &fixture,
                &json!({
                    "command": "launch", "launch_id": "destination", "host": host,
                    "title": "continued", "cwd": fixture.path("destination"),
                    "wrapper_pid": 202, "wrapper_started_at": NEW_TIME,
                    "parent": if difference == "parent" { None } else { Some("claude:parent") },
                }),
            )["launch"]
                .clone();
            let registered = input(
                &fixture,
                &json!({
                    "command": "register", "session": handle, "launch_id": "destination",
                    "cwd": if difference == "cwd" {
                        json!(fixture.path("elsewhere"))
                    } else {
                        destination["cwd"].clone()
                    },
                }),
            )["session"]
                .clone();
            for field in ["cwd", "worktree", "terminal", "wrapper", "title", "parent"] {
                if field == difference {
                    assert_ne!(registered[field], destination[field], "{host}/{difference}");
                } else {
                    assert_eq!(
                        registered[field], destination[field],
                        "{host}/{difference}/{field}"
                    );
                }
            }
            assert_eq!(claims(&fixture, &handle, Some("source")), json!([]));
            let before = snapshot(&fixture);
            let run = fixture.run_input(&request(&handle, "destination").to_string());
            assert_eq!(run.code, 4, "{host}/{difference}: {}", run.json);
            assert_eq!(run.json["error"]["code"], "conflict");
            assert_eq!(snapshot(&fixture), before, "{host}/{difference}");
        }
    }
}

#[test]
fn ordinary_rebinding_invalidates_admission_without_changing_claims() {
    for host in ["claude", "codex"] {
        for with_claims in [false, true] {
            let fixture = Fixture::new();
            let handle = format!("{host}:owner");
            launch(&fixture, "source", host);
            launch(&fixture, "destination", host);
            register(&fixture, &handle, Some("source"));
            if with_claims {
                claim(&fixture, &handle, Some("source"), "owned");
            }
            input(&fixture, &request(&handle, "destination"));
            let owned = claims(&fixture, &handle, Some("destination"));
            register(&fixture, &handle, Some("source"));
            register(&fixture, &handle, Some("destination"));
            let before = snapshot(&fixture);
            let run = fixture.run_input(&request(&handle, "destination").to_string());
            assert_eq!(run.code, 4, "{host}/{with_claims}: {}", run.json);
            assert_eq!(run.json["error"]["code"], "conflict");
            assert_eq!(snapshot(&fixture), before);
            assert_eq!(claims(&fixture, &handle, Some("destination")), owned);
            assert_eq!(claims(&fixture, &handle, Some("source")), json!([]));
        }
    }
}

#[test]
fn same_binding_registration_and_metadata_updates_preserve_resume_retries() {
    for managed in [false, true] {
        let fixture = seed();
        input(&fixture, &request("codex:owner", "destination"));
        input(
            &fixture,
            &json!({
                "command": "register", "session": "codex:owner",
                "launch_id": if managed { Some("destination") } else { None },
                "cwd": fixture.path("updated cwd"), "worktree": fixture.path("updated worktree"),
                "wrapper_pid": 202, "wrapper_started_at": NEW_TIME,
                "title": "updated", "parent": "claude:updated-parent",
                "terminal_kind": "tmux", "terminal_id": "updated",
            }),
        );
        fixture.ok(&[
            "link",
            "--session",
            "codex:owner",
            "--item",
            NEW_ITEM,
            "--role",
            "reviewer",
        ]);
        // The source is historical; its wrapper ending does not replace the current binding.
        fixture.ok(&["end", "--launch-id", "source"]);
        let before = snapshot(&fixture);
        let resumed = input(&fixture, &request("codex:owner", "destination"));
        assert_eq!(resumed, fixture.ok(&["show", "codex:owner"]));
        assert_eq!(snapshot(&fixture), before);
    }
}

#[test]
fn restored_wrapper_or_reactivated_session_cannot_reuse_an_old_admission() {
    for interruption in ["pid", "started-at", "no-wrapper", "ended", "closing"] {
        let fixture = seed();
        input(&fixture, &request("codex:owner", "destination"));
        match interruption {
            "ended" => {
                fixture.ok(&["end", "--session", "codex:owner"]);
            }
            "closing" => {
                fixture.ok(&["mark-closing", "--session", "codex:owner"]);
                fixture.ok(&["end", "--session", "codex:owner"]);
            }
            _ => {
                // Without --launch-id this exercises the ordinary library update path.
                let mut registration = json!({
                    "command": "register", "session": "codex:owner", "cwd": fixture.path("other"),
                });
                if interruption != "no-wrapper" {
                    registration["wrapper_pid"] =
                        json!(if interruption == "pid" { 999 } else { 202 });
                    registration["wrapper_started_at"] = json!(if interruption == "started-at" {
                        OLD_TIME
                    } else {
                        NEW_TIME
                    });
                }
                input(&fixture, &registration);
            }
        }
        register(&fixture, "codex:owner", Some("destination"));
        let before = snapshot(&fixture);
        let run = fixture.run_input(&request("codex:owner", "destination").to_string());
        assert_eq!(run.code, 4, "{interruption}: {}", run.json);
        assert_eq!(snapshot(&fixture), before);
    }
}

#[test]
fn a_later_guarded_resume_replaces_only_its_sessions_admission() {
    let fixture = seed();
    input(&fixture, &request("codex:owner", "destination"));
    for id in ["foreign-source", "foreign-destination", "later"] {
        launch(&fixture, id, "codex");
    }
    register(&fixture, "codex:foreign", Some("foreign-source"));
    let mut foreign = request("codex:foreign", "foreign-destination");
    foreign["from_launch_id"] = json!("foreign-source");
    foreign["from_wrapper_pid"] = json!(202);
    foreign["from_wrapper_started_at"] = json!(NEW_TIME);
    let foreign_admitted = input(&fixture, &foreign);
    let mut later = request("codex:owner", "later");
    later["from_launch_id"] = json!("destination");
    later["from_wrapper_pid"] = json!(202);
    later["from_wrapper_started_at"] = json!(NEW_TIME);
    let admitted = input(&fixture, &later);
    assert_eq!(input(&fixture, &later), admitted);
    register(&fixture, "codex:owner", Some("destination"));
    let before = snapshot(&fixture);
    let run = fixture.run_input(&request("codex:owner", "destination").to_string());
    assert_eq!(run.code, 4, "{}", run.json);
    assert_eq!(snapshot(&fixture), before);
    assert_eq!(input(&fixture, &foreign), foreign_admitted);
    assert_eq!(
        claims(&fixture, "codex:owner", Some("later"))
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn failed_binding_replacement_restores_the_admission_receipt() {
    let fixture = seed();
    let admitted = input(&fixture, &request("codex:owner", "destination"));
    let before = snapshot(&fixture);
    let options = SqliteConnectOptions::new().filename(fixture.database());
    let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
    query(
        "CREATE TRIGGER refuse_rebinding BEFORE UPDATE OF session_id ON launches
        WHEN NEW.launch_id = 'source'
        BEGIN SELECT RAISE(ABORT, 'injected binding failure'); END",
    )
    .execute(&mut connection)
    .await
    .unwrap();
    let run = fixture.run(&[
        "register",
        "--session",
        "codex:owner",
        "--cwd",
        "changed",
        "--launch-id",
        "source",
    ]);
    assert_eq!(run.code, 5, "{}", run.json);
    assert_eq!(snapshot(&fixture), before);
    assert_eq!(receipt_count(&mut connection).await, 1);
    assert_eq!(
        input(&fixture, &request("codex:owner", "destination")),
        admitted
    );
}

#[test]
fn resume_receipts_apply_only_to_the_exact_session_source_and_destination() {
    for difference in ["session", "source", "destination"] {
        let fixture = seed();
        input(&fixture, &request("codex:owner", "destination"));
        let mut retry = request("codex:owner", "destination");
        match difference {
            "session" => {
                register(&fixture, "codex:owner", Some("source"));
                fixture.ok(&["end", "--session", "codex:owner"]);
                register(&fixture, "codex:foreign", Some("source"));
                register(&fixture, "codex:owner", Some("destination"));
                fixture.ok(&["end", "--session", "codex:owner"]);
                register(&fixture, "codex:foreign", Some("destination"));
                retry["session"] = json!("codex:foreign");
            }
            "source" => {
                input(
                    &fixture,
                    &json!({
                        "command": "launch", "launch_id": "other-source", "host": "codex",
                        "title": "other", "cwd": fixture.path("other"),
                        "wrapper_pid": 101, "wrapper_started_at": OLD_TIME,
                    }),
                );
                register(&fixture, "codex:owner", Some("other-source"));
                register(&fixture, "codex:owner", Some("destination"));
                retry["from_launch_id"] = json!("other-source");
            }
            "destination" => {
                launch(&fixture, "other-destination", "codex");
                register(&fixture, "codex:owner", Some("other-destination"));
                retry["launch_id"] = json!("other-destination");
            }
            _ => unreachable!(),
        }
        let before = snapshot(&fixture);
        let run = fixture.run_input(&retry.to_string());
        assert_eq!(run.code, 4, "{difference}: {}", run.json);
        assert_eq!(snapshot(&fixture), before, "{difference}");
    }
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
    assert_eq!(receipt_count(&mut connection).await, 0);
    query("DROP TRIGGER refuse_transfer")
        .execute(&mut connection)
        .await
        .unwrap();
    input(&fixture, &request("codex:owner", "destination"));
    assert_eq!(receipt_count(&mut connection).await, 1);
    assert_eq!(
        claims(&fixture, "codex:owner", Some("destination"))
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

async fn receipt_count(connection: &mut SqliteConnection) -> i64 {
    query_scalar("SELECT COUNT(*) FROM session_resumes")
        .fetch_one(connection)
        .await
        .unwrap()
}

#[tokio::test]
async fn receipt_migration_preserves_existing_records_without_inventing_admissions() {
    let fixture = seed();
    let before = snapshot(&fixture);
    let options = SqliteConnectOptions::new()
        .filename(fixture.database())
        .foreign_keys(true);
    let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
    // Removing only the new table and migration entry restores the prior, populated schema.
    query("DROP TABLE session_resumes")
        .execute(&mut connection)
        .await
        .unwrap();
    query("DELETE FROM seaql_migrations WHERE version = 'm20261118_000001_record_resumes'")
        .execute(&mut connection)
        .await
        .unwrap();
    assert_eq!(snapshot(&fixture), before);
    assert_eq!(receipt_count(&mut connection).await, 0);
    let resumed = input(&fixture, &request("codex:owner", "destination"));
    assert_eq!(receipt_count(&mut connection).await, 1);
    assert_eq!(
        input(&fixture, &request("codex:owner", "destination")),
        resumed
    );
    assert_eq!(receipt_count(&mut connection).await, 1);
    let owner: i64 = query_scalar("SELECT id FROM sessions WHERE native_id = 'owner'")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    for (session, source, destination) in [
        (owner, "source", "destination"),
        (-1, "source", "destination"),
        (owner, "missing", "destination"),
        (owner, "source", "missing"),
    ] {
        assert!(
            query("INSERT INTO session_resumes VALUES (?, ?, ?)")
                .bind(session)
                .bind(source)
                .bind(destination)
                .execute(&mut connection)
                .await
                .is_err(),
            "receipt must be unique and reference existing records"
        );
    }
    assert_eq!(receipt_count(&mut connection).await, 1);
}

#[tokio::test]
async fn failed_receipt_migration_leaves_existing_data_and_migration_history_unchanged() {
    let fixture = seed();
    let options = SqliteConnectOptions::new().filename(fixture.database());
    let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
    // The pre-existing table makes CREATE TABLE fail before admission can begin.
    query("DELETE FROM seaql_migrations WHERE version = 'm20261118_000001_record_resumes'")
        .execute(&mut connection)
        .await
        .unwrap();
    let run = fixture.run_input(&request("codex:owner", "destination").to_string());
    assert_eq!(run.code, 5, "{}", run.json);
    let recorded: i64 = query_scalar(
        "SELECT COUNT(*) FROM seaql_migrations WHERE version = 'm20261118_000001_record_resumes'",
    )
    .fetch_one(&mut connection)
    .await
    .unwrap();
    assert_eq!(recorded, 0);
    assert_eq!(receipt_count(&mut connection).await, 0);
    let binding: String = query_scalar("SELECT launch_id FROM sessions WHERE native_id = 'owner'")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    assert_eq!(binding, "source");
    let claims: i64 =
        query_scalar("SELECT COUNT(*) FROM worktree_ownerships WHERE launch_id = 'source'")
            .fetch_one(&mut connection)
            .await
            .unwrap();
    assert_eq!(claims, 2);
}
