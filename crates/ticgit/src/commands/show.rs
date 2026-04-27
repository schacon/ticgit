use anyhow::Result;
use clap::Parser;

use crate::commands::{open_store, resolve_ticket};
use crate::render;

#[derive(Debug, Parser)]
pub struct Args {
    /// Ticket id (or unique prefix). Defaults to the currently checked-out ticket.
    pub ticket: Option<String>,

    /// Output the ticket as JSON.
    #[arg(long = "json")]
    pub json: bool,
}

pub fn run(args: Args) -> Result<()> {
    let store = open_store()?;
    let id = resolve_ticket(&store, args.ticket.as_deref())?;
    let ticket = store.load(&id)?;

    if args.json {
        println!("{}", render::ticket_json(&ticket)?);
    } else {
        print!("{}", render::ticket_detail(&ticket));
    }
    Ok(())
}
