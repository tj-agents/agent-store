use std::path::Path;

use sea_orm::sqlx::sqlite::SqliteConnectOptions;
use sea_orm::sqlx::{ConnectOptions, Connection, Executor};

use crate::Fixture;

const LEGACY_SCHEMA: &str = r"
CREATE TABLE work_items (key TEXT PRIMARY KEY, repo TEXT NOT NULL, feature TEXT, item TEXT,
    status TEXT NOT NULL, branch TEXT, pr TEXT, title TEXT, plan_path TEXT, updated_at REAL);
CREATE TABLE work_item_references (provider TEXT NOT NULL, scope TEXT NOT NULL,
    native_id TEXT NOT NULL, work_item TEXT NOT NULL REFERENCES work_items(key),
    PRIMARY KEY (provider, scope, native_id));
CREATE TABLE sessions (handle TEXT PRIMARY KEY, host TEXT NOT NULL, session_id TEXT NOT NULL,
    machine TEXT, profile TEXT, title TEXT, cwd TEXT, repo TEXT, branch TEXT, head TEXT,
    work_item TEXT REFERENCES work_items(key), role TEXT, pid INTEGER, pid_started_at REAL,
    started_at REAL, last_activity_at REAL, turn_state TEXT, ended_at REAL, end_reason TEXT,
    predecessor TEXT, successor TEXT);
INSERT INTO work_items (key, repo, status) VALUES
    ('execution:a', 'github.com/tj-agents/core', 'active'),
    ('execution:b', 'github.com/tj-agents/cli', 'active'),
    ('execution:none', 'github.com/tj-agents/docs', 'active');
INSERT INTO work_item_references VALUES
    ('github', 'github.com/tj-agents/core', '473', 'execution:a'),
    ('github', 'github.com/tj-agents/core', '530', 'execution:a'),
    ('github', 'github.com/tj-agents/cli', '31', 'execution:b'),
    ('github', 'github.com/tj-agents/docs', '8', 'execution:none');
INSERT INTO sessions (handle, host, session_id, cwd, work_item, role, started_at, ended_at, end_reason) VALUES
    ('codex:01a1', 'codex', '01a1', 'C:\repos\core', 'execution:a', 'owner', 1792599225.4595122, NULL, NULL),
    ('claude:c2', 'claude', 'c2', '/repos/cli', 'execution:b', 'bounded: reusable onboarding repair',
        1792881361.0, 1792882481.6538434, 'handed_off'),
    ('claude:loose', 'claude', 'loose', '/repos/x', NULL, NULL, 1792000000.0, NULL, NULL);
PRAGMA user_version = 4;
";

async fn create_legacy(path: &Path, schema: &'static str) {
    let mut connection = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .connect()
        .await
        .unwrap();
    connection.execute(schema).await.unwrap();
    connection.close().await.unwrap();
}

fn legacy_database(fixture: &Fixture, schema: &'static str) -> String {
    let path = fixture.path("sessions.db");
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(create_legacy(&path, schema));
    path.to_str().unwrap().to_owned()
}

#[test]
fn imports_sessions_links_and_end_state() {
    let fixture = Fixture::new();
    let legacy = legacy_database(&fixture, LEGACY_SCHEMA);
    let summary = fixture.ok(&["import-legacy", "--from", &legacy]);
    assert_eq!(summary["sessions"], 3);
    assert_eq!(summary["links"], 3);
    assert_eq!(summary["unlinked_references"], 1);

    let owner = fixture.ok(&["show", "codex:01a1"])["session"].clone();
    assert_eq!(owner["state"], "active");
    assert_eq!(owner["cwd"], r"C:\repos\core");
    assert_eq!(owner["started_at"], "2026-10-21T16:13:45.459512Z");
    assert_eq!(owner["links"].as_array().unwrap().len(), 2);
    assert_eq!(owner["links"][0]["role"], "owner");

    let helper = fixture.ok(&["show", "claude:c2"])["session"].clone();
    assert_eq!(helper["state"], "ended");
    assert_eq!(helper["end_reason"], "handed_off");
    assert_eq!(helper["links"][0]["role"], "helper");

    assert_eq!(
        fixture.ok(&["show", "claude:loose"])["session"]["links"],
        serde_json::json!([])
    );

    let rerun = fixture.ok(&["import-legacy", "--from", &legacy]);
    assert_eq!(rerun["sessions"], 3);
    assert_eq!(fixture.ok(&["items"])["items"].as_array().unwrap().len(), 3);
}

#[test]
fn refuses_another_schema_version() {
    let fixture = Fixture::new();
    let legacy = legacy_database(&fixture, "CREATE TABLE x (y); PRAGMA user_version = 5;");
    let run = fixture.run(&["import-legacy", "--from", &legacy]);
    assert_eq!(run.code, 2, "{}", run.json);
    assert!(
        run.json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("schema version 5")
    );
}
