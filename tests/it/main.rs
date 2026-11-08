// `allow-unwrap-in-tests` does not reach helper functions outside `#[test]`.
#![allow(clippy::unwrap_used)]

mod cli;
mod launch;
mod legacy;
mod store;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

pub(crate) struct Run {
    pub(crate) code: i32,
    pub(crate) json: Value,
}

pub(crate) struct Fixture {
    pub(crate) dir: tempfile::TempDir,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    pub(crate) fn database(&self) -> PathBuf {
        self.dir.path().join("state").join("agent-store.db")
    }

    pub(crate) fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent-store"));
        command
            .args(args)
            .env("AGENT_STORE_DATABASE", self.database())
            .current_dir(self.dir.path());
        command
    }

    pub(crate) fn run(&self, args: &[&str]) -> Run {
        finish(&mut self.command(args), None)
    }

    pub(crate) fn run_input(&self, request: &str) -> Run {
        finish(&mut self.command(&["--input", "-"]), Some(request))
    }

    pub(crate) fn ok(&self, args: &[&str]) -> Value {
        let run = self.run(args);
        assert_eq!(run.code, 0, "{args:?} -> {}", run.json);
        assert_eq!(run.json["schema_version"], 1);
        assert_eq!(run.json["ok"], true);
        run.json["data"].clone()
    }

    pub(crate) fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }
}

fn finish(command: &mut Command, stdin: Option<&str>) -> Run {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.unwrap_or_default().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let json = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not one JSON document ({e}): {stdout}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    Run {
        code: output.status.code().unwrap(),
        json,
    }
}

pub(crate) fn git_repository(dir: &Path, origin: &str) {
    for args in [
        &["init", "-q"][..],
        &["remote", "add", "origin", origin][..],
    ] {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success());
    }
}
