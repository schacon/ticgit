use anyhow::Result;
use clap::Parser;

use crate::commands::{open_store, resolve_ticket};
use crate::render;

#[derive(Debug, Parser)]
pub struct Args {
    /// Ticket to make a sub-issue. Defaults to the currently checked-out ticket.
    #[arg(short = 't', long = "ticket")]
    pub ticket: Option<String>,

    /// Parent ticket id (or prefix). Omit with --clear to remove.
    pub parent: Option<String>,

    /// Remove this ticket's sub-issue relationship.
    #[arg(short = 'c', long = "clear", conflicts_with = "parent")]
    pub clear: bool,

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

    if args.clear {
        store.clear_parent(&id)?;
    } else {
        let parent_ref = args
            .parent
            .ok_or_else(|| anyhow::anyhow!("specify a parent ticket id (or pass --clear)"))?;
        let parent_id = store.resolve_id(&parent_ref)?;
        store.set_parent(&id, &parent_id)?;
    }

    let ticket = store.load(&id)?;
    if args.json {
        println!("{}", render::ticket_json(&ticket)?);
        return Ok(());
    }
    if args.markdown {
        println!("{}", render::ticket_markdown(&ticket));
        return Ok(());
    }

    match ticket.parent {
        Some(pid) => {
            let parent = store.load(&pid)?;
            let short: String = pid.to_string().chars().take(6).collect();
            println!(
                "{} sub-issue of: {} {}",
                ticket.short_id(),
                short,
                parent.title
            );
        }
        None => println!("{} sub-issue: (none)", ticket.short_id()),
    }
    Ok(())
}
