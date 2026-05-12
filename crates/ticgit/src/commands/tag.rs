use anyhow::Result;
use clap::Parser;

use crate::commands::{open_store, resolve_ticket};
use crate::render;

#[derive(Debug, Parser)]
pub struct Args {
    /// Ticket id (or prefix). Defaults to the currently checked-out ticket.
    #[arg(short = 't', long = "ticket", conflicts_with = "writeup")]
    pub ticket: Option<String>,

    /// Writeup id (or unique prefix) to tag instead of a ticket.
    #[arg(short = 'w', long = "writeup")]
    pub writeup: Option<String>,

    /// Tag(s) to add. Comma- or space-separated.
    #[arg(num_args = 0.., conflicts_with = "remove")]
    pub tags: Vec<String>,

    /// Remove the given tag(s) instead of adding.
    #[arg(short = 'd', long = "remove")]
    pub remove: Vec<String>,

    /// Output the updated item as JSON.
    #[arg(long = "json")]
    pub json: bool,

    /// Output the updated item as Markdown.
    #[arg(long = "markdown", conflicts_with = "json")]
    pub markdown: bool,
}

pub fn run(args: Args) -> Result<()> {
    let store = open_store()?;

    if args.tags.is_empty() && args.remove.is_empty() {
        anyhow::bail!("specify at least one tag to add (or use -d to remove)");
    }

    if let Some(writeup_ref) = args.writeup.as_deref() {
        let id = store.resolve_writeup_id(writeup_ref)?;
        for raw in &args.tags {
            for t in split_tags(raw) {
                store.add_writeup_tag(&id, &t)?;
            }
        }
        for raw in &args.remove {
            for t in split_tags(raw) {
                store.remove_writeup_tag(&id, &t)?;
            }
        }

        let writeup = store.load_writeup(&id)?;
        if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "id": writeup.id,
                    "short_id": writeup.short_id(),
                    "title": writeup.title,
                    "status": writeup.status.as_str(),
                    "tags": writeup.tags,
                }))?
            );
            return Ok(());
        }
        if args.markdown {
            let joined: Vec<_> = writeup.tags.iter().cloned().collect();
            println!("# Writeup: {}", writeup.title);
            println!();
            println!("- Id: `{}`", writeup.id);
            println!("- Short id: `{}`", writeup.short_id());
            println!("- Status: `{}`", writeup.status.as_str());
            println!(
                "- Tags: {}",
                if joined.is_empty() {
                    "(none)".to_string()
                } else {
                    joined.join(", ")
                }
            );
            return Ok(());
        }

        let joined: Vec<_> = writeup.tags.iter().cloned().collect();
        println!(
            "Tags on writeup {}: {}",
            writeup.short_id(),
            if joined.is_empty() {
                "(none)".to_string()
            } else {
                joined.join(", ")
            }
        );
        return Ok(());
    }

    let id = resolve_ticket(&store, args.ticket.as_deref())?;
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
    if args.json {
        println!("{}", render::ticket_json(&ticket)?);
        return Ok(());
    }
    if args.markdown {
        println!("{}", render::ticket_markdown(&ticket));
        return Ok(());
    }

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
