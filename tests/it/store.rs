use std::process::Stdio;

use crate::Fixture;

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
