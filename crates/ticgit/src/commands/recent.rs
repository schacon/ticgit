use anyhow::Result;
use clap::Parser;

use crate::commands::{open_store, SessionGitDir};
use crate::render;
use crate::session_state::State;

#[derive(Debug, Parser)]
pub struct Args {
    /// Number of recent tickets to show. Default 10.
    #[arg(short = 'n', long = "limit", default_value_t = 10)]
    pub limit: usize,

    /// Emit JSON.
    #[arg(long = "json")]
    pub json: bool,
}

pub fn run(args: Args) -> Result<()> {
    let store = open_store()?;
    let mut tickets = store.list()?;

    // Most-recently-touched proxy: created_at ∪ last comment time.
    tickets.sort_by(|a, b| {
        let a_recent = a.comments.last().map(|c| c.at).unwrap_or(a.created_at);
        let b_recent = b.comments.last().map(|c| c.at).unwrap_or(b.created_at);
        b_recent.cmp(&a_recent)
    });
    tickets.truncate(args.limit);

    if args.json {
        println!("{}", render::tickets_json(&tickets)?);
        return Ok(());
    }
    if tickets.is_empty() {
        println!("(no tickets)");
        return Ok(());
    }
    let state = State::load().unwrap_or_default();
    let current = state.current_for(&store.session().repo_git_dir());
    println!("{}", render::tickets_table(&tickets, current.as_ref()));
    Ok(())
}
