use anyhow::Result;
use clap::Parser;

use crate::commands::{open_store, SessionGitDir};
use crate::render;
use crate::session_state::State;

#[derive(Debug, Parser)]
pub struct Args {
    /// Ticket id (or unique prefix) to mark current.
    pub ticket: Option<String>,

    /// Clear the currently-checked-out ticket.
    #[arg(short = 'c', long = "clear", conflicts_with = "ticket")]
    pub clear: bool,

    /// Output the checked-out ticket as JSON.
    #[arg(long = "json")]
    pub json: bool,
}

pub fn run(args: Args) -> Result<()> {
    let store = open_store()?;
    let git_dir = store.session().repo_git_dir();
    let mut state = State::load().unwrap_or_default();

    if args.clear {
        state.clear_current(&git_dir);
        state.save()?;
        if args.json {
            println!("{}", serde_json::json!({ "current": null }));
            return Ok(());
        }
        println!("Cleared current ticket.");
        return Ok(());
    }

    let reference = args
        .ticket
        .ok_or_else(|| anyhow::anyhow!("ticket id (or prefix) required"))?;
    let id = store.resolve_id(&reference)?;
    state.set_current(&git_dir, id);
    state.save()?;

    let ticket = store.load(&id)?;
    if args.json {
        println!("{}", render::ticket_json(&ticket)?);
        return Ok(());
    }
    println!("Checked out: {} - {}", ticket.short_id(), ticket.title);
    Ok(())
}
