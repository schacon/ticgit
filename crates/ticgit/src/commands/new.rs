use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use ticgit_lib::NewTicketOpts;

use crate::commands::{open_store, SessionGitDir};
use crate::editor;
use crate::render;
use crate::session_state::State;

#[derive(Debug, Parser)]
pub struct Args {
    /// Ticket title. If omitted, your `$EDITOR` is opened to write one.
    #[arg(short = 't', long = "title")]
    pub title: Option<String>,

    /// Read the title and description from a file.
    #[arg(short = 'F', long = "file", conflicts_with = "title")]
    pub file: Option<PathBuf>,

    /// Comma- or space-separated list of tags to apply on creation.
    #[arg(short = 'g', long = "tags")]
    pub tags: Option<String>,

    /// Initial assignee.
    #[arg(short = 'a', long = "assigned")]
    pub assigned: Option<String>,

    /// Initial priority (lower = more important).
    #[arg(long = "priority")]
    pub priority: Option<i64>,

    /// Create as a sub-issue of this ticket (id or prefix).
    #[arg(short = 'p', long = "subissue")]
    pub subissue: Option<String>,

    /// Initial comment body. Use `--comment-edit` to compose in `$EDITOR`.
    #[arg(long = "comment")]
    pub comment: Option<String>,

    /// Check out the new ticket after creating it.
    #[arg(short = 'c', long = "checkout")]
    pub checkout: bool,

    /// Open `$EDITOR` to write the initial comment.
    #[arg(long = "comment-edit", conflicts_with = "comment")]
    pub comment_edit: bool,

    /// Don't print the new ticket; just print the new id.
    #[arg(long = "id-only")]
    pub id_only: bool,

    /// Output the created ticket as JSON.
    #[arg(long = "json")]
    pub json: bool,

    /// Output the created ticket as Markdown.
    #[arg(long = "markdown", conflicts_with = "json")]
    pub markdown: bool,
}

pub fn run(args: Args) -> Result<()> {
    let store = open_store()?;

    let (title, description) = if let Some(path) = args.file {
        editor::read_ticket_edit_file(&path)?
    } else {
        let title = match args.title {
            Some(t) if !t.trim().is_empty() => t.trim().to_string(),
            _ => editor::capture("Ticket title")?.context("ticket title cannot be empty")?,
        };
        (title, None)
    };

    let comment = if args.comment_edit {
        editor::capture("Initial comment (lines starting with # are ignored)")?
    } else {
        args.comment
    };

    let tags = parse_tags(args.tags.as_deref());

    let parent = args.subissue.map(|p| store.resolve_id(&p)).transpose()?;

    let opts = NewTicketOpts {
        comment,
        tags,
        assigned: args.assigned,
        parent,
        ..Default::default()
    };
    let mut ticket = store.create(&title, opts)?;
    let mut needs_reload = false;
    if let Some(description) = description {
        store.set_description(&ticket.id, Some(&description))?;
        needs_reload = true;
    }
    if let Some(priority) = args.priority {
        store.set_priority(&ticket.id, Some(priority))?;
        needs_reload = true;
    }
    if needs_reload {
        ticket = store.load(&ticket.id)?;
    }

    if args.checkout {
        let git_dir = store.session().repo_git_dir();
        let mut state = State::load().unwrap_or_default();
        state.set_current(&git_dir, ticket.id);
        state.save()?;
    }

    if args.json {
        println!("{}", render::ticket_json(&ticket)?);
        return Ok(());
    }

    if args.markdown {
        println!("{}", render::ticket_markdown(&ticket));
        return Ok(());
    }

    if args.id_only {
        println!("{}", ticket.id);
    } else {
        println!("Created ticket {} ({})", ticket.short_id(), ticket.title);
        println!("Full id: {}", ticket.id);
        if args.checkout {
            println!("Checked out: {}", ticket.short_id());
        }
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
