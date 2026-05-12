use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

use anyhow::{Context, Result};
use clap::Parser;
use ticgit_lib::{Ticket, TicketStore};
use uuid::Uuid;

use crate::commands::open_store;

#[derive(Debug, Parser)]
pub struct Args {
    /// URL or saved nickname of the fork to pull tickets from.
    pub source: String,

    /// Save the URL under this nickname for future use.
    pub nickname: Option<String>,

    /// Output the pull summary as JSON.
    #[arg(long = "json")]
    pub json: bool,

    /// Output the pull summary as Markdown.
    #[arg(long = "markdown", conflicts_with = "json")]
    pub markdown: bool,
}

pub fn run(args: Args) -> Result<()> {
    let store = open_store()?;

    // Resolve source to a URL (check nicknames first, then treat as URL).
    let url = resolve_source(&args.source)?;

    // If a nickname was given, save it.
    if let Some(ref nick) = args.nickname {
        save_nickname(nick, &url)?;
        println!("Saved remote \"{nick}\" → {url}");
    }

    // Fetch into a temp bare repo and open a store on it.
    let remote_tickets = fetch_remote_tickets(&url)?;

    if remote_tickets.is_empty() {
        println!("No tickets found on remote.");
        return Ok(());
    }

    // Load local tickets indexed by UUID.
    let local_tickets: BTreeMap<Uuid, Ticket> =
        store.list()?.into_iter().map(|t| (t.id, t)).collect();

    let mut imported = Vec::new();
    let mut updated = Vec::new();

    for remote in &remote_tickets {
        match local_tickets.get(&remote.id) {
            None => {
                // New ticket — import it.
                import_ticket(&store, remote)?;
                imported.push(remote.clone());
            }
            Some(local) => {
                // Existing ticket — merge modifications.
                if merge_ticket(&store, local, remote)? {
                    updated.push(remote.clone());
                }
            }
        }
    }

    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "source": url,
                "imported": imported.len(),
                "updated": updated.len(),
                "imported_tickets": imported,
                "updated_tickets": updated,
            })
        );
        return Ok(());
    }

    if args.markdown {
        println!("{}", pull_markdown(&url, &imported, &updated));
        return Ok(());
    }

    println!("Pulled from: {url}");
    if imported.is_empty() && updated.is_empty() {
        println!("No changes.");
    } else {
        if !imported.is_empty() {
            println!("Imported {} new ticket(s):", imported.len());
            for t in imported.iter().take(10) {
                println!("  {} {}", t.short_id(), t.title);
            }
            if imported.len() > 10 {
                println!("  ... and {} more", imported.len() - 10);
            }
        }
        if !updated.is_empty() {
            println!("Updated {} ticket(s):", updated.len());
            for t in updated.iter().take(10) {
                println!("  {} {}", t.short_id(), t.title);
            }
            if updated.len() > 10 {
                println!("  ... and {} more", updated.len() - 10);
            }
        }
    }

    Ok(())
}

/// Resolve a source string to a URL. Checks `.git/config` for a saved
/// nickname first, then treats the string as a literal URL.
fn resolve_source(source: &str) -> Result<String> {
    // Try as a nickname first.
    let key = format!("ticgit.remotes.{source}.url");
    if let Some(url) = git_config_get(&key)? {
        return Ok(url);
    }
    // Treat as URL if it looks like one (contains :// or : for SSH).
    if source.contains("://") || source.contains(':') || source.starts_with('/') {
        return Ok(source.to_string());
    }
    anyhow::bail!(
        "unknown remote \"{source}\" — use a URL or save one first with: ti pull <url> {source}"
    );
}

/// Save a nickname → URL mapping in `.git/config`.
fn save_nickname(nick: &str, url: &str) -> Result<()> {
    let key = format!("ticgit.remotes.{nick}.url");
    git_run(&["config", "--local", &key, url])
}

/// Set up a temp repo with the URL as a remote, pull git-meta data
/// from it, and return all tickets found.
fn fetch_remote_tickets(url: &str) -> Result<Vec<Ticket>> {
    let tmpdir = tempfile::tempdir().context("creating temp dir for fetch")?;
    let path = tmpdir.path();

    // Init a normal repo (git-meta needs a non-bare repo for its sqlite db).
    git_at(path, &["init", "--quiet", "-b", "main"])?;
    git_at(path, &["config", "user.email", "pull@ticgit.dev"])?;
    git_at(path, &["config", "user.name", "ticgit-pull"])?;
    git_at(path, &["commit", "--allow-empty", "-m", "init", "--quiet"])?;

    // Configure a remote with git-meta refspecs so session.pull() works.
    git_at(path, &["remote", "add", "origin", url])?;
    let namespace = meta_namespace()?;
    let fetch_refspec = format!("+refs/{namespace}/main:refs/{namespace}/remotes/main");
    git_at(
        path,
        &["config", "--add", "remote.origin.fetch", &fetch_refspec],
    )?;
    git_at(path, &["config", "remote.origin.meta", "true"])?;

    // Open a store on the temp repo and pull from the remote.
    let repo = gix::open(path).context("opening temp repo")?;
    let session = ticgit_lib::Session::open(repo.path()).context("opening session on temp repo")?;
    let remote_store =
        TicketStore::from_session(session).context("opening ticket store on temp repo")?;

    // Pull populates the sqlite database from the remote's refs.
    remote_store
        .pull(Some("origin"))
        .context("pulling metadata from remote")?;

    remote_store.list().context("listing tickets from remote")
}

fn meta_namespace() -> Result<String> {
    Ok(git_config_get("meta.namespace")?.unwrap_or_else(|| "meta".to_string()))
}

/// Import a brand-new ticket from the remote into the local store.
fn import_ticket(store: &TicketStore, remote: &Ticket) -> Result<()> {
    let p = store.session().target(&ticgit_lib::Target::project());

    // Write all fields directly using the remote's UUID.
    let id = &remote.id;
    p.set(
        &ticgit_lib::keys::ticket_field(id, "title"),
        remote.title.as_str(),
    )?;
    p.set(
        &ticgit_lib::keys::ticket_field(id, "status"),
        remote.status.as_str(),
    )?;
    p.set(
        &ticgit_lib::keys::ticket_field(id, "state"),
        remote.state.as_str(),
    )?;
    let created_at_str = remote
        .created_at
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    p.set(
        &ticgit_lib::keys::ticket_field(id, "created-at"),
        created_at_str.as_str(),
    )?;
    p.set(
        &ticgit_lib::keys::ticket_field(id, "created-by"),
        remote.created_by.as_str(),
    )?;

    if let Some(ref desc) = remote.description {
        p.set(
            &ticgit_lib::keys::ticket_field(id, "description"),
            desc.as_str(),
        )?;
    }
    if let Some(ref spec) = remote.spec {
        p.set(&ticgit_lib::keys::ticket_field(id, "spec"), spec.as_str())?;
    }
    if let Some(ref assigned) = remote.assigned {
        p.set(
            &ticgit_lib::keys::ticket_field(id, "assigned"),
            assigned.as_str(),
        )?;
    }
    if let Some(ref closed_by) = remote.closed_by {
        p.set(
            &ticgit_lib::keys::ticket_field(id, "closed-by"),
            closed_by.as_str(),
        )?;
    }
    if let Some(points) = remote.points {
        let pts = points.to_string();
        p.set(&ticgit_lib::keys::ticket_field(id, "points"), pts.as_str())?;
    }
    if let Some(ref milestone) = remote.milestone {
        p.set(
            &ticgit_lib::keys::ticket_field(id, "milestone"),
            milestone.as_str(),
        )?;
    }
    if let Some(ref code) = remote.code {
        p.set(&ticgit_lib::keys::ticket_field(id, "code"), code.as_str())?;
    }
    if let Some(ref parent_id) = remote.parent {
        let pid = parent_id.to_string();
        p.set(&ticgit_lib::keys::ticket_field(id, "parent"), pid.as_str())?;
    }

    for tag in &remote.tags {
        p.set_add(&ticgit_lib::keys::ticket_field(id, "tags"), tag.as_str())?;
    }

    for child_id in &remote.children {
        let cid = child_id.to_string();
        p.set_add(
            &ticgit_lib::keys::ticket_field(id, "children"),
            cid.as_str(),
        )?;
    }

    for (key, value) in &remote.meta {
        p.set(
            &ticgit_lib::keys::ticket_meta_field(id, key),
            value.as_str(),
        )?;
    }

    // Comments: push each one as raw JSON to preserve authorship and timestamp.
    for comment in &remote.comments {
        let payload = serde_json::json!({
            "author": comment.author,
            "body": comment.body,
        });
        let json = payload.to_string();
        p.list_push(
            &ticgit_lib::keys::ticket_field(id, "comments"),
            json.as_str(),
        )?;
    }

    Ok(())
}

/// Merge modifications from a remote ticket into a local ticket.
/// Returns true if any changes were made.
fn merge_ticket(store: &TicketStore, local: &Ticket, remote: &Ticket) -> Result<bool> {
    let id = &local.id;
    let mut changed = false;

    // Scalar fields: fork wins if different.
    if remote.title != local.title {
        store.set_title(id, &remote.title)?;
        changed = true;
    }
    if remote.description != local.description {
        store.set_description(id, remote.description.as_deref())?;
        changed = true;
    }
    if remote.spec != local.spec {
        store.set_spec(id, remote.spec.as_deref())?;
        changed = true;
    }
    if remote.state != local.state {
        store.set_state(id, remote.state)?;
        changed = true;
    }
    if remote.assigned != local.assigned {
        store.set_assigned(id, remote.assigned.as_deref())?;
        changed = true;
    }
    if remote.closed_by != local.closed_by
        || (remote.status == ticgit_lib::TicketStatus::Closed && remote.closed_by.is_none())
    {
        store.set_closed_by(id, remote.closed_by.as_deref())?;
        changed = true;
    }
    if remote.points != local.points {
        store.set_points(id, remote.points)?;
        changed = true;
    }
    if remote.milestone != local.milestone {
        store.set_milestone(id, remote.milestone.as_deref())?;
        changed = true;
    }
    if remote.code != local.code {
        store.set_code(id, remote.code.as_deref())?;
        changed = true;
    }

    // Tags: union (add remote tags not in local).
    for tag in &remote.tags {
        if !local.tags.contains(tag) {
            store.add_tag(id, tag)?;
            changed = true;
        }
    }

    // Meta: add/overwrite remote meta keys.
    for (key, value) in &remote.meta {
        if local.meta.get(key) != Some(value) {
            store.set_meta(id, key, value)?;
            changed = true;
        }
    }

    // Parent: take remote's if different (but don't clear if remote has none).
    if remote.parent.is_some() && remote.parent != local.parent {
        let parent_id = remote.parent.unwrap();
        let p = store.session().target(&ticgit_lib::Target::project());
        let pid = parent_id.to_string();
        p.set(&ticgit_lib::keys::ticket_field(id, "parent"), pid.as_str())?;
        changed = true;
    }

    // Children: union.
    for child_id in &remote.children {
        if !local.children.contains(child_id) {
            let p = store.session().target(&ticgit_lib::Target::project());
            let cid = child_id.to_string();
            p.set_add(
                &ticgit_lib::keys::ticket_field(id, "children"),
                cid.as_str(),
            )?;
            changed = true;
        }
    }

    // Comments: deduplicate by author+body, append new ones.
    let local_comment_keys: BTreeSet<(String, String)> = local
        .comments
        .iter()
        .map(|c| (c.author.clone(), c.body.clone()))
        .collect();

    for comment in &remote.comments {
        let key = (comment.author.clone(), comment.body.clone());
        if !local_comment_keys.contains(&key) {
            let payload = serde_json::json!({
                "author": comment.author,
                "body": comment.body,
            });
            let json = payload.to_string();
            let p = store.session().target(&ticgit_lib::Target::project());
            p.list_push(
                &ticgit_lib::keys::ticket_field(id, "comments"),
                json.as_str(),
            )?;
            changed = true;
        }
    }

    Ok(changed)
}

fn pull_markdown(url: &str, imported: &[Ticket], updated: &[Ticket]) -> String {
    let mut out = String::new();
    out.push_str(&format!("## Pull from {url}\n\n"));
    if imported.is_empty() && updated.is_empty() {
        out.push_str("No changes.\n");
        return out;
    }
    if !imported.is_empty() {
        out.push_str(&format!(
            "### Imported {} new ticket(s)\n\n",
            imported.len()
        ));
        for t in imported {
            out.push_str(&format!("- **{}** {}\n", t.short_id(), t.title));
        }
        out.push('\n');
    }
    if !updated.is_empty() {
        out.push_str(&format!("### Updated {} ticket(s)\n\n", updated.len()));
        for t in updated {
            out.push_str(&format!("- **{}** {}\n", t.short_id(), t.title));
        }
        out.push('\n');
    }
    out
}

fn git_config_get(key: &str) -> Result<Option<String>> {
    let output = Command::new("git")
        .args(["config", "--get", key])
        .output()
        .with_context(|| format!("running git config --get {key}"))?;
    if output.status.success() {
        Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        ))
    } else if output.status.code() == Some(1) {
        Ok(None)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git config --get {key} failed: {}", stderr.trim());
    }
}

fn git_run(args: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .args(args)
        .output()
        .with_context(|| format!("running git {}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git {} failed: {}", args.join(" "), stderr.trim());
    }
    Ok(())
}

fn git_at(cwd: &std::path::Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .with_context(|| format!("running git {}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git {} failed: {}", args.join(" "), stderr.trim());
    }
    Ok(())
}
