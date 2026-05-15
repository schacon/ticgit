use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use ticgit_lib::{NewWriteupOpts, Writeup, WriteupStatus};
use time::format_description::well_known::Rfc3339;

use crate::commands::open_store;
use crate::editor;

#[derive(Debug, Parser)]
pub struct Args {
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Create a new writeup.
    New(NewArgs),
    /// List writeups.
    List(ListArgs),
    /// Show a writeup.
    Show(ShowArgs),
    /// Append a new version to a writeup.
    Edit(EditArgs),
    /// Promote a writeup into a ticket.
    Promote(PromoteArgs),
    /// Close a writeup.
    Close(IdArgs),
    /// Archive a writeup (alias for close).
    Archive(IdArgs),
    /// Link a writeup to an existing ticket.
    Link(LinkArgs),
    /// Remove a writeup-ticket link.
    Unlink(LinkArgs),
}

#[derive(Debug, Parser)]
pub struct NewArgs {
    /// Writeup title. If omitted, your `$EDITOR` is opened to write one.
    #[arg(short = 't', long = "title")]
    pub title: Option<String>,

    /// Initial writeup body.
    #[arg(long = "body", conflicts_with = "file")]
    pub body: Option<String>,

    /// Read the initial writeup body from a file.
    #[arg(short = 'F', long = "file", conflicts_with = "body")]
    pub file: Option<PathBuf>,

    /// Comma- or space-separated list of tags to apply on creation.
    #[arg(short = 'g', long = "tags")]
    pub tags: Option<String>,

    /// Don't print the new writeup; just print the new id.
    #[arg(long = "id-only")]
    pub id_only: bool,
}

#[derive(Debug, Parser)]
pub struct ListArgs {
    /// Include closed writeups.
    #[arg(long = "all")]
    pub all: bool,
}

#[derive(Debug, Parser)]
pub struct ShowArgs {
    /// Writeup id or unique prefix.
    pub id: String,

    /// Show every version instead of only the latest version.
    #[arg(long = "all")]
    pub all: bool,
}

#[derive(Debug, Parser)]
pub struct EditArgs {
    /// Writeup id or unique prefix.
    pub id: String,

    /// New version body.
    #[arg(long = "body", conflicts_with = "file")]
    pub body: Option<String>,

    /// Read the new version body from a file.
    #[arg(short = 'F', long = "file", conflicts_with = "body")]
    pub file: Option<PathBuf>,
}

#[derive(Debug, Parser)]
pub struct PromoteArgs {
    /// Writeup id or unique prefix.
    pub id: String,
}

#[derive(Debug, Parser)]
pub struct IdArgs {
    /// Writeup id or unique prefix.
    pub id: String,
}

#[derive(Debug, Parser)]
pub struct LinkArgs {
    /// Writeup id or unique prefix.
    pub writeup: String,
    /// Ticket id or unique prefix.
    pub ticket: String,
}

pub fn run(args: Args) -> Result<()> {
    match args.action {
        Action::New(args) => run_new(args),
        Action::List(args) => run_list(args),
        Action::Show(args) => run_show(args),
        Action::Edit(args) => run_edit(args),
        Action::Promote(args) => run_promote(args),
        Action::Close(args) | Action::Archive(args) => run_close(args),
        Action::Link(args) => run_link(args),
        Action::Unlink(args) => run_unlink(args),
    }
}

fn run_new(args: NewArgs) -> Result<()> {
    let store = open_store()?;
    let title = match args.title {
        Some(title) if !title.trim().is_empty() => title.trim().to_string(),
        _ => editor::capture("Writeup title")?.context("writeup title cannot be empty")?,
    };
    let body = body_from_args(args.body, args.file, "Writeup body")?;
    let writeup = store.create_writeup(
        &title,
        NewWriteupOpts {
            body,
            tags: parse_tags(args.tags.as_deref()),
            ..Default::default()
        },
    )?;

    if args.id_only {
        println!("{}", writeup.id);
    } else {
        println!("Created writeup {} ({})", writeup.short_id(), writeup.title);
        println!("Full id: {}", writeup.id);
    }
    Ok(())
}

fn run_list(args: ListArgs) -> Result<()> {
    let store = open_store()?;
    let writeups = store.list_writeups()?;
    let mut shown = 0;
    for writeup in writeups {
        if !args.all && writeup.status == WriteupStatus::Closed {
            continue;
        }
        shown += 1;
        let tags = if writeup.tags.is_empty() {
            String::new()
        } else {
            format!(
                " [{}]",
                writeup.tags.iter().cloned().collect::<Vec<_>>().join(",")
            )
        };
        println!(
            "{} {:<6} {:<3} {:<6} v{} {}{}",
            writeup.short_id(),
            writeup.status.as_str(),
            writeup
                .priority
                .map(|priority| format!("p{priority}"))
                .unwrap_or_else(|| "-".to_string()),
            writeup.authors.len(),
            writeup.versions.len(),
            writeup.title,
            tags
        );
    }
    if shown == 0 {
        println!("(no writeups)");
    }
    Ok(())
}

fn run_show(args: ShowArgs) -> Result<()> {
    let store = open_store()?;
    let id = store.resolve_writeup_id(&args.id)?;
    let writeup = store.load_writeup(&id)?;
    print_writeup(&writeup, args.all)
}

fn run_edit(args: EditArgs) -> Result<()> {
    let store = open_store()?;
    let id = store.resolve_writeup_id(&args.id)?;
    let current = store.load_writeup(&id)?;
    let initial = writeup_edit_body(&current);
    let body = match (args.body, args.file) {
        (Some(body), None) => {
            store.append_writeup_version(&id, &body)?;
            let writeup = store.load_writeup(&id)?;
            println!(
                "Appended version {} to writeup {}.",
                writeup.versions.len(),
                writeup.short_id()
            );
            return Ok(());
        }
        (None, Some(path)) => std::fs::read_to_string(&path)
            .with_context(|| format!("reading writeup body from `{}`", path.display()))?,
        (None, None) => editor::capture_with_initial("Writeup body", &initial)?
            .context("writeup edit cancelled")?,
        (Some(_), Some(_)) => unreachable!("clap enforces conflicts"),
    };
    let (title, body) = editor::parse_ticket_edit(&body)?;
    store.set_writeup_title(&id, &title)?;
    let appended = body.is_some();
    if let Some(body) = body {
        store.append_writeup_version(&id, &body)?;
    }
    let writeup = store.load_writeup(&id)?;
    if appended {
        println!(
            "Appended version {} to writeup {}.",
            writeup.versions.len(),
            writeup.short_id()
        );
    } else {
        println!("Updated writeup {}.", writeup.short_id());
    }
    Ok(())
}

fn run_promote(args: PromoteArgs) -> Result<()> {
    let store = open_store()?;
    let id = store.resolve_writeup_id(&args.id)?;
    let ticket = store.promote_writeup(&id)?;
    println!(
        "Promoted writeup {} to ticket {} ({})",
        &id.to_string()[..6],
        ticket.short_id(),
        ticket.title
    );
    println!("Full ticket id: {}", ticket.id);
    Ok(())
}

fn run_close(args: IdArgs) -> Result<()> {
    let store = open_store()?;
    let id = store.resolve_writeup_id(&args.id)?;
    store.set_writeup_status(&id, WriteupStatus::Closed)?;
    println!("Closed writeup {}.", &id.to_string()[..6]);
    Ok(())
}

fn run_link(args: LinkArgs) -> Result<()> {
    let store = open_store()?;
    let writeup_id = store.resolve_writeup_id(&args.writeup)?;
    let ticket_id = store.resolve_id(&args.ticket)?;
    store.link_writeup_ticket(&writeup_id, &ticket_id)?;
    println!(
        "Linked writeup {} to ticket {}.",
        &writeup_id.to_string()[..6],
        &ticket_id.to_string()[..6]
    );
    Ok(())
}

fn run_unlink(args: LinkArgs) -> Result<()> {
    let store = open_store()?;
    let writeup_id = store.resolve_writeup_id(&args.writeup)?;
    let ticket_id = store.resolve_id(&args.ticket)?;
    store.unlink_writeup_ticket(&writeup_id, &ticket_id)?;
    println!(
        "Unlinked writeup {} from ticket {}.",
        &writeup_id.to_string()[..6],
        &ticket_id.to_string()[..6]
    );
    Ok(())
}

fn print_writeup(writeup: &Writeup, all: bool) -> Result<()> {
    println!("# Writeup: {}", writeup.title);
    println!();
    println!("- Id: `{}`", writeup.id);
    println!("- Short id: `{}`", writeup.short_id());
    println!("- Status: `{}`", writeup.status.as_str());
    if let Some(priority) = writeup.priority {
        println!("- Priority: `{priority}`");
    }
    println!(
        "- Created: `{}` by {}",
        writeup.created_at.format(&Rfc3339)?,
        writeup.created_by
    );
    println!(
        "- Authors: {}",
        display_list(writeup.authors.iter().map(String::as_str))
    );
    println!(
        "- Tags: {}",
        display_list(writeup.tags.iter().map(String::as_str))
    );
    let ticket_ids = writeup
        .tickets
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>();
    println!(
        "- Tickets: {}",
        display_list(ticket_ids.iter().map(String::as_str))
    );
    println!("- Versions: {}", writeup.versions.len());
    println!();

    if all {
        for (index, version) in writeup.versions.iter().enumerate() {
            println!("## Version {}", index + 1);
            println!();
            println!("- Author: {}", version.author);
            println!("- Date: `{}`", version.at.format(&Rfc3339)?);
            println!();
            println!("{}", version.body);
            println!();
        }
    } else if let Some(version) = writeup.versions.last() {
        println!("## Latest Version");
        println!();
        println!("- Author: {}", version.author);
        println!("- Date: `{}`", version.at.format(&Rfc3339)?);
        println!();
        println!("{}", version.body);
    } else {
        println!("_No versions yet._");
    }
    Ok(())
}

fn writeup_edit_body(writeup: &Writeup) -> String {
    let mut body = writeup.title.clone();
    if let Some(latest_body) = writeup.latest_body() {
        body.push_str("\n\n");
        body.push_str(latest_body);
    }
    body
}

fn body_from_args(
    body: Option<String>,
    file: Option<PathBuf>,
    prompt: &str,
) -> Result<Option<String>> {
    match (body, file) {
        (Some(body), None) => Ok(Some(body)),
        (None, Some(path)) => {
            Ok(Some(std::fs::read_to_string(&path).with_context(|| {
                format!("reading writeup body from `{}`", path.display())
            })?))
        }
        (None, None) => Ok(editor::capture(prompt)?),
        (Some(_), Some(_)) => unreachable!("clap enforces conflicts"),
    }
}

fn display_list<'a>(items: impl Iterator<Item = &'a str>) -> String {
    let items = items.filter(|item| !item.is_empty()).collect::<Vec<_>>();
    if items.is_empty() {
        "none".to_string()
    } else {
        items.join(", ")
    }
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
