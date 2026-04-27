use anyhow::{Context, Result};
use clap::Parser;
use ticgit_lib::NewTicketOpts;

use crate::commands::open_store;
use crate::editor;

#[derive(Debug, Parser)]
pub struct Args {
    /// Ticket title. If omitted, your `$EDITOR` is opened to write one.
    #[arg(short = 't', long = "title")]
    pub title: Option<String>,

    /// Comma- or space-separated list of tags to apply on creation.
    #[arg(short = 'g', long = "tags")]
    pub tags: Option<String>,

    /// Initial assignee.
    #[arg(short = 'a', long = "assigned")]
    pub assigned: Option<String>,

    /// Initial comment body. Use `--comment-edit` to compose in `$EDITOR`.
    #[arg(short = 'c', long = "comment")]
    pub comment: Option<String>,

    /// Open `$EDITOR` to write the initial comment.
    #[arg(long = "comment-edit", conflicts_with = "comment")]
    pub comment_edit: bool,

    /// Don't print the new ticket; just print the new id.
    #[arg(long = "id-only")]
    pub id_only: bool,
}

pub fn run(args: Args) -> Result<()> {
    let store = open_store()?;

    let title = match args.title {
        Some(t) if !t.trim().is_empty() => t.trim().to_string(),
        _ => editor::capture("Ticket title")?.context("ticket title cannot be empty")?,
    };

    let comment = if args.comment_edit {
        editor::capture("Initial comment (lines starting with # are ignored)")?
    } else {
        args.comment
    };

    let tags = parse_tags(args.tags.as_deref());

    let opts = NewTicketOpts {
        comment,
        tags,
        assigned: args.assigned,
    };
    let ticket = store.create(&title, opts)?;

    if args.id_only {
        println!("{}", ticket.id);
    } else {
        println!("Created ticket {} ({})", ticket.short_id(), ticket.title);
        println!("Full id: {}", ticket.id);
    }
    Ok(())
}

fn parse_tags(raw: Option<&str>) -> Vec<String> {
    raw.map(|s| {
        s.split(|c: char| c == ',' || c.is_whitespace())
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect()
    })
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tags_handles_commas_and_spaces() {
        assert_eq!(
            parse_tags(Some("a, b ,c d")),
            vec![
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
                "d".to_string()
            ]
        );
        assert_eq!(parse_tags(Some("")).len(), 0);
        assert_eq!(parse_tags(None).len(), 0);
    }
}
