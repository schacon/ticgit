use std::collections::BTreeSet;
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

#[test]
fn review_cli_records_branch_review_flow() {
    let repo = TestRepo::new();
    let ticket_id = create_ticket(&repo, "Code review tooling");

    repo.ti()
        .args([
            "review",
            "new",
            "--branch",
            "main",
            "--ticket",
            &ticket_id,
            "--title",
            "Stable review title",
            "--description",
            "Stable review description",
            "--reviewer",
            "alice@example.com",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created review main@"))
        .stdout(predicate::str::contains("Stable review title"));

    repo.ti()
        .args(["review", "list", "--status", "open"])
        .assert()
        .success()
        .stdout(predicate::str::contains("main"))
        .stdout(predicate::str::contains("open"))
        .stdout(predicate::str::contains("Stable review title"));

    fs::write(
        repo.dir.path().join("review.txt"),
        "updated review content\n",
    )
    .expect("write file");
    git(repo.dir.path(), &["add", "review.txt"]);
    git(
        repo.dir.path(),
        &["commit", "-m", "Last commit message", "--quiet"],
    );
    repo.ti()
        .args(["review", "update", "main"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Updated review main@"));

    repo.ti()
        .args(["review", "show", "main"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Title: Stable review title"))
        .stdout(predicate::str::contains(
            "Description: Stable review description",
        ))
        .stdout(predicate::str::contains("Last commit message").not());

    repo.ti()
        .args(["review", "add-reviewer", "bob@example.com"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Added reviewer bob@example.com"));

    repo.ti()
        .args([
            "review",
            "comment",
            "--path",
            "src/parser.rs",
            "--line",
            "42",
            "needs bounds checking",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Added comment"));

    repo.ti()
        .args(["review", "request-changes", "needs error handling"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Requested changes"));

    repo.ti()
        .args(["review", "approve", "--comment", "looks good"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Approved main@"));

    let head = git_output(repo.dir.path(), &["rev-parse", "HEAD"]);
    repo.ti()
        .args(["review", "integrate", &head])
        .assert()
        .success()
        .stdout(predicate::str::contains("Integrated main@"));

    repo.ti()
        .args(["review", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Status: merged"))
        .stdout(predicate::str::contains("Tickets:"))
        .stdout(predicate::str::contains("alice@example.com"))
        .stdout(predicate::str::contains("bob@example.com"))
        .stdout(predicate::str::contains("[comment]"))
        .stdout(predicate::str::contains("src/parser.rs:42"))
        .stdout(predicate::str::contains("[changes-requested]"))
        .stdout(predicate::str::contains("[approval]"))
        .stdout(predicate::str::contains("looks good"));
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
fn capturing_editor_script(repo: &TestRepo, captured: &Path, contents: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = repo.state_file.path().join("capturing-editor.sh");
    fs::write(
        &path,
        format!(
            "#!/bin/sh\ncp \"$1\" \"{}\"\ncat > \"$1\" <<'EOF'\n{contents}\nEOF\n",
            captured.display()
        ),
    )
    .expect("write capturing editor script");

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
fn agent_prints_markdown_guide() {
    let mut cmd = assert_cmd::Command::cargo_bin("ti").expect("ti binary");
    cmd.arg("agent")
        .assert()
        .success()
        .stdout(predicate::str::contains("---"))
        .stdout(predicate::str::contains("name: ticgit"))
        .stdout(predicate::str::contains("# TicGit Agent Guide"))
        .stdout(predicate::str::contains("ti new -F /tmp/ticket.md"))
        .stdout(predicate::str::contains("ti list --markdown"))
        .stdout(predicate::str::contains("Prefer `--markdown`"))
        .stdout(predicate::str::contains("ti close -t <id>"))
        .stdout(predicate::str::contains("--json").not());
}

#[test]
fn agent_skill_installs_local_shared_skill_and_checks_it() {
    let repo = TestRepo::new();

    repo.ti()
        .args(["agent", "skill", "--target", "agents-local"])
        .assert()
        .success()
        .stdout(predicate::str::contains(".agents/skills/ticgit/SKILL.md"));

    let skill = fs::read_to_string(repo.dir.path().join(".agents/skills/ticgit/SKILL.md"))
        .expect("skill file");
    assert!(skill.contains("name: ticgit"));
    assert!(skill.contains("# TicGit Agent Guide"));
    assert!(skill.contains("ti list --markdown"));

    repo.ti()
        .args(["agent", "skill", "--target", "agents-local", "--check"])
        .assert()
        .success()
        .stdout(predicate::str::contains("installed and current"));
}

#[test]
fn agent_skill_installs_agents_md_idempotently() {
    let repo = TestRepo::new();

    repo.ti()
        .args(["agent", "skill", "--target", "agents-md"])
        .assert()
        .success();
    repo.ti()
        .args(["agent", "skill", "--target", "agents-md"])
        .assert()
        .success();

    let agents = fs::read_to_string(repo.dir.path().join("AGENTS.md")).expect("AGENTS.md");
    assert_eq!(agents.matches("<!-- ticgit-agent-start -->").count(), 1);
    assert!(agents.contains("This project uses TicGit"));
    assert!(agents.contains("Run `ti agent`"));
}

#[test]
fn machine_output_schema_is_published_and_matches_cli_contract() {
    let schema: Value = serde_json::from_str(include_str!(env!("TICGIT_SCHEMA_V1_PATH"))).unwrap();
    assert_eq!(schema["$id"], "https://ticgit.dev/schema/v1.json");
    assert_eq!(schema["$defs"]["ticket"]["additionalProperties"], false);

    let required: BTreeSet<_> = schema["$defs"]["ticket"]["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        required,
        BTreeSet::from([
            "id".to_string(),
            "title".to_string(),
            "description".to_string(),
            "spec".to_string(),
            "status".to_string(),
            "state".to_string(),
            "assigned".to_string(),
            "closed_by".to_string(),
            "priority".to_string(),
            "points".to_string(),
            "milestone".to_string(),
            "code".to_string(),
            "parent".to_string(),
            "children".to_string(),
            "depends_on".to_string(),
            "blocks".to_string(),
            "tags".to_string(),
            "meta".to_string(),
            "comments".to_string(),
            "created_at".to_string(),
            "created_by".to_string(),
        ])
    );
    assert!(schema["$defs"]["ticket"]["properties"]["state"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .any(|state| state == "blocked"));
    assert_eq!(
        schema["$defs"]["ticket"]["properties"]["meta"]["additionalProperties"]["type"],
        "string"
    );

    let repo = TestRepo::new();
    let id = create_ticket(&repo, "schema ticket");
    repo.ti()
        .args(["meta", "-t", &id, "branch", "feature/schema"])
        .assert()
        .success();
    repo.ti()
        .args(["comment", "-t", &id, "schema note"])
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
    let ticket: Value = serde_json::from_slice(&output).unwrap();
    let ticket_keys: BTreeSet<_> = ticket
        .as_object()
        .unwrap()
        .keys()
        .map(|key| key.to_string())
        .collect();
    assert_eq!(ticket_keys, required);
    assert_eq!(ticket["id"], id);
    assert_eq!(ticket["status"], "open");
    assert_eq!(ticket["state"], "new");
    assert_eq!(ticket["meta"]["branch"], "feature/schema");
    assert_eq!(ticket["comments"][0]["body"], "schema note");

    let output = repo
        .ti()
        .args(["list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let list: Value = serde_json::from_slice(&output).unwrap();
    let first = &list.as_array().unwrap()[0];
    assert_eq!(first["id"], id);
    assert_eq!(first.as_object().unwrap().len(), required.len());
}

#[test]
fn json_machine_mode_keeps_stdout_parseable_and_plain() {
    let repo = TestRepo::new();
    let id = create_ticket(&repo, "machine ticket");

    for args in [
        vec!["show", &id, "--json"],
        vec!["list", "--json"],
        vec!["state", "blocked", "-t", &id, "--json"],
    ] {
        let output = repo
            .ti()
            .args(args)
            .assert()
            .success()
            .stderr(predicate::eq(""))
            .get_output()
            .stdout
            .clone();
        let stdout = String::from_utf8(output).unwrap();
        assert!(
            !stdout.contains("\x1b["),
            "JSON stdout must not contain ANSI escape sequences: {stdout:?}"
        );
        serde_json::from_str::<Value>(&stdout).expect("machine stdout is valid JSON");
    }
}

#[test]
fn json_machine_mode_errors_use_stderr_without_stdout() {
    let repo = TestRepo::new();

    repo.ti()
        .args(["show", "ffffffff", "--json"])
        .assert()
        .failure()
        .stdout(predicate::eq(""))
        .stderr(predicate::str::contains(
            "ticket prefix `ffffffff` matches no ticket",
        ));
}

#[test]
fn ambiguous_ticket_prefixes_fail_cleanly_in_machine_mode() {
    let repo = TestRepo::new();
    let mut prefixes: std::collections::BTreeMap<char, Vec<String>> =
        std::collections::BTreeMap::new();
    let ambiguous_prefix = (0..64).find_map(|i| {
        let id = create_ticket(&repo, &format!("ambiguous {i}"));
        let prefix = id.chars().next().unwrap();
        let ids = prefixes.entry(prefix).or_default();
        ids.push(id);
        (ids.len() == 2).then_some(prefix.to_string())
    });
    let ambiguous_prefix = ambiguous_prefix.expect("created two tickets with same leading hex");

    repo.ti()
        .args(["show", &ambiguous_prefix, "--json"])
        .assert()
        .failure()
        .stdout(predicate::eq(""))
        .stderr(predicate::str::contains("ambiguous"));
}

#[test]
fn version_flags_print_cargo_version() {
    let expected = format!("ti {}\n", env!("CARGO_PKG_VERSION"));

    for flag in ["-v", "--version"] {
        let mut cmd = assert_cmd::Command::cargo_bin("ti").expect("ti binary");
        cmd.arg(flag)
            .assert()
            .success()
            .stdout(predicate::eq(expected.as_str()));
    }
}

#[test]
fn help_lists_sync_and_pull_but_not_push() {
    let mut cmd = assert_cmd::Command::cargo_bin("ti").expect("ti binary");
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("sync"))
        .stdout(predicate::str::contains("pull"))
        .stdout(predicate::str::contains(" push ").not());
}

#[test]
fn bare_ti_defaults_to_list() {
    let repo = TestRepo::new();
    create_ticket(&repo, "bare ti");

    repo.ti()
        .assert()
        .success()
        .stdout(predicate::str::contains("bare ti"))
        .stderr(predicate::str::contains("unknown tag mode").not());
}

#[test]
fn list_open_shows_all_open_tickets_without_default_truncation() {
    let repo = TestRepo::new();
    for i in 0..25 {
        create_ticket(&repo, &format!("open ticket {i}"));
    }

    let default_output = repo
        .ti()
        .args(["list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let default_list: Value = serde_json::from_slice(&default_output).unwrap();
    assert_eq!(default_list.as_array().unwrap().len(), 20);

    let open_output = repo
        .ti()
        .args(["list", "--open", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let open_list: Value = serde_json::from_slice(&open_output).unwrap();
    assert_eq!(open_list.as_array().unwrap().len(), 25);
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
fn sync_prints_remote_url_and_ref() {
    let repo = TestRepo::new();
    let remote = tempfile::tempdir().expect("bare remote tempdir");
    git(remote.path(), &["init", "--bare", "--quiet"]);
    let remote_url = remote.path().to_string_lossy().to_string();

    git(repo.dir.path(), &["remote", "add", "origin", &remote_url]);
    repo.ti().arg("init").assert().success();

    repo.ti()
        .arg("sync")
        .assert()
        .failure()
        .stdout(predicate::str::contains("Remote: origin"))
        .stdout(predicate::str::contains("Ref: refs/meta/main"))
        .stdout(predicate::str::contains(format!("URL: {remote_url}")));
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
    assert_eq!(json["status"], "open");
    assert_eq!(json["state"], "new");

    repo.ti()
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("first bug"));

    repo.ti()
        .args(["show", &id, "--filter", ".title"])
        .assert()
        .success()
        .stdout(predicate::eq("first bug\n"));

    repo.ti()
        .args(["show", &id, "--filter"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Available filters:"))
        .stdout(predicate::str::contains("ti show <id> --filter '.title'"));
}

#[test]
fn markdown_output_includes_ticket_data_and_next_commands() {
    let repo = TestRepo::new();
    let id = create_ticket(&repo, "markdown ticket");

    repo.ti()
        .args(["meta", "-t", &id, "branch", "feature/markdown"])
        .assert()
        .success();
    repo.ti()
        .args(["comment", "-t", &id, "markdown note"])
        .assert()
        .success();

    repo.ti()
        .args(["show", &id, "--markdown"])
        .assert()
        .success()
        .stdout(predicate::str::contains("# Ticket: markdown ticket"))
        .stdout(predicate::str::contains(format!("- Id: `{id}`")))
        .stdout(predicate::str::contains("feature/markdown"))
        .stdout(predicate::str::contains("markdown note"))
        .stdout(predicate::str::contains("## Next Commands"))
        .stdout(predicate::str::contains("ti checkout"));

    repo.ti()
        .args(["list", "--markdown"])
        .assert()
        .success()
        .stdout(predicate::str::contains("# Tickets"))
        .stdout(predicate::str::contains(
            "| Id | Title | Status | State | Assigned | Tags | Created |",
        ))
        .stdout(predicate::str::contains("## Ticket Details"))
        .stdout(predicate::str::contains("ti show"));

    repo.ti()
        .args(["state", "blocked", "-t", &id, "--markdown"])
        .assert()
        .success()
        .stdout(predicate::str::contains("- Status: `open`"))
        .stdout(predicate::str::contains("- State: `blocked`"))
        .stdout(predicate::str::contains("ti state closed"));

    repo.ti().args(["checkout", &id]).assert().success();
    repo.ti()
        .args(["checkout", "--clear", "--markdown"])
        .assert()
        .success()
        .stdout(predicate::str::contains("# Current Ticket"))
        .stdout(predicate::str::contains("- Current: none"))
        .stdout(predicate::str::contains("ti list --markdown"));
}

#[test]
fn new_reads_title_and_description_from_file() {
    let repo = TestRepo::new();
    let file = repo.state_file.path().join("ticket.md");
    fs::write(&file, "file title\n\nfile description\nsecond line\n").unwrap();

    let output = repo
        .ti()
        .args(["new", "-F"])
        .arg(&file)
        .args(["--tags", "agent,feature", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["title"], "file title");
    assert_eq!(json["description"], "file description\nsecond line");
    let tags = json["tags"].as_array().unwrap();
    assert!(tags.iter().any(|tag| tag == "agent"));
    assert!(tags.iter().any(|tag| tag == "feature"));
}

#[test]
#[cfg(unix)]
fn edit_updates_title_and_description() {
    let repo = TestRepo::new();
    let id = create_ticket(&repo, "old title");
    let editor = editor_script(&repo, "new title\n\nnew description\nsecond line\n");

    repo.ti()
        .env("GIT_EDITOR", &editor)
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
#[cfg(unix)]
fn comment_editor_prompt_includes_ticket_title() {
    let repo = TestRepo::new();
    let id = create_ticket(&repo, "prompt title");
    let captured = repo.state_file.path().join("comment-template.md");
    let editor = capturing_editor_script(&repo, &captured, "edited body\n");

    repo.ti()
        .env("GIT_EDITOR", &editor)
        .args(["comment", "-t", &id, "--edit"])
        .assert()
        .success();

    let template = fs::read_to_string(captured).unwrap();
    assert!(template.contains("# Ticket comment"));
    assert!(template.contains("# Ticket: prompt title"));

    let output = repo
        .ti()
        .args(["show", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["comments"][0]["body"], "edited body");
}

#[test]
fn edit_reads_title_and_description_from_file() {
    let repo = TestRepo::new();
    let id = create_ticket(&repo, "old title");
    let file = repo.state_file.path().join("ticket-edit.md");
    fs::write(&file, "file edit title\n\nfile edit description\n").unwrap();

    let output = repo
        .ti()
        .args(["edit", &id, "-F"])
        .arg(&file)
        .arg("--json")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["title"], "file edit title");
    assert_eq!(json["description"], "file edit description");
}

#[test]
fn meta_sets_inline_and_file_values() {
    let repo = TestRepo::new();
    let id = create_ticket(&repo, "metadata ticket");
    let file = repo.state_file.path().join("meta-value.txt");
    fs::write(&file, "feature/meta\n").unwrap();

    repo.ti()
        .args(["meta", "-t", &id, "branch", "feature/parser"])
        .assert()
        .success()
        .stdout(predicate::str::contains("meta branch: feature/parser"));

    let output = repo
        .ti()
        .args(["meta", "-t", &id, "notes", "-F"])
        .arg(&file)
        .arg("--json")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["meta"]["branch"], "feature/parser");
    assert_eq!(json["meta"]["notes"], "feature/meta\n");

    repo.ti()
        .args(["show", &id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Metadata:"))
        .stdout(predicate::str::contains("branch"))
        .stdout(predicate::str::contains("feature/parser"));
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
        .args(["meta", "-t", &id, "source", "cli-test"])
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
    assert_eq!(json["status"], "closed");
    assert_eq!(json["state"], "resolved");
    assert_eq!(json["assigned"], "tester@example.com");
    assert_eq!(json["closed_by"], "tester@example.com");
    assert_eq!(json["points"], 5);
    assert_eq!(json["milestone"], "v1");
    assert_eq!(json["tags"].as_array().unwrap().len(), 2);
    assert_eq!(json["meta"]["source"], "cli-test");
    assert_eq!(json["comments"][0]["body"], "fixed now");
}

#[test]
fn ticket_mutations_support_json_output() {
    let repo = TestRepo::new();

    let output = repo
        .ti()
        .args(["new", "--title", "json ticket", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    let id = json["id"].as_str().unwrap().to_string();
    assert_eq!(json["title"], "json ticket");

    let output = repo
        .ti()
        .args(["tag", "-t", &id, "bug", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert!(json["tags"].as_array().unwrap().iter().any(|t| t == "bug"));

    let output = repo
        .ti()
        .args(["assign", "-t", &id, "octocat@github.com", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["assigned"], "octocat@github.com");

    let output = repo
        .ti()
        .args(["points", "-t", &id, "8", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["points"], 8);

    let output = repo
        .ti()
        .args(["milestone", "-t", &id, "v2", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["milestone"], "v2");

    let output = repo
        .ti()
        .args(["comment", "-t", &id, "hello", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["comments"][0]["body"], "hello");

    let output = repo
        .ti()
        .args(["state", "blocked", "-t", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["status"], "open");
    assert_eq!(json["state"], "blocked");
    assert_eq!(json["closed_by"], Value::Null);

    let output = repo
        .ti()
        .args(["claim", "-t", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["assigned"], "tester@example.com");
    assert_eq!(json["status"], "open");
    assert_eq!(json["state"], "assigned");

    let output = repo
        .ti()
        .args(["checkout", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["id"], id);
}

#[test]
fn state_and_status_commands_accept_status_state_and_combined_values() {
    let repo = TestRepo::new();
    let id = create_ticket(&repo, "lifecycle ticket");

    let output = repo
        .ti()
        .args(["state", "closed", "-t", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["status"], "closed");
    assert_eq!(json["state"], "resolved");

    let output = repo
        .ti()
        .args(["status", "open:blocked", "-t", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["status"], "open");
    assert_eq!(json["state"], "blocked");

    let output = repo
        .ti()
        .args(["state", "closed:wontfix", "-t", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["status"], "closed");
    assert_eq!(json["state"], "wontfix");

    repo.ti()
        .args(["status", "review", "-t", &id])
        .assert()
        .success()
        .stdout(predicate::str::contains("open:review"));
}

#[test]
fn state_without_value_requires_tty_stdin() {
    let repo = TestRepo::new();
    let id = create_ticket(&repo, "lifecycle menu");

    repo.ti()
        .args(["state", "-t", &id])
        .write_stdin("")
        .assert()
        .failure()
        .stderr(predicate::str::contains("missing STATE"));
}

#[test]
fn state_without_value_rejects_json_without_explicit_state() {
    let repo = TestRepo::new();
    let id = create_ticket(&repo, "json state");

    repo.ti()
        .args(["state", "-t", &id, "--json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--json"));
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
fn close_resolves_current_ticket_and_clears_checkout() {
    let repo = TestRepo::new();
    let id = create_ticket(&repo, "current close ticket");

    repo.ti().args(["checkout", &id]).assert().success();
    repo.ti()
        .arg("close")
        .assert()
        .success()
        .stdout(predicate::str::contains("cleared current ticket"));

    let output = repo
        .ti()
        .args(["show", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["status"], "closed");
    assert_eq!(json["state"], "resolved");

    repo.ti()
        .arg("show")
        .assert()
        .failure()
        .stderr(predicate::str::contains("none checked out"));
}

#[test]
fn close_explicit_ticket_keeps_other_checkout() {
    let repo = TestRepo::new();
    let current = create_ticket(&repo, "current ticket");
    let other = create_ticket(&repo, "other ticket");

    repo.ti().args(["checkout", &current]).assert().success();
    let output = repo
        .ti()
        .args(["close", &other, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["id"], other);
    assert_eq!(json["status"], "closed");
    assert_eq!(json["state"], "resolved");

    repo.ti()
        .arg("show")
        .assert()
        .success()
        .stdout(predicate::str::contains("current ticket"));
}

#[test]
fn new_checkout_selects_created_ticket() {
    let repo = TestRepo::new();

    repo.ti()
        .args(["new", "--title", "checked out on create", "--checkout"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Checked out:"));

    let output = repo
        .ti()
        .args(["show", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["title"], "checked out on create");
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

    // Save the last list filters as a view.
    repo.ti().args(["views", "save", "bugs"]).assert().success();

    repo.ti()
        .args(["views"])
        .assert()
        .success()
        .stdout(predicate::str::contains("bugs"))
        .stdout(predicate::str::contains("--tag bug"));

    // Load the view via `ti list <name>`.
    repo.ti()
        .args(["list", "bugs"])
        .assert()
        .success()
        .stdout(predicate::str::contains("bug ticket"))
        .stdout(predicate::str::contains("docs ticket").not());

    // Delete the view.
    repo.ti()
        .args(["views", "delete", "bugs"])
        .assert()
        .success();
}

#[test]
fn writeup_workflow_creates_versions_links_and_promotes() {
    let repo = TestRepo::new();
    let output = repo
        .ti()
        .args([
            "writeup",
            "new",
            "--title",
            "Rethink sync",
            "--body",
            "Initial notes",
            "--tags",
            "design",
            "--id-only",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let writeup = String::from_utf8(output).unwrap().trim().to_string();
    let writeup_prefix = &writeup[..6];

    repo.ti()
        .args(["writeup", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains(writeup_prefix))
        .stdout(predicate::str::contains("Rethink sync"))
        .stdout(predicate::str::contains("[design]"));

    repo.ti()
        .args(["writeup", "edit", writeup_prefix, "--body", "Second notes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Appended version 2"));

    repo.ti()
        .args(["tag", "--writeup", writeup_prefix, "review"])
        .assert()
        .success()
        .stdout(predicate::str::contains("review"));
    repo.ti()
        .args(["tag", "--writeup", writeup_prefix, "--remove", "design"])
        .assert()
        .success();

    repo.ti()
        .args(["writeup", "show", writeup_prefix])
        .assert()
        .success()
        .stdout(predicate::str::contains("# Writeup: Rethink sync"))
        .stdout(predicate::str::contains("- Tags: review"))
        .stdout(predicate::str::contains("Second notes"))
        .stdout(predicate::str::contains("Initial notes").not());

    let ticket = create_ticket(&repo, "related ticket");
    repo.ti()
        .args(["writeup", "link", writeup_prefix, &ticket[..6]])
        .assert()
        .success();
    repo.ti()
        .args(["writeup", "show", writeup_prefix])
        .assert()
        .success()
        .stdout(predicate::str::contains(&ticket));
    repo.ti()
        .args(["writeup", "unlink", writeup_prefix, &ticket[..6]])
        .assert()
        .success();

    let promoted_output = repo
        .ti()
        .args(["writeup", "promote", writeup_prefix])
        .assert()
        .success()
        .stdout(predicate::str::contains("Promoted writeup"))
        .get_output()
        .stdout
        .clone();
    let promoted_stdout = String::from_utf8(promoted_output).unwrap();
    let promoted_id = promoted_stdout
        .lines()
        .find_map(|line| line.strip_prefix("Full ticket id: "))
        .expect("promoted ticket id");
    repo.ti()
        .args(["show", promoted_id, "--markdown"])
        .assert()
        .success()
        .stdout(predicate::str::contains("# Ticket: Rethink sync"))
        .stdout(predicate::str::contains("review"))
        .stdout(predicate::str::contains("Second notes"));

    repo.ti()
        .args(["writeup", "close", writeup_prefix])
        .assert()
        .success();
    repo.ti()
        .args(["writeup", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Rethink sync").not());
    repo.ti()
        .args(["writeup", "list", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Rethink sync"));
}

#[cfg(unix)]
#[test]
fn writeup_edit_editor_uses_first_line_as_title() {
    let repo = TestRepo::new();
    let output = repo
        .ti()
        .args([
            "writeup",
            "new",
            "--title",
            "Original title",
            "--body",
            "Original body",
            "--id-only",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let writeup = String::from_utf8(output).unwrap().trim().to_string();
    let writeup_prefix = &writeup[..6];
    let editor = editor_script(&repo, "Updated title\n\nUpdated body");

    repo.ti()
        .env("EDITOR", editor)
        .args(["writeup", "edit", writeup_prefix])
        .assert()
        .success();

    repo.ti()
        .args(["writeup", "show", writeup_prefix])
        .assert()
        .success()
        .stdout(predicate::str::contains("# Writeup: Updated title"))
        .stdout(predicate::str::contains("Updated body"))
        .stdout(predicate::str::contains("Updated title\n\nUpdated title").not())
        .stdout(predicate::str::contains("Original body").not());
}

#[cfg(unix)]
#[test]
fn writeup_edit_editor_preserves_markdown_headings() {
    let repo = TestRepo::new();
    let output = repo
        .ti()
        .args([
            "writeup",
            "new",
            "--title",
            "Original title",
            "--body",
            "Original body",
            "--id-only",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let writeup = String::from_utf8(output).unwrap().trim().to_string();
    let writeup_prefix = &writeup[..6];
    let editor = editor_script(
        &repo,
        "Updated title\n\n# First heading\n\nBody\n\n## Second heading",
    );

    repo.ti()
        .env("EDITOR", editor)
        .args(["writeup", "edit", writeup_prefix])
        .assert()
        .success();

    repo.ti()
        .args(["writeup", "show", writeup_prefix])
        .assert()
        .success()
        .stdout(predicate::str::contains("# First heading"))
        .stdout(predicate::str::contains("## Second heading"))
        .stdout(predicate::str::contains("Original body").not());
}

#[test]
fn list_search_filters_title_description_and_comments() {
    let repo = TestRepo::new();
    let title = create_ticket(&repo, "parser panic");
    let file = repo.state_file.path().join("description-ticket.md");
    fs::write(
        &file,
        "description ticket\n\nThis ticket explains parser recovery.\n",
    )
    .unwrap();
    let output = repo
        .ti()
        .args(["new", "-F"])
        .arg(&file)
        .arg("--json")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let description: Value = serde_json::from_slice(&output).unwrap();
    let description_id = description["id"].as_str().unwrap().to_string();
    let comment = create_ticket(&repo, "comment ticket");
    repo.ti()
        .args(["comment", "-t", &comment, "parser appears in a comment"])
        .assert()
        .success();

    let output = repo
        .ti()
        .args(["list", "--search", "parser", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    let tickets = json.as_array().unwrap();
    assert_eq!(tickets.len(), 3);

    let output = repo
        .ti()
        .args(["list", "--search", "title:parser", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    let tickets = json.as_array().unwrap();
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0]["id"], title);

    let output = repo
        .ti()
        .args(["list", "--search", "description:recovery", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    let tickets = json.as_array().unwrap();
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0]["id"], description_id);

    let output = repo
        .ti()
        .args(["list", "--search", "comments:appears", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    let tickets = json.as_array().unwrap();
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0]["id"], comment);
}

#[test]
fn list_all_includes_non_open_tickets() {
    let repo = TestRepo::new();
    let id = create_ticket(&repo, "closed ticket");
    repo.ti()
        .args(["state", "resolved", "-t", &id])
        .assert()
        .success();

    repo.ti()
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("closed ticket").not());

    let output = repo
        .ti()
        .args(["list", "--all", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json.as_array().unwrap()[0]["title"], "closed ticket");
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
    "author": {"login": "monalisa"},
    "labels": [{"name": "bug"}],
    "assignees": [{"login": "octocat"}, {"login": "hubot"}],
    "milestone": {"title": "v1"}
  },
  {
    "number": 8,
    "title": "second gh issue",
    "body": "",
    "url": "https://github.com/owner/repo/issues/8",
    "author": {"login": "hubot"},
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
    assert_eq!(first["assigned"], "octocat@users.noreply.github.com");
    assert_eq!(first["milestone"], "v1");
    assert_eq!(
        first["description"],
        "GitHub issue: https://github.com/owner/repo/issues/7\nGitHub author: monalisa\nGitHub assignees: octocat, hubot\n\nImported body"
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
