use anyhow::Result;
use clap::Parser;
use ticgit_lib::TicketState;

use crate::commands::{open_store, resolve_ticket};

#[derive(Debug, Parser)]
pub struct Args {
    /// New state: open, resolved, invalid, or hold.
    pub state: String,

    /// Ticket id (or prefix). Defaults to the currently checked-out ticket.
    #[arg(short = 't', long = "ticket")]
    pub ticket: Option<String>,
}

pub fn run(args: Args) -> Result<()> {
    let store = open_store()?;
    let id = resolve_ticket(&store, args.ticket.as_deref())?;
    let new_state = TicketState::parse(&args.state)?;
    store.set_state(&id, new_state)?;
    let ticket = store.load(&id)?;
    println!("{} -> {}", ticket.short_id(), ticket.state);
    Ok(())
}
