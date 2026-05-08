use anyhow::Result;
use clap::Parser;

use crate::commands::{open_store, resolve_ticket};
use crate::render;

#[derive(Debug, Parser)]
pub struct Args {
    /// Points estimate. Omit with --clear to remove points.
    pub points: Option<i64>,

    /// Ticket id (or prefix). Defaults to the currently checked-out ticket.
    #[arg(short = 't', long = "ticket")]
    pub ticket: Option<String>,

    /// Clear the current points value.
    #[arg(short = 'c', long = "clear", conflicts_with = "points")]
    pub clear: bool,

    /// Output the updated ticket as JSON.
    #[arg(long = "json")]
    pub json: bool,
}

pub fn run(args: Args) -> Result<()> {
    let store = open_store()?;
    let id = resolve_ticket(&store, args.ticket.as_deref())?;

    if args.clear {
        store.set_points(&id, None)?;
    } else {
        let points = args
            .points
            .ok_or_else(|| anyhow::anyhow!("specify points (or pass --clear)"))?;
        store.set_points(&id, Some(points))?;
    }

    let ticket = store.load(&id)?;
    if args.json {
        println!("{}", render::ticket_json(&ticket)?);
        return Ok(());
    }

    let display = ticket
        .points
        .map(|p| p.to_string())
        .unwrap_or_else(|| "(none)".to_string());
    println!("{} points: {}", ticket.short_id(), display);
    Ok(())
}
