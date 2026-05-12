use anyhow::Result;
use clap::Parser;
use ticgit_lib::{Filter, SearchFilter, SortOrder, TicketLifecycle, TicketStatus};

use crate::commands::{open_store, SessionGitDir};
use crate::render;
use crate::session_state::{SavedView, State};

#[derive(Debug, Default, Parser)]
pub struct Args {
    /// Load a saved view by name.
    pub view: Option<String>,

    /// Show only tickets in this status/state. Defaults to status open.
    #[arg(short = 's', long = "state")]
    pub state: Option<String>,

    /// Show only tickets in this broad status.
    #[arg(long = "status")]
    pub status: Option<String>,

    /// Show all tickets, without the default open-only filter or limit.
    #[arg(long = "all")]
    pub all: bool,

    /// Show only tickets with this tag.
    #[arg(short = 'g', long = "tag")]
    pub tag: Vec<String>,

    /// How multiple --tag filters combine: all or any.
    #[arg(long = "tag-mode", default_value = "all")]
    pub tag_mode: String,

    /// Show only tickets assigned to this user.
    #[arg(short = 'a', long = "assigned")]
    pub assigned: Option<String>,

    /// Show only tickets that have at least one tag.
    #[arg(short = 'T', long = "only-tagged")]
    pub only_tagged: bool,

    /// Search title, description, and comments. Use `title:term`, `description:term`, or `comments:term` to scope.
    #[arg(long = "search")]
    pub search: Option<String>,

    /// Sort order. e.g. `priority`, `state`, `title.desc`, `created`, `assigned`.
    #[arg(short = 'o', long = "order")]
    pub order: Option<String>,

    /// Include sub-issues (hidden by default).
    #[arg(long = "subissues")]
    pub subissues: bool,

    /// Maximum number of tickets to show.
    #[arg(short = 'n', long = "limit", default_value_t = 20)]
    pub limit: usize,

    /// Output as JSON.
    #[arg(long = "json")]
    pub json: bool,

    /// Output as Markdown.
    #[arg(long = "markdown", conflicts_with = "json")]
    pub markdown: bool,
}

pub fn run(args: Args) -> Result<()> {
    let store = open_store()?;
    let git_dir = store.session().repo_git_dir();
    let tickets = store.list()?;
    let open_ref_lengths = render::open_ticket_ref_lengths(&tickets);

    // If a positional arg is given, try to load it as a saved view.
    let args = if let Some(ref view_name) = args.view {
        let state = State::load().unwrap_or_default();
        let saved = state
            .load_view(&git_dir, view_name)
            .ok_or_else(|| anyhow::anyhow!("no saved view named `{view_name}`"))?;
        Args {
            view: None,
            state: saved.state.clone(),
            status: saved.status.clone(),
            all: saved.all,
            tag: saved_tags(&saved),
            tag_mode: if saved.tag_match_all { "all" } else { "any" }.to_string(),
            assigned: saved.assigned.clone(),
            only_tagged: saved.only_tagged,
            search: saved.search.clone(),
            order: saved.order.clone(),
            subissues: saved.subissues,
            limit: if saved.limit > 0 { saved.limit } else { 20 },
            json: args.json,
            markdown: args.markdown,
        }
    } else {
        args
    };

    let mut status = match args.status.as_deref() {
        Some(s) => Some(TicketStatus::parse(s)?),
        None if args.all || args.state.is_some() => None,
        None => Some(TicketStatus::Open),
    };
    let mut state = None;
    if let Some(spec) = args.state.as_deref() {
        let lifecycle = TicketLifecycle::parse(spec)?;
        status = Some(lifecycle.status);
        if TicketStatus::parse(spec).is_err() {
            state = Some(lifecycle.state);
        }
    }
    let order = match args.order.as_deref() {
        Some(spec) => Some(
            SortOrder::parse(spec).ok_or_else(|| anyhow::anyhow!("unknown sort order `{spec}`"))?,
        ),
        None => None,
    };
    let search = match args.search.as_deref() {
        Some(spec) => Some(SearchFilter::parse(spec).map_err(|e| anyhow::anyhow!(e))?),
        None => None,
    };
    let tag_match_all = match args.tag_mode.as_str() {
        "all" => true,
        "any" | "either" => false,
        other => anyhow::bail!("unknown tag mode `{other}` (expected `all` or `any`)"),
    };

    let filter = Filter {
        status,
        state,
        tag: args.tag.first().cloned(),
        tags: args.tag.clone(),
        tag_match_all,
        assigned: args.assigned.clone(),
        only_tagged: args.only_tagged,
        search,
        order,
        hide_subissues: !args.subissues,
    };
    let mut tickets = ticgit_lib::query::apply(tickets, &filter);
    if !args.all && args.limit > 0 {
        tickets.truncate(args.limit);
    }

    // Save last-used filters so `ti views save` can recall them.
    if args.view.is_none() {
        let saved = SavedView {
            created_at: None,
            status: args.status.clone(),
            state: args.state.clone(),
            tag: args.tag.first().cloned(),
            tags: args.tag.clone(),
            tag_match_all,
            assigned: args.assigned.clone(),
            only_tagged: args.only_tagged,
            search: args.search.clone(),
            order: args.order.clone(),
            all: args.all,
            subissues: args.subissues,
            limit: args.limit,
        };
        if let Ok(mut session_state) = State::load() {
            session_state.set_last_filters(&git_dir, saved);
            let _ = session_state.save();
        }
    }

    if args.json {
        println!("{}", render::tickets_json(&tickets)?);
        return Ok(());
    }

    if args.markdown {
        println!("{}", render::tickets_markdown(&tickets));
        return Ok(());
    }

    if tickets.is_empty() {
        println!("(no tickets)");
        return Ok(());
    }

    let session_state = State::load().unwrap_or_default();
    let current = session_state.current_for(&git_dir);
    let users = store.list_users().unwrap_or_default();
    let nicks = render::build_nick_map(&users);
    println!(
        "{}",
        render::tickets_table_with_refs(
            &tickets,
            current.as_ref(),
            &open_ref_lengths,
            Some(&nicks)
        )
    );
    Ok(())
}

fn saved_tags(saved: &SavedView) -> Vec<String> {
    if !saved.tags.is_empty() {
        return saved.tags.clone();
    }
    saved.tag.iter().cloned().collect()
}
