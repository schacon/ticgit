use anyhow::Result;
use clap::Parser;

use crate::commands::{open_store, resolve_ticket};

#[derive(Debug, Parser)]
pub struct Args {
    /// Milestone name. Omit with --clear to remove milestone.
    pub milestone: Option<String>,

    /// Ticket id (or prefix). Defaults to the currently checked-out ticket.
    #[arg(short = 't', long = "ticket")]
    pub ticket: Option<String>,

    /// Clear the current milestone.
    #[arg(short = 'c', long = "clear", conflicts_with = "milestone")]
    pub clear: bool,
}

pub fn run(args: Args) -> Result<()> {
    let store = open_store()?;
    let id = resolve_ticket(&store, args.ticket.as_deref())?;

    if args.clear {
        store.set_milestone(&id, None)?;
    } else {
        let milestone = args
            .milestone
            .ok_or_else(|| anyhow::anyhow!("specify a milestone (or pass --clear)"))?;
        store.set_milestone(&id, Some(&milestone))?;
    }

    let ticket = store.load(&id)?;
    let display = ticket.milestone.as_deref().unwrap_or("(none)");
    println!("{} milestone: {}", ticket.short_id(), display);
    Ok(())
}
