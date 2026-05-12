use anyhow::Result;
use clap::Parser;
use ticgit_lib::{TicketState, TicketStatus};

use crate::commands::{open_store, resolve_ticket};
use crate::render;

#[derive(Debug, Parser)]
pub struct Args {
    /// Ticket id (or prefix). Defaults to the currently checked-out ticket.
    #[arg(short = 't', long = "ticket")]
    pub ticket: Option<String>,

    /// Output the updated ticket as JSON.
    #[arg(long = "json")]
    pub json: bool,

    /// Output the updated ticket as Markdown.
    #[arg(long = "markdown", conflicts_with = "json")]
    pub markdown: bool,
}

pub fn run(args: Args) -> Result<()> {
    let store = open_store()?;
    let id = resolve_ticket(&store, args.ticket.as_deref())?;
    let email = store.email().to_string();

    store.set_assigned(&id, Some(&email))?;
    store.set_lifecycle(&id, TicketStatus::Open, TicketState::Assigned)?;

    let ticket = store.load(&id)?;
    if args.json {
        println!("{}", render::ticket_json(&ticket)?);
        return Ok(());
    }
    if args.markdown {
        println!("{}", render::ticket_markdown(&ticket));
        return Ok(());
    }

    println!("{} claimed by {}", ticket.short_id(), email);
    Ok(())
}
