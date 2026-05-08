use anyhow::{Context, Result};
use clap::Parser;
use ticgit_lib::Ticket;

use crate::commands::{open_store, resolve_ticket};
use crate::editor;
use crate::render;

#[derive(Debug, Parser)]
pub struct Args {
    /// Ticket id (or prefix). Defaults to the currently checked-out ticket.
    pub ticket: Option<String>,

    /// Output the updated ticket as JSON.
    #[arg(long = "json")]
    pub json: bool,
}

pub fn run(args: Args) -> Result<()> {
    let store = open_store()?;
    let id = resolve_ticket(&store, args.ticket.as_deref())?;
    let ticket = store.load(&id)?;

    let edited = editor::capture_with_initial(
        "Edit the title on the first line. Remaining non-comment lines become the description.",
        &editor_body(&ticket),
    )?
    .context("ticket title cannot be empty")?;
    let (title, description) = parse_ticket_edit(&edited)?;

    store.set_title(&id, &title)?;
    store.set_description(&id, description.as_deref())?;

    let ticket = store.load(&id)?;
    if args.json {
        println!("{}", render::ticket_json(&ticket)?);
        return Ok(());
    }
    println!("Updated {}.", ticket.short_id());
    Ok(())
}

fn editor_body(ticket: &Ticket) -> String {
    let mut body = ticket.title.clone();
    if let Some(description) = &ticket.description {
        body.push_str("\n\n");
        body.push_str(description);
    }
    body
}

fn parse_ticket_edit(raw: &str) -> Result<(String, Option<String>)> {
    let mut lines = raw.lines();
    let title = lines.next().unwrap_or_default().trim().to_string();
    if title.is_empty() {
        anyhow::bail!("ticket title cannot be empty");
    }

    let description = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    let description = if description.is_empty() {
        None
    } else {
        Some(description)
    };

    Ok((title, description))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ticket_edit_splits_title_and_description() {
        let (title, description) =
            parse_ticket_edit("updated title\n\nfirst line\nsecond line\n").unwrap();

        assert_eq!(title, "updated title");
        assert_eq!(description.as_deref(), Some("first line\nsecond line"));
    }

    #[test]
    fn parse_ticket_edit_allows_clearing_description() {
        let (title, description) = parse_ticket_edit("updated title\n\n").unwrap();

        assert_eq!(title, "updated title");
        assert_eq!(description, None);
    }

    #[test]
    fn parse_ticket_edit_rejects_empty_title() {
        assert!(parse_ticket_edit("\nbody").is_err());
    }
}
