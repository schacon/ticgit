use anyhow::Result;
use clap::Parser;
use ticgit_lib::{Filter, SortOrder, TicketState};

use crate::commands::{open_store, SessionGitDir};
use crate::render;
use crate::session_state::State;

#[derive(Debug, Parser)]
pub struct Args {
    /// Show only tickets in this state.
    #[arg(short = 's', long = "state")]
    pub state: Option<String>,

    /// Show only tickets with this tag.
    #[arg(short = 'g', long = "tag")]
    pub tag: Option<String>,

    /// Show only tickets assigned to this user.
    #[arg(short = 'a', long = "assigned")]
    pub assigned: Option<String>,

    /// Show only tickets that have at least one tag.
    #[arg(short = 'T', long = "only-tagged")]
    pub only_tagged: bool,

    /// Sort order. e.g. `state`, `title.desc`, `created`, `assigned`.
    #[arg(short = 'o', long = "order")]
    pub order: Option<String>,

    /// Show tickets that belong to a saved view.
    #[arg(short = 'V', long = "view")]
    pub view: Option<String>,

    /// Output as JSON.
    #[arg(long = "json")]
    pub json: bool,
}

pub fn run(args: Args) -> Result<()> {
    let store = open_store()?;
    let mut tickets = store.list()?;

    if let Some(view_name) = &args.view {
        let ids = store.load_view(view_name)?;
        tickets.retain(|t| ids.contains(&t.id));
    }

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
    let tickets = ticgit_lib::query::apply(tickets, &filter);

    if args.json {
        println!("{}", render::tickets_json(&tickets)?);
        return Ok(());
    }

    if tickets.is_empty() {
        println!("(no tickets)");
        return Ok(());
    }

    let session_state = State::load().unwrap_or_default();
    let current = session_state.current_for(&store.session().repo_git_dir());
    println!("{}", render::tickets_table(&tickets, current.as_ref()));
    Ok(())
}
