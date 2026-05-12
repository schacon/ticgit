use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::commands::{open_store, SessionGitDir};
use crate::session_state::State;

#[derive(Debug, Parser)]
pub struct Args {
    #[command(subcommand)]
    pub action: Option<Action>,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Save the last-used list filters as a named view.
    Save(SaveArgs),
    /// Delete a saved view.
    Delete(DeleteArgs),
}

#[derive(Debug, Parser)]
pub struct SaveArgs {
    /// View name.
    pub name: String,
}

#[derive(Debug, Parser)]
pub struct DeleteArgs {
    /// View name.
    pub name: String,
}

pub fn run(args: Args) -> Result<()> {
    match args.action {
        None => run_list(),
        Some(Action::Save(a)) => run_save(a),
        Some(Action::Delete(a)) => run_delete(a),
    }
}

fn run_list() -> Result<()> {
    let store = open_store()?;
    let git_dir = store.session().repo_git_dir();
    let state = State::load().unwrap_or_default();
    let views = state.list_views(&git_dir);
    if views.is_empty() {
        println!("(no saved views)");
        println!("Run `ti list` with filters, then `ti views save <name>` to save.");
    } else {
        for (name, view) in &views {
            println!("  {} {}", name, describe_view(view));
        }
    }
    Ok(())
}

fn run_save(args: SaveArgs) -> Result<()> {
    let store = open_store()?;
    let git_dir = store.session().repo_git_dir();
    let mut state = State::load().unwrap_or_default();
    let last = state.last_filters_for(&git_dir).cloned().ok_or_else(|| {
        anyhow::anyhow!("no recent `ti list` filters to save — run `ti list` with filters first")
    })?;
    let desc = describe_view(&last);
    state.save_view(&git_dir, &args.name, last);
    state.save()?;
    println!("Saved view `{}`: {}", args.name, desc);
    Ok(())
}

fn run_delete(args: DeleteArgs) -> Result<()> {
    let store = open_store()?;
    let git_dir = store.session().repo_git_dir();
    let mut state = State::load().unwrap_or_default();
    if state.delete_view(&git_dir, &args.name) {
        state.save()?;
        println!("Deleted view `{}`.", args.name);
    } else {
        anyhow::bail!("no view named `{}`", args.name);
    }
    Ok(())
}

use crate::session_state::SavedView;

pub fn describe_view(v: &SavedView) -> String {
    let mut parts = Vec::new();
    if v.all {
        parts.push("--all".to_string());
    }
    if let Some(s) = &v.status {
        parts.push(format!("--status {s}"));
    }
    if let Some(s) = &v.state {
        parts.push(format!("--state {s}"));
    }
    let tags = saved_tags(v);
    for tag in &tags {
        parts.push(format!("--tag {tag}"));
    }
    if tags.len() > 1 {
        parts.push(format!(
            "--tag-mode {}",
            if v.tag_match_all { "all" } else { "any" }
        ));
    }
    if let Some(a) = &v.assigned {
        parts.push(format!("--assigned {a}"));
    }
    if v.only_tagged {
        parts.push("--only-tagged".to_string());
    }
    if let Some(s) = &v.search {
        parts.push(format!("--search {s}"));
    }
    if let Some(o) = &v.order {
        parts.push(format!("--order {o}"));
    }
    if v.subissues {
        parts.push("--subissues".to_string());
    }
    if v.limit > 0 && v.limit != 20 {
        parts.push(format!("--limit {}", v.limit));
    }
    if parts.is_empty() {
        "(no filters)".to_string()
    } else {
        parts.join(" ")
    }
}

fn saved_tags(v: &SavedView) -> Vec<String> {
    if !v.tags.is_empty() {
        return v.tags.clone();
    }
    v.tag.iter().cloned().collect()
}
