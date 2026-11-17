use std::process::Stdio;

use crate::Fixture;

#[tokio::test]
async fn existing_explicit_provider_rows_keep_their_identity_when_reopened() {
    use sea_orm::sqlx::{Connection, SqliteConnection, query, sqlite::SqliteConnectOptions};

    for (provider, scope, native_id) in [
        ("url", "acme", "APP-12"),
        ("url", "1invalid", "APP-12"),
        ("url", "tenant:sub", "APP-12"),
        ("url", "acme", "APP:12"),
        ("jira", "https://tracker.example", "APP-12"),
    ] {
        let fixture = Fixture::new();
        fixture.ok(&["register", "--session", "claude:existing", "--cwd", "/w"]);
        fixture.ok(&[
            "link",
            "--session",
            "claude:existing",
            "--item",
            "github:github.com/acme/app:1",
        ]);
        // Seed previously valid storage directly, independently of today's reference parser.
        let options = SqliteConnectOptions::new().filename(fixture.database());
        let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
        query("UPDATE work_item_references SET provider = ?, scope = ?, native_id = ?")
            .bind(provider)
            .bind(scope)
            .bind(native_id)
            .execute(&mut connection)
            .await
            .unwrap();
        connection.close().await.unwrap();

        let expected = serde_json::json!({
            "ref": format!("{provider}:{scope}:{native_id}"),
            "provider": provider, "scope": scope, "native_id": native_id,
        });
        let shown = fixture.ok(&["show", "claude:existing"]);
        assert_eq!(shown["session"]["links"][0]["item"], expected);
        assert_eq!(fixture.ok(&["items"])["items"][0]["item"], expected);
    }
}

#[test]
fn tracker_urls_round_trip_and_match_sessions_and_launches_after_reopening() {
    for (input, lookup, expected) in [
        (
            "https://dev.azure.com/Contoso/Web/_workitems/edit/00461",
            "https://dev.azure.com/contoso/_workitems/edit/461",
            serde_json::json!({
                "ref": "azure-devops:dev.azure.com/contoso:461",
                "provider": "azure-devops", "scope": "dev.azure.com/contoso", "native_id": "461",
            }),
        ),
        (
            "https://dev.azure.com/Contoso/_workitems/edit/461",
            "https://contoso.visualstudio.com/Web/_workitems/edit/461",
            serde_json::json!({
                "ref": "azure-devops:dev.azure.com/contoso:461",
                "provider": "azure-devops", "scope": "dev.azure.com/contoso", "native_id": "461",
            }),
        ),
        (
            "HTTPS://Tracker.Example:8443/items/APP-12?view=a:b#comment:2",
            "https://tracker.example:8443/items/APP-12?view=a:b#comment:2",
            serde_json::json!({
                "ref": "url:https://tracker.example:8443/items/APP-12?view=a:b#comment:2",
                "provider": "url", "scope": "https",
                "native_id": "//tracker.example:8443/items/APP-12?view=a:b#comment:2",
            }),
        ),
    ] {
        let fixture = Fixture::new();
        // The code host does not choose which tracker owns these references.
        crate::git_repository(fixture.dir.path(), "git@github.com:acme/app.git");
        let launch = fixture.ok(&[
            "launch",
            "--launch-id",
            "tracker",
            "--host",
            "claude",
            "--title",
            "tracker item",
            "--cwd",
            "/w",
            "--wrapper-pid",
            "900",
            "--wrapper-started-at",
            "2026-11-08T10:00:00Z",
            "--item",
            input,
        ]);
        assert_eq!(launch["launch"]["item"], expected);
        fixture.ok(&[
            "register",
            "--session",
            "claude:owner",
            "--cwd",
            "/w",
            "--launch-id",
            "tracker",
        ]);
        fixture.ok(&["register", "--session", "codex:reviewer", "--cwd", "/w"]);
        let linked = fixture.run_input(
            &serde_json::json!({
                "command": "link", "session": "codex:reviewer", "item": input,
                "role": "reviewer",
            })
            .to_string(),
        );
        assert_eq!(linked.code, 0, "{}", linked.json);

        // Every CLI call opens the persisted database in a fresh process.
        let shown = fixture.ok(&["show", "claude:owner"]);
        assert_eq!(shown["session"]["links"][0]["item"], expected);
        let shown = fixture.ok(&["show", "--launch-id", "tracker"]);
        assert_eq!(shown["launch"]["item"], expected);
        assert_eq!(shown["session"]["links"][0]["item"], expected);

        let canonical = expected["ref"].as_str().unwrap();
        for reference in [lookup, canonical] {
            fixture.ok(&["link", "--session", "codex:reviewer", "--item", reference]);
            let sessions = fixture.ok(&["sessions", "--item", reference]);
            assert_eq!(sessions["item"], expected);
            assert_eq!(sessions["sessions"].as_array().unwrap().len(), 2);
            for session in sessions["sessions"].as_array().unwrap() {
                assert_eq!(session["links"].as_array().unwrap().len(), 1);
                assert_eq!(session["links"][0]["item"], expected);
            }
            let launches = fixture.ok(&["launches", "--item", reference]);
            assert_eq!(launches["launches"].as_array().unwrap().len(), 1);
            assert_eq!(launches["launches"][0]["item"], expected);
        }
        let items = fixture.ok(&["items"]);
        assert_eq!(items["items"].as_array().unwrap().len(), 1);
        assert_eq!(items["items"][0]["item"], expected);

        let closed = fixture.ok(&["mark-closing", "--item", lookup]);
        assert_eq!(closed["sessions"].as_array().unwrap().len(), 2);
        assert_eq!(closed["launches"].as_array().unwrap().len(), 1);
        fixture.ok(&["end", "--session", "claude:owner"]);
        let recovered = fixture.ok(&["sessions", "--item", canonical]);
        assert_eq!(recovered["sessions"][0]["state"], "ended");
        assert_eq!(recovered["sessions"][1]["state"], "closing");
        assert_eq!(recovered["sessions"][0]["links"][0]["item"], expected);
    }
}

#[test]
fn reopening_a_migrated_database_keeps_its_rows() {
    let fixture = Fixture::new();
    fixture.ok(&["register", "--session", "claude:keep", "--cwd", "/w"]);
    fixture.ok(&["items"]);
    assert_eq!(fixture.ok(&["show", "claude:keep"])["session"]["cwd"], "/w");
}

#[test]
fn concurrent_writers_never_undo_each_other() {
    let fixture = Fixture::new();
    fixture.ok(&["register", "--session", "claude:busy", "--cwd", "/w"]);
    let mut children: Vec<_> = (1..=6)
        .map(|n| {
            fixture
                .command(&[
                    "link",
                    "--session",
                    "claude:busy",
                    "--item",
                    &format!("github:github.com/a/b:{n}"),
                ])
                .stdout(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    children.push(
        fixture
            .command(&["end", "--session", "claude:busy"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    for child in children {
        assert!(child.wait_with_output().unwrap().status.success());
    }
    let session = fixture.ok(&["show", "claude:busy"])["session"].clone();
    assert_eq!(session["state"], "ended");
    assert_eq!(session["links"].as_array().unwrap().len(), 6);
}

#[test]
fn concurrent_first_use_creates_the_schema_once() {
    for _ in 0..4 {
        let fixture = Fixture::new();
        let children: Vec<_> = (0..8)
            .map(|n| {
                fixture
                    .command(&[
                        "register",
                        "--session",
                        &format!("codex:c{n}"),
                        "--cwd",
                        "/w",
                    ])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap()
            })
            .collect();
        for child in children {
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
        for n in 0..8 {
            fixture.ok(&["show", &format!("codex:c{n}")]);
        }
    }
}
