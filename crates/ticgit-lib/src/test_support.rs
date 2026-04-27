//! Helpers for spinning up an isolated git repo + ticket store for tests.
//!
//! Only compiled under `cfg(test)` (or when consumers enable the
//! `test-support` feature).

#![cfg(any(test, feature = "test-support"))]

use std::process::Command;

use git_meta_lib::Session;
use tempfile::TempDir;

use crate::store::TicketStore;

/// Build a fresh git repo in a tempdir, configure user identity, and
/// open a [`TicketStore`] on top of it. Returns both so the tempdir
/// stays alive for the duration of the test.
pub fn test_store() -> (TicketStore, TempDir) {
    let td = tempfile::tempdir().expect("tempdir");
    let path = td.path();

    run_git(path, &["init", "--quiet", "-b", "main"]);
    run_git(path, &["config", "user.email", "tester@example.com"]);
    run_git(path, &["config", "user.name", "Tester"]);
    // git-meta needs at least one commit to anchor refs in some flows;
    // we make an empty initial commit so push/pull paths have something
    // to attach to.
    run_git(path, &["commit", "--allow-empty", "-m", "init", "--quiet"]);

    let repo = gix::open(path).expect("gix open");
    let session = Session::open(repo).expect("session open");
    let store = TicketStore::from_session(session).expect("ticket store");
    (store, td)
}

fn run_git(cwd: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .expect("spawn git");
    assert!(status.success(), "git {args:?} failed");
}
