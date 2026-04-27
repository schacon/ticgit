use anyhow::{Context, Result};
use clap::Parser;

use crate::commands::{open_store, resolve_ticket};
use crate::editor;

#[derive(Debug, Parser)]
pub struct Args {
    /// Ticket id (or prefix). Defaults to the currently checked-out ticket.
    #[arg(short = 't', long = "ticket")]
    pub ticket: Option<String>,

    /// Comment body. If omitted, `$EDITOR` is opened.
    pub body: Vec<String>,

    /// Force opening `$EDITOR`, ignoring positional body text.
    #[arg(short = 'e', long = "edit")]
    pub edit: bool,
}

pub fn run(args: Args) -> Result<()> {
    let store = open_store()?;
    let id = resolve_ticket(&store, args.ticket.as_deref())?;

    let body = if args.edit || args.body.is_empty() {
        editor::capture("Ticket comment (lines starting with # are ignored)")?
            .context("comment cannot be empty")?
    } else {
        args.body.join(" ")
    };

    store.add_comment(&id, &body)?;
    let ticket = store.load(&id)?;
    println!("Added comment to {}.", ticket.short_id());
    Ok(())
}
