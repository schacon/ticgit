//! Per-subcommand handlers. Each module exposes `Args` (a clap struct)
//! and `run(args) -> anyhow::Result<()>`.

pub mod assign;
pub mod checkout;
pub mod claim;
pub mod close;
pub mod code;
pub mod comment;
pub mod depends;
pub mod edit;
pub mod history;
pub mod import;
pub mod init;
pub mod list;
pub mod meta;
pub mod milestone;
pub mod new;
pub mod next;
pub mod points;
pub mod priority;
pub mod pull;
pub mod recent;
pub mod setup;
pub mod show;
pub mod spec;
pub mod state;
pub mod stats;
pub mod subissue;
pub mod sync;
pub mod tag;
pub mod tui;
pub mod update;
pub mod users;
pub mod view;
pub mod writeup;

use anyhow::{Context, Result};
use ticgit_lib::TicketStore;
use uuid::Uuid;

use crate::session_state::State;

/// Open a ticket store rooted at the repo discovered from the current dir.
/// If no git-meta remote is configured but a `.git-meta` file exists,
/// auto-runs setup first so things just work after a fresh clone.
pub fn open_store() -> Result<TicketStore> {
    let _ = setup::auto_setup_if_needed();
    TicketStore::discover().context("opening ticgit store (are you inside a git repository?)")
}

/// Resolve a ticket reference using the explicit arg if given, otherwise
/// the currently checked-out ticket from session state.
pub fn resolve_ticket(store: &TicketStore, explicit: Option<&str>) -> Result<Uuid> {
    if let Some(reference) = explicit {
        return Ok(store.resolve_id(reference)?);
    }

    let state = State::load().unwrap_or_default();
    let git_dir = store.session().repo_git_dir();
    if let Some(id) = state.current_for(&git_dir) {
        return Ok(id);
    }
    anyhow::bail!(
        "no ticket specified and none checked out - pass a ticket id or run `ti checkout <id>` first"
    );
}

/// Tiny helper trait so we can ask the session for its git dir. We keep
/// this in the commands layer so ticgit-lib stays git-agnostic at its
/// surface.
pub trait SessionGitDir {
    fn repo_git_dir(&self) -> std::path::PathBuf;
}

impl SessionGitDir for ticgit_lib::Session {
    fn repo_git_dir(&self) -> std::path::PathBuf {
        // We don't have a public accessor in git-meta-lib, so fall back
        // to gix discovering from the current directory. In practice
        // both paths agree.
        gix::discover(".")
            .map(|r| r.git_dir().to_path_buf())
            .unwrap_or_else(|_| std::path::PathBuf::from(".git"))
    }
}
