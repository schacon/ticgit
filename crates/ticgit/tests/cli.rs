use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;

struct TestRepo {
    dir: TempDir,
    state_file: TempDir,
}

impl TestRepo {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("repo tempdir");
        let state_file = tempfile::tempdir().expect("state tempdir");
        git(dir.path(), &["init", "--quiet", "-b", "main"]);
        git(dir.path(), &["config", "user.email", "tester@example.com"]);
        git(dir.path(), &["config", "user.name", "Tester"]);
        git(
            dir.path(),
            &["commit", "--allow-empty", "-m", "init", "--quiet"],
        );
        Self { dir, state_file }
    }

    fn ti(&self) -> assert_cmd::Command {
        let mut cmd = assert_cmd::Command::cargo_bin("ti").expect("ti binary");
        cmd.current_dir(self.dir.path());
        cmd.env(
            "TICGIT_STATE_FILE",
            self.state_file.path().join("state.json"),
        );
        cmd
    }
}

fn git(cwd: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .expect("spawn git");
    assert!(status.success(), "git {args:?} failed");
}

fn git_output(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("spawn git");
    assert!(output.status.success(), "git {args:?} failed");
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn create_ticket(repo: &TestRepo, title: &str) -> String {
    let output = repo
        .ti()
        .args(["new", "--title", title, "--id-only"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output).unwrap().trim().to_string()
}

#[cfg(unix)]
fn editor_script(repo: &TestRepo, contents: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = repo.state_file.path().join("editor.sh");
    fs::write(
        &path,
        format!("#!/bin/sh\ncat > \"$1\" <<'EOF'\n{contents}\nEOF\n"),
    )
    .expect("write editor script");

    let mut permissions = fs::metadata(&path).expect("editor metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).expect("chmod editor script");
    path
}

#[cfg(unix)]
fn executable_script(dir: &Path, name: &str, contents: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    fs::create_dir_all(dir).expect("script dir");
    let path = dir.join(name);
    fs::write(&path, contents).expect("write executable script");

    let mut permissions = fs::metadata(&path).expect("script metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).expect("chmod executable script");
    path
}

#[test]
fn init_is_idempotent() {
    let repo = TestRepo::new();
    repo.ti()
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("ticgit initialised"));

    repo.ti()
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("schema 1"));
}

#[test]
fn init_bootstraps_git_meta_defaults() {
    let repo = TestRepo::new();
    git(
        repo.dir.path(),
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/repo.git",
        ],
    );

    repo.ti()
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Configured git-meta remote 'origin' with defaults.",
        ))
        .stdout(predicate::str::contains("ticgit initialised"));

    assert_eq!(
        git_output(repo.dir.path(), &["config", "--get", "meta.namespace"]),
        "meta",
    );
    assert_eq!(
        git_output(
            repo.dir.path(),
            &["config", "--bool", "--get", "remote.origin.meta"],
        ),
        "true",
    );
    let fetch = git_output(
        repo.dir.path(),
        &["config", "--get-all", "remote.origin.fetch"],
    );
    assert!(fetch.contains("+refs/meta/main:refs/meta/remotes/main"));
}

#[test]
fn new_show_and_list_round_trip() {
    let repo = TestRepo::new();
    let id = create_ticket(&repo, "first bug");

    let output = repo
        .ti()
        .args(["show", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["id"], id);
    assert_eq!(json["title"], "first bug");
    assert_eq!(json["state"], "open");

    repo.ti()
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("first bug"))
        .stdout(predicate::str::contains(&id[..6]));
}

#[test]
#[cfg(unix)]
fn edit_updates_title_and_description() {
    let repo = TestRepo::new();
    let id = create_ticket(&repo, "old title");
    let editor = editor_script(&repo, "new title\n\nnew description\nsecond line\n");

    repo.ti()
        .env("EDITOR", editor)
        .args(["edit", &id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Updated"));

    let output = repo
        .ti()
        .args(["show", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["title"], "new title");
    assert_eq!(json["description"], "new description\nsecond line");
}

#[test]
fn mutating_commands_update_ticket() {
    let repo = TestRepo::new();
    let id = create_ticket(&repo, "mutate me");

    repo.ti()
        .args(["tag", "-t", &id, "bug,ui"])
        .assert()
        .success();
    repo.ti()
        .args(["assign", "-t", &id, "tester@example.com"])
        .assert()
        .success();
    repo.ti()
        .args(["points", "-t", &id, "5"])
        .assert()
        .success();
    repo.ti()
        .args(["milestone", "-t", &id, "v1"])
        .assert()
        .success();
    repo.ti()
        .args(["state", "resolved", "-t", &id])
        .assert()
        .success();
    repo.ti()
        .args(["comment", "-t", &id, "fixed", "now"])
        .assert()
        .success();

    let output = repo
        .ti()
        .args(["show", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["state"], "resolved");
    assert_eq!(json["assigned"], "tester@example.com");
    assert_eq!(json["points"], 5);
    assert_eq!(json["milestone"], "v1");
    assert_eq!(json["tags"].as_array().unwrap().len(), 2);
    assert_eq!(json["comments"][0]["body"], "fixed now");
}

#[test]
fn checkout_makes_ticket_optional_for_show_and_comment() {
    let repo = TestRepo::new();
    let id = create_ticket(&repo, "selected ticket");

    repo.ti().args(["checkout", &id[..6]]).assert().success();
    repo.ti()
        .args(["comment", "from", "current"])
        .assert()
        .success();

    repo.ti()
        .arg("show")
        .assert()
        .success()
        .stdout(predicate::str::contains("selected ticket"))
        .stdout(predicate::str::contains("from current"));
}

#[test]
fn list_filters_and_saved_views_work() {
    let repo = TestRepo::new();
    let bug = create_ticket(&repo, "bug ticket");
    let docs = create_ticket(&repo, "docs ticket");

    repo.ti()
        .args(["tag", "-t", &bug, "bug"])
        .assert()
        .success();
    repo.ti()
        .args(["tag", "-t", &docs, "docs"])
        .assert()
        .success();

    repo.ti()
        .args(["list", "--tag", "bug"])
        .assert()
        .success()
        .stdout(predicate::str::contains("bug ticket"))
        .stdout(predicate::str::contains("docs ticket").not());

    repo.ti()
        .args(["save-view", "bugs", "--tag", "bug"])
        .assert()
        .success();

    repo.ti()
        .args(["views"])
        .assert()
        .success()
        .stdout(predicate::str::contains("bugs"));

    repo.ti()
        .args(["views", "bugs"])
        .assert()
        .success()
        .stdout(predicate::str::contains(&bug))
        .stdout(predicate::str::contains(&docs).not());
}

#[test]
#[cfg(unix)]
fn import_gh_creates_tickets_and_skips_existing_issues() {
    let repo = TestRepo::new();
    let bin = repo.state_file.path().join("bin");
    executable_script(
        &bin,
        "gh",
        r#"#!/bin/sh
cat <<'JSON'
[
  {
    "number": 7,
    "title": "first gh issue",
    "body": "Imported body",
    "url": "https://github.com/owner/repo/issues/7",
    "labels": [{"name": "bug"}],
    "assignees": [{"login": "octocat"}, {"login": "hubot"}],
    "milestone": {"title": "v1"}
  },
  {
    "number": 8,
    "title": "second gh issue",
    "body": "",
    "url": "https://github.com/owner/repo/issues/8",
    "labels": [],
    "assignees": [],
    "milestone": null
  }
]
JSON
"#,
    );
    let path = format!(
        "{}:{}",
        bin.display(),
        env::var_os("PATH").unwrap_or_default().to_string_lossy()
    );

    repo.ti()
        .env("PATH", &path)
        .args(["import", "gh", "--repo", "owner/repo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Imported 2 GitHub issue(s)."));

    let output = repo
        .ti()
        .args(["list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    let tickets = json.as_array().unwrap();
    assert_eq!(tickets.len(), 2);

    let first = tickets
        .iter()
        .find(|ticket| ticket["title"] == "first gh issue")
        .unwrap();
    assert_eq!(first["assigned"], "octocat");
    assert_eq!(first["milestone"], "v1");
    assert_eq!(
        first["description"],
        "GitHub issue: https://github.com/owner/repo/issues/7\nGitHub assignees: octocat, hubot\n\nImported body"
    );
    let tags = first["tags"].as_array().unwrap();
    assert!(tags.iter().any(|tag| tag == "github"));
    assert!(tags.iter().any(|tag| tag == "github-issue-7"));
    assert!(tags.iter().any(|tag| tag == "bug"));

    repo.ti()
        .env("PATH", &path)
        .args(["import", "gh", "--repo", "owner/repo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Imported 0 GitHub issue(s)."))
        .stdout(predicate::str::contains(
            "Skipped 2 issue(s) that were already imported.",
        ));
}
