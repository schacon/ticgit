use std::collections::BTreeSet;

use anyhow::Result;
use clap::Parser;
use ticgit_lib::{Filter, SortOrder, TicketState};

use crate::commands::open_store;

#[derive(Debug, Parser)]
pub struct SaveArgs {
    /// View name to write.
    pub name: String,

    /// Include only tickets in this state.
    #[arg(short = 's', long = "state")]
    pub state: Option<String>,

    /// Include only tickets with this tag.
    #[arg(short = 'g', long = "tag")]
    pub tag: Option<String>,

    /// Include only tickets assigned to this user.
    #[arg(short = 'a', long = "assigned")]
    pub assigned: Option<String>,

    /// Include only tickets that have at least one tag.
    #[arg(short = 'T', long = "only-tagged")]
    pub only_tagged: bool,

    /// Sort order before saving. The saved value is still just a set.
    #[arg(short = 'o', long = "order")]
    pub order: Option<String>,
}

#[derive(Debug, Parser)]
pub struct ListArgs {
    /// Show only this view's ticket ids. Without a name, list view names.
    pub name: Option<String>,
}

pub fn run_save(args: SaveArgs) -> Result<()> {
    let store = open_store()?;
    let tickets = store.list()?;
    let state = match args.state.as_deref() {
        Some(s) => Some(TicketState::parse(s)?),
        None => None,
    };
    let order = match args.order.as_deref() {
        Some(spec) => Some(
            SortOrder::parse(spec).ok_or_else(|| anyhow::anyhow!("unknown sort order `{spec}`"))?,
        ),
        None => None,
    };
    let filter = Filter {
        state,
        tag: args.tag,
        assigned: args.assigned,
        only_tagged: args.only_tagged,
        order,
    };
    let ids: BTreeSet<_> = ticgit_lib::query::apply(tickets, &filter)
        .into_iter()
        .map(|t| t.id)
        .collect();
    store.save_view(&args.name, &ids)?;
    println!("Saved view `{}` with {} tickets.", args.name, ids.len());
    Ok(())
}

pub fn run_list(args: ListArgs) -> Result<()> {
    let store = open_store()?;
    if let Some(name) = args.name {
        let ids = store.load_view(&name)?;
        if ids.is_empty() {
            println!("(empty view)");
        } else {
            for id in ids {
                println!("{id}");
            }
        }
        return Ok(());
    }

    let names = store.list_views()?;
    if names.is_empty() {
        println!("(no views)");
    } else {
        for name in names {
            println!("{name}");
        }
    }
    Ok(())
}
