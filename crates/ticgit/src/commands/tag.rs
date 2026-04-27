use anyhow::Result;
use clap::Parser;

use crate::commands::{open_store, resolve_ticket};

#[derive(Debug, Parser)]
pub struct Args {
    /// Ticket id (or prefix). Defaults to the currently checked-out ticket.
    #[arg(short = 't', long = "ticket")]
    pub ticket: Option<String>,

    /// Tag(s) to add. Comma- or space-separated.
    #[arg(num_args = 0.., conflicts_with = "remove")]
    pub tags: Vec<String>,

    /// Remove the given tag(s) instead of adding.
    #[arg(short = 'd', long = "remove")]
    pub remove: Vec<String>,
}

pub fn run(args: Args) -> Result<()> {
    let store = open_store()?;
    let id = resolve_ticket(&store, args.ticket.as_deref())?;

    if args.tags.is_empty() && args.remove.is_empty() {
        anyhow::bail!("specify at least one tag to add (or use -d to remove)");
    }

    for raw in &args.tags {
        for t in split_tags(raw) {
            store.add_tag(&id, &t)?;
        }
    }
    for raw in &args.remove {
        for t in split_tags(raw) {
            store.remove_tag(&id, &t)?;
        }
    }

    let ticket = store.load(&id)?;
    let joined: Vec<_> = ticket.tags.iter().cloned().collect();
    println!(
        "Tags on {}: {}",
        ticket.short_id(),
        if joined.is_empty() {
            "(none)".to_string()
        } else {
            joined.join(", ")
        }
    );
    Ok(())
}

fn split_tags(raw: &str) -> Vec<String> {
    raw.split(|c: char| c == ',' || c.is_whitespace())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}
