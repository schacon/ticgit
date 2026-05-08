use anyhow::Result;
use clap::Parser;

use crate::commands::{open_store, resolve_ticket};
use crate::render;

#[derive(Debug, Parser)]
pub struct Args {
    /// User to assign (typically an email). Omit with --clear to unassign.
    pub user: Option<String>,

    /// Ticket id (or prefix). Defaults to the currently checked-out ticket.
    #[arg(short = 't', long = "ticket")]
    pub ticket: Option<String>,

    /// Clear the current assignee.
    #[arg(short = 'c', long = "clear", conflicts_with = "user")]
    pub clear: bool,

    /// Output the updated ticket as JSON.
    #[arg(long = "json")]
    pub json: bool,
}

pub fn run(args: Args) -> Result<()> {
    let store = open_store()?;
    let id = resolve_ticket(&store, args.ticket.as_deref())?;

    if args.clear {
        store.set_assigned(&id, None)?;
    } else {
        let user = args
            .user
            .ok_or_else(|| anyhow::anyhow!("specify a user (or pass --clear)"))?;
        store.set_assigned(&id, Some(&user))?;
    }

    let ticket = store.load(&id)?;
    if args.json {
        println!("{}", render::ticket_json(&ticket)?);
        return Ok(());
    }

    let display = ticket.assigned.as_deref().unwrap_or("(unassigned)");
    println!("{} assigned: {}", ticket.short_id(), display);
    Ok(())
}
