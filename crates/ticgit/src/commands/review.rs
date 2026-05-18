use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use ticgit_lib::{keys, MetaValue, Target};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::commands::open_store;

const REVIEW_INDEX_KEY: &str = "review:branches";

#[derive(Debug, Parser)]
pub struct Args {
    #[command(subcommand)]
    pub command: CommandKind,
}

#[derive(Debug, Subcommand)]
pub enum CommandKind {
    /// Create review metadata for a branch.
    New(NewArgs),
    /// List known reviews.
    List(ListArgs),
    /// Show review metadata and messages.
    Show(ShowArgs),
    /// Request a reviewer.
    AddReviewer(AddReviewerArgs),
    /// Add a review comment.
    Comment(CommentArgs),
    /// Approve a review or commit.
    Approve(ApproveArgs),
    /// Request changes on a review.
    RequestChanges(RequestChangesArgs),
    /// Refresh the review head and revision list.
    Update(UpdateArgs),
    /// Record an integration/merge commit.
    Integrate(IntegrateArgs),
}

#[derive(Debug, Parser)]
pub struct NewArgs {
    /// Branch name to review. Defaults to the current branch.
    #[arg(long)]
    pub branch: Option<String>,

    /// Base ref for the review. Defaults to the first available main/master ref.
    #[arg(long)]
    pub base: Option<String>,

    /// Ticket id (or prefix) to link to this review.
    #[arg(long)]
    pub ticket: Option<String>,

    /// Review title. Defaults to the linked ticket title or branch name.
    #[arg(long)]
    pub title: Option<String>,

    /// Review description.
    #[arg(long)]
    pub description: Option<String>,

    /// Initial reviewer email. May be passed multiple times.
    #[arg(long = "reviewer")]
    pub reviewers: Vec<String>,
}

#[derive(Debug, Parser)]
pub struct ListArgs {
    /// Only show reviews with this status.
    #[arg(long)]
    pub status: Option<String>,
}

#[derive(Debug, Parser)]
pub struct ShowArgs {
    /// Branch name or branch-id. Defaults to the current branch.
    pub review: Option<String>,
}

#[derive(Debug, Parser)]
pub struct AddReviewerArgs {
    /// Either `<email>` or `<branch> <email>`.
    #[arg(required = true, num_args = 1..=2)]
    pub args: Vec<String>,
}

#[derive(Debug, Parser)]
pub struct CommentArgs {
    /// Optional path for an inline comment.
    #[arg(long)]
    pub path: Option<String>,

    /// Optional single line for an inline comment.
    #[arg(long)]
    pub line: Option<u32>,

    /// Optional line range for an inline comment, such as 42-82.
    #[arg(long)]
    pub lines: Option<String>,

    /// Optional commit for this comment.
    #[arg(long)]
    pub commit: Option<String>,

    /// Either `<body...>` or `<branch> <body...>`.
    #[arg(required = true, num_args = 1..)]
    pub args: Vec<String>,
}

#[derive(Debug, Parser)]
pub struct ApproveArgs {
    /// Optional approval comment.
    #[arg(long)]
    pub comment: Option<String>,

    /// Optional branch-id, branch name, or commit SHA.
    pub target: Option<String>,
}

#[derive(Debug, Parser)]
pub struct RequestChangesArgs {
    /// Either `<body...>` or `<branch> <body...>`.
    #[arg(required = true, num_args = 1..)]
    pub args: Vec<String>,
}

#[derive(Debug, Parser)]
pub struct UpdateArgs {
    /// Branch name or branch-id. Defaults to the current branch.
    pub review: Option<String>,

    /// New head commit. Defaults to the branch head.
    #[arg(long)]
    pub head: Option<String>,
}

#[derive(Debug, Parser)]
pub struct IntegrateArgs {
    /// Either `<sha>` or `<branch> <sha>`.
    #[arg(required = true, num_args = 1..=2)]
    pub args: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ReviewMessage {
    author: String,
    body: String,
    #[serde(rename = "type")]
    message_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lines: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ReviewRevisionChange {
    sha: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    change_id: Option<String>,
}

#[derive(Debug)]
struct ReviewRef {
    branch_id: String,
    branch_name: Option<String>,
}

pub fn run(args: Args) -> Result<()> {
    match args.command {
        CommandKind::New(args) => new(args),
        CommandKind::List(args) => list(args),
        CommandKind::Show(args) => show(args),
        CommandKind::AddReviewer(args) => add_reviewer(args),
        CommandKind::Comment(args) => comment(args),
        CommandKind::Approve(args) => approve(args),
        CommandKind::RequestChanges(args) => request_changes(args),
        CommandKind::Update(args) => update(args),
        CommandKind::Integrate(args) => integrate(args),
    }
}

fn new(args: NewArgs) -> Result<()> {
    let store = open_store()?;
    let branch_name = args.branch.unwrap_or(current_branch()?);
    let branch_id = ensure_branch_id(&store, &branch_name)?;
    let base_ref = args.base.unwrap_or_else(default_base_ref);
    let base_sha =
        resolve_ref(&base_ref).unwrap_or_else(|_| resolve_ref("HEAD").unwrap_or_default());
    let head_sha = resolve_ref(&branch_name).or_else(|_| resolve_ref("HEAD"))?;
    let now = now_rfc3339()?;
    let target = store.session().target(&Target::branch(&branch_id));
    let mut title = args.title.unwrap_or_else(|| branch_name.clone());
    let mut description = args.description.unwrap_or_default();

    if let Some(ticket_ref) = args.ticket.as_deref() {
        let ticket_id = store.resolve_id(ticket_ref)?;
        let ticket = store.load(&ticket_id)?;
        if title == branch_name {
            title = ticket.title.clone();
        }
        if description.is_empty() {
            description = ticket.description.clone().unwrap_or_default();
        }
        target.set_add("issue:id", &ticket_id.to_string())?;
        store.session().target(&Target::project()).set(
            &keys::ticket_field(&ticket_id, "branch-id"),
            branch_id.as_str(),
        )?;
    }

    target.set("title", title.as_str())?;
    target.set("description", description.as_str())?;
    target.set("status", "open")?;
    target.set("base:sha", base_sha.as_str())?;
    target.set("head:sha", head_sha.as_str())?;
    target.set("review:created-at", now.as_str())?;
    target.set("review:created-by", store.email())?;
    for reviewer in &args.reviewers {
        target.set_add("review:reviewers", reviewer)?;
    }
    refresh_revisions(&store, &branch_id, &base_sha, &head_sha)?;
    index_review(&store, &branch_id)?;

    let branch_target = store.session().target(&Target::branch(&branch_id));
    branch_target.set("code:branch", branch_name.as_str())?;
    if let Some(url) = remote_url()? {
        branch_target.set("code:url", url.as_str())?;
    }

    println!("Created review {branch_id}: {title}");
    Ok(())
}

fn list(args: ListArgs) -> Result<()> {
    let store = open_store()?;
    let project = store.session().target(&Target::project());
    let ids = string_set(project.get_value(REVIEW_INDEX_KEY)?);
    if ids.is_empty() {
        println!("No reviews");
        return Ok(());
    }

    println!("{:<28} {:<17} {:<12} Title", "BranchId", "Branch", "Status");
    for branch_id in ids {
        let target = store.session().target(&Target::branch(&branch_id));
        let status = string_value(target.get_value("status")?).unwrap_or_else(|| "unknown".into());
        if args
            .status
            .as_deref()
            .is_some_and(|wanted| wanted != status)
        {
            continue;
        }
        let title = string_value(target.get_value("title")?).unwrap_or_default();
        let branch = string_value(target.get_value("code:branch")?).unwrap_or_default();
        println!("{:<28} {:<17} {:<12} {}", branch_id, branch, status, title);
    }
    Ok(())
}

fn show(args: ShowArgs) -> Result<()> {
    let store = open_store()?;
    let review = resolve_review(&store, args.review.as_deref())?;
    let target = store.session().target(&Target::branch(&review.branch_id));
    println!("Review: {}", review.branch_id);
    if let Some(branch) = review.branch_name {
        println!("Branch: {branch}");
    }
    for (label, key) in [
        ("Title", "title"),
        ("Description", "description"),
        ("Status", "status"),
        ("Base", "base:sha"),
        ("Head", "head:sha"),
        ("Integration", "integration:sha"),
    ] {
        if let Some(value) = string_value(target.get_value(key)?) {
            if !value.is_empty() {
                println!("{label}: {value}");
            }
        }
    }
    for (label, key) in [("Tickets", "issue:id"), ("Reviewers", "review:reviewers")] {
        let values = string_set(target.get_value(key)?);
        if !values.is_empty() {
            println!("{label}: {}", values.join(", "));
        }
    }

    let revisions = target.list_entries("review:revisions").unwrap_or_default();
    if !revisions.is_empty() {
        println!("Revisions:");
        for entry in revisions {
            println!("  {}", entry.value);
        }
    }

    let messages = target.list_entries("review:messages").unwrap_or_default();
    if !messages.is_empty() {
        println!("Messages:");
        for entry in messages {
            match serde_json::from_str::<ReviewMessage>(&entry.value) {
                Ok(message) => {
                    let location = message_location(&message);
                    println!(
                        "  [{}] {}{}: {}",
                        message.message_type, message.author, location, message.body
                    );
                }
                Err(_) => println!("  {}", entry.value),
            }
        }
    }
    Ok(())
}

fn add_reviewer(args: AddReviewerArgs) -> Result<()> {
    let (review_arg, email) = split_optional_review_arg(args.args)?;
    let store = open_store()?;
    let review = resolve_review(&store, review_arg.as_deref())?;
    store
        .session()
        .target(&Target::branch(&review.branch_id))
        .set_add("review:reviewers", &email)?;
    println!("Added reviewer {email} to {}", review.branch_id);
    Ok(())
}

fn comment(args: CommentArgs) -> Result<()> {
    let (review_arg, body) = split_review_and_body(args.args)?;
    let store = open_store()?;
    let review = resolve_review(&store, review_arg.as_deref())?;
    let lines = args
        .lines
        .or_else(|| args.line.map(|line| line.to_string()));
    append_message(
        &store,
        &review.branch_id,
        ReviewMessage {
            author: store.email().to_string(),
            body,
            message_type: "comment".into(),
            commit: args.commit,
            path: args.path,
            lines,
        },
    )?;
    println!("Added comment to {}", review.branch_id);
    Ok(())
}

fn approve(args: ApproveArgs) -> Result<()> {
    let store = open_store()?;
    let (review, commit) = match args.target.as_deref() {
        Some(target) if looks_like_sha(target) && resolve_review(&store, Some(target)).is_err() => {
            (resolve_review(&store, None).ok(), Some(target.to_string()))
        }
        Some(target) => (Some(resolve_review(&store, Some(target))?), None),
        None => (Some(resolve_review(&store, None)?), None),
    };

    if let Some(commit) = commit.as_deref() {
        let commit_target = store.session().target(&Target::commit(commit)?);
        commit_target.set_add("review:approvals", store.email())?;
        commit_target.set_add("review:reviewed", store.email())?;
    }

    if let Some(review) = review {
        let target = store.session().target(&Target::branch(&review.branch_id));
        target.set("status", "approved")?;
        append_message(
            &store,
            &review.branch_id,
            ReviewMessage {
                author: store.email().to_string(),
                body: args.comment.unwrap_or_else(|| "Approved".into()),
                message_type: "approval".into(),
                commit,
                path: None,
                lines: None,
            },
        )?;
        println!("Approved {}", review.branch_id);
    } else if let Some(commit) = commit {
        println!("Approved commit {commit}");
    }
    Ok(())
}

fn request_changes(args: RequestChangesArgs) -> Result<()> {
    let (review_arg, body) = split_review_and_body(args.args)?;
    let store = open_store()?;
    let review = resolve_review(&store, review_arg.as_deref())?;
    store
        .session()
        .target(&Target::branch(&review.branch_id))
        .set("status", "changes-requested")?;
    append_message(
        &store,
        &review.branch_id,
        ReviewMessage {
            author: store.email().to_string(),
            body,
            message_type: "changes-requested".into(),
            commit: None,
            path: None,
            lines: None,
        },
    )?;
    println!("Requested changes on {}", review.branch_id);
    Ok(())
}

fn update(args: UpdateArgs) -> Result<()> {
    let store = open_store()?;
    let review = resolve_review(&store, args.review.as_deref())?;
    let target = store.session().target(&Target::branch(&review.branch_id));
    let title = string_value(target.get_value("title")?);
    let description = string_value(target.get_value("description")?);
    let branch =
        string_value(target.get_value("code:branch")?).unwrap_or_else(|| review.branch_id.clone());
    let head = match args.head {
        Some(head) => resolve_ref(&head).unwrap_or(head),
        None => resolve_ref(&branch).or_else(|_| resolve_ref("HEAD"))?,
    };
    let base = string_value(target.get_value("base:sha")?).unwrap_or_default();
    target.set("head:sha", head.as_str())?;
    refresh_revisions(&store, &review.branch_id, &base, &head)?;
    if let Some(title) = title {
        target.set("title", title.as_str())?;
    }
    if let Some(description) = description {
        target.set("description", description.as_str())?;
    }
    println!(
        "Updated review {} to {}",
        review.branch_id,
        short_sha(&head)
    );
    Ok(())
}

fn integrate(args: IntegrateArgs) -> Result<()> {
    let (review_arg, sha) = split_optional_review_arg(args.args)?;
    let store = open_store()?;
    let review = resolve_review(&store, review_arg.as_deref())?;
    let resolved_sha = resolve_ref(&sha).unwrap_or(sha);
    let target = store.session().target(&Target::branch(&review.branch_id));
    target.set("integration:sha", resolved_sha.as_str())?;
    target.set("status", "merged")?;
    target.set("review:merged-at", now_rfc3339()?.as_str())?;
    target.set("review:merged-by", store.email())?;
    println!(
        "Integrated {} at {}",
        review.branch_id,
        short_sha(&resolved_sha)
    );
    Ok(())
}

fn resolve_review(store: &ticgit_lib::TicketStore, explicit: Option<&str>) -> Result<ReviewRef> {
    let name = match explicit {
        Some(name) => name.to_string(),
        None => current_branch()?,
    };
    let branch_target = store.session().target(&Target::branch(&name));
    if let Some(branch_id) = string_value(branch_target.get_value("branch-id")?) {
        return Ok(ReviewRef {
            branch_id,
            branch_name: Some(name),
        });
    }

    let review_target = store.session().target(&Target::branch(&name));
    if review_target.get_value("status")?.is_some() || review_target.get_value("title")?.is_some() {
        let branch_name = string_value(review_target.get_value("code:branch")?);
        return Ok(ReviewRef {
            branch_id: name,
            branch_name,
        });
    }

    bail!("no review metadata found for `{name}`; run `ti review new` first")
}

fn ensure_branch_id(store: &ticgit_lib::TicketStore, branch_name: &str) -> Result<String> {
    let branch_target = store.session().target(&Target::branch(branch_name));
    if let Some(branch_id) = string_value(branch_target.get_value("branch-id")?) {
        return Ok(branch_id);
    }

    let timestamp = OffsetDateTime::now_utc().unix_timestamp();
    let branch_id = format!("{branch_name}@{timestamp}");
    branch_target.set("branch-id", branch_id.as_str())?;
    Ok(branch_id)
}

fn index_review(store: &ticgit_lib::TicketStore, branch_id: &str) -> Result<()> {
    store
        .session()
        .target(&Target::project())
        .set_add(REVIEW_INDEX_KEY, branch_id)?;
    Ok(())
}

fn append_message(
    store: &ticgit_lib::TicketStore,
    branch_id: &str,
    message: ReviewMessage,
) -> Result<()> {
    let json = serde_json::to_string(&message)?;
    store
        .session()
        .target(&Target::branch(branch_id))
        .list_push("review:messages", &json)?;
    Ok(())
}

fn refresh_revisions(
    store: &ticgit_lib::TicketStore,
    branch_id: &str,
    base_sha: &str,
    head_sha: &str,
) -> Result<()> {
    let target = store.session().target(&Target::branch(branch_id));
    let previous = target
        .list_entries("review:revisions")
        .unwrap_or_default()
        .into_iter()
        .map(|entry| entry.value)
        .collect::<Vec<_>>();
    let commits = revision_list(base_sha, head_sha)?;
    target.remove("review:revisions")?;
    for sha in &commits {
        target.list_push("review:revisions", &sha)?;
    }
    append_revision_change_history(store, branch_id, &previous)?;
    append_revision_change_history(store, branch_id, &commits)?;
    Ok(())
}

fn append_revision_change_history(
    store: &ticgit_lib::TicketStore,
    branch_id: &str,
    commits: &[String],
) -> Result<()> {
    let target = store.session().target(&Target::branch(branch_id));
    let mut seen = target
        .list_entries("review:revision-history")
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| serde_json::from_str::<ReviewRevisionChange>(&entry.value).ok())
        .map(|entry| entry.sha)
        .collect::<std::collections::BTreeSet<_>>();
    for sha in commits.iter().rev() {
        if seen.insert(sha.clone()) {
            target.list_push(
                "review:revision-history",
                &serde_json::to_string(&ReviewRevisionChange {
                    sha: sha.clone(),
                    change_id: commit_change_id(sha),
                })?,
            )?;
        }
    }
    Ok(())
}

fn revision_list(base_sha: &str, head_sha: &str) -> Result<Vec<String>> {
    if base_sha.is_empty() || head_sha.is_empty() {
        return Ok(Vec::new());
    }
    let range = format!("{base_sha}..{head_sha}");
    let output = Command::new("git")
        .args(["rev-list", &range])
        .output()
        .with_context(|| "running git rev-list")?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

fn current_branch() -> Result<String> {
    let branch = git_output(&["rev-parse", "--abbrev-ref", "HEAD"])?;
    if branch == "HEAD" {
        bail!("cannot default review to detached HEAD; pass --branch or a review id")
    }
    Ok(branch)
}

fn commit_change_id(sha: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["cat-file", "-p", sha])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .take_while(|line| !line.is_empty())
        .find_map(|line| line.strip_prefix("change-id ").map(str::to_string))
}

fn default_base_ref() -> String {
    for candidate in ["origin/main", "origin/master", "main", "master"] {
        if resolve_ref(candidate).is_ok() {
            return candidate.to_string();
        }
    }
    "HEAD".into()
}

fn resolve_ref(reference: &str) -> Result<String> {
    git_output(&["rev-parse", reference])
}

fn remote_url() -> Result<Option<String>> {
    match git_output(&["remote", "get-url", "origin"]) {
        Ok(url) if !url.is_empty() => Ok(Some(url)),
        _ => Ok(None),
    }
}

fn git_output(args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .with_context(|| format!("running git {}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("git {} failed: {}", args.join(" "), stderr.trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn now_rfc3339() -> Result<String> {
    Ok(OffsetDateTime::now_utc().format(&Rfc3339)?)
}

fn string_value(value: Option<MetaValue>) -> Option<String> {
    match value {
        Some(MetaValue::String(value)) => Some(value),
        _ => None,
    }
}

fn string_set(value: Option<MetaValue>) -> Vec<String> {
    match value {
        Some(MetaValue::Set(values)) => values.into_iter().collect(),
        Some(MetaValue::String(value)) if !value.is_empty() => vec![value],
        _ => Vec::new(),
    }
}

fn split_optional_review_arg(args: Vec<String>) -> Result<(Option<String>, String)> {
    match args.as_slice() {
        [value] => Ok((None, value.clone())),
        [review, value] => Ok((Some(review.clone()), value.clone())),
        _ => bail!("expected one or two arguments"),
    }
}

fn split_review_and_body(args: Vec<String>) -> Result<(Option<String>, String)> {
    if args.len() == 1 {
        return Ok((None, args[0].clone()));
    }

    let store = open_store()?;
    if resolve_review(&store, Some(&args[0])).is_ok() {
        Ok((Some(args[0].clone()), args[1..].join(" ")))
    } else {
        Ok((None, args.join(" ")))
    }
}

fn looks_like_sha(value: &str) -> bool {
    value.len() >= 3 && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn short_sha(value: &str) -> &str {
    value.get(..7).unwrap_or(value)
}

fn message_location(message: &ReviewMessage) -> String {
    let mut pieces = Vec::new();
    if let Some(commit) = message.commit.as_deref() {
        pieces.push(short_sha(commit).to_string());
    }
    if let Some(path) = message.path.as_deref() {
        if let Some(lines) = message.lines.as_deref() {
            pieces.push(format!("{path}:{lines}"));
        } else {
            pieces.push(path.to_string());
        }
    }
    if pieces.is_empty() {
        String::new()
    } else {
        format!(" ({})", pieces.join(" "))
    }
}
