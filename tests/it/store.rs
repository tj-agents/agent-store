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
