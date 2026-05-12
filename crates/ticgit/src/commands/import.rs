use std::collections::BTreeSet;
use std::process::Command;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::Deserialize;
use ticgit_lib::{NewTicketOpts, Ticket, TicketStore};

use crate::commands::open_store;
use crate::render;

const GH_ISSUE_FIELDS: &str = "number,title,body,url,author,labels,assignees,milestone";
const GITHUB_TAG: &str = "github";
const GITHUB_ISSUE_TAG_PREFIX: &str = "github-issue-";

const LINEAR_TAG: &str = "linear";
const LINEAR_ISSUE_TAG_PREFIX: &str = "linear-";
const LINEAR_API_URL: &str = "https://api.linear.app/graphql";

#[derive(Debug, Parser)]
pub struct Args {
    #[command(subcommand)]
    pub source: Source,
}

#[derive(Debug, Subcommand)]
pub enum Source {
    /// Import open issues using the GitHub CLI (`gh`).
    #[command(name = "gh")]
    Gh(GhArgs),

    /// Import issues from Linear via the Linear GraphQL API.
    Linear(LinearArgs),
}

#[derive(Debug, Parser)]
pub struct GhArgs {
    /// GitHub repository to import from, in OWNER/REPO form.
    #[arg(short = 'R', long = "repo")]
    pub repo: Option<String>,

    /// Maximum number of open issues to request from GitHub.
    #[arg(long = "limit", default_value_t = 1000, value_parser = clap::value_parser!(u32).range(1..))]
    pub limit: u32,

    /// Output an import summary and imported tickets as JSON.
    #[arg(long = "json")]
    pub json: bool,

    /// Output an import summary and imported tickets as Markdown.
    #[arg(long = "markdown", conflicts_with = "json")]
    pub markdown: bool,
}

#[derive(Debug, Parser)]
pub struct LinearArgs {
    /// Linear team key (e.g. "ENG"). If omitted, lists available teams.
    #[arg(short = 't', long = "team")]
    pub team: Option<String>,

    /// Maximum number of issues to import.
    #[arg(long = "limit", default_value_t = 1000, value_parser = clap::value_parser!(u32).range(1..))]
    pub limit: u32,

    /// Output an import summary and imported tickets as JSON.
    #[arg(long = "json")]
    pub json: bool,

    /// Output an import summary and imported tickets as Markdown.
    #[arg(long = "markdown", conflicts_with = "json")]
    pub markdown: bool,
}

pub fn run(args: Args) -> Result<()> {
    match args.source {
        Source::Gh(args) => run_gh(args),
        Source::Linear(args) => run_linear(args),
    }
}

fn run_gh(args: GhArgs) -> Result<()> {
    let store = open_store()?;
    let issues = fetch_gh_issues(&args)?;
    let mut seen = existing_github_issue_numbers(&store)?;

    let mut imported = 0usize;
    let mut skipped = 0usize;
    let mut imported_tickets = Vec::new();

    for issue in issues {
        if seen.contains(&issue.number) {
            skipped += 1;
            continue;
        }

        let opts = NewTicketOpts {
            comment: None,
            tags: issue_tags(&issue),
            assigned: primary_assignee(&issue),
            ..Default::default()
        };
        let ticket = store.create(&issue.title, opts)?;
        store.set_description(&ticket.id, Some(&issue_description(&issue)))?;

        if let Some(milestone) = issue.milestone.as_ref().and_then(|m| non_empty(&m.title)) {
            store.set_milestone(&ticket.id, Some(milestone))?;
        }

        imported_tickets.push(store.load(&ticket.id)?);
        seen.insert(issue.number);
        imported += 1;
    }

    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "imported": imported,
                "skipped": skipped,
                "tickets": imported_tickets,
            })
        );
        return Ok(());
    }
    if args.markdown {
        println!(
            "{}",
            render::import_markdown(imported, skipped, &imported_tickets)
        );
        return Ok(());
    }

    println!("Imported {imported} GitHub issue(s).");
    if skipped > 0 {
        println!("Skipped {skipped} issue(s) that were already imported.");
    }

    Ok(())
}

fn fetch_gh_issues(args: &GhArgs) -> Result<Vec<GhIssue>> {
    let limit = args.limit.to_string();
    let mut command = Command::new("gh");
    command
        .arg("issue")
        .arg("list")
        .arg("--state")
        .arg("open")
        .arg("--limit")
        .arg(&limit)
        .arg("--json")
        .arg(GH_ISSUE_FIELDS);

    if let Some(repo) = &args.repo {
        command.arg("--repo").arg(repo);
    }

    let output = command.output().context(
        "running `gh issue list`; install GitHub CLI and authenticate with `gh auth login`",
    )?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = stderr.trim();
        if message.is_empty() {
            anyhow::bail!("gh issue list failed with status {}", output.status);
        }
        anyhow::bail!("gh issue list failed: {message}");
    }

    serde_json::from_slice(&output.stdout).context("parsing `gh issue list --json` output")
}

fn existing_github_issue_numbers(store: &TicketStore) -> Result<BTreeSet<u64>> {
    let mut numbers = BTreeSet::new();
    for ticket in store.list()? {
        collect_github_issue_numbers(&ticket, &mut numbers);
    }
    Ok(numbers)
}

fn collect_github_issue_numbers(ticket: &Ticket, out: &mut BTreeSet<u64>) {
    for tag in &ticket.tags {
        if let Some(number) = tag
            .strip_prefix(GITHUB_ISSUE_TAG_PREFIX)
            .and_then(|s| s.parse::<u64>().ok())
        {
            out.insert(number);
        }
    }
}

fn issue_tags(issue: &GhIssue) -> Vec<String> {
    let mut tags = vec![
        GITHUB_TAG.to_string(),
        format!("{GITHUB_ISSUE_TAG_PREFIX}{}", issue.number),
    ];
    tags.extend(
        issue
            .labels
            .iter()
            .filter_map(|label| non_empty(&label.name).map(ToString::to_string)),
    );
    tags
}

fn primary_assignee(issue: &GhIssue) -> Option<String> {
    issue.assignees.iter().find_map(|assignee| {
        non_empty(&assignee.login).map(|login| {
            if login.contains('@') {
                login.to_string()
            } else {
                format!("{login}@users.noreply.github.com")
            }
        })
    })
}

fn issue_description(issue: &GhIssue) -> String {
    let mut description = format!("GitHub issue: {}", issue.url);

    if let Some(author) = issue
        .author
        .as_ref()
        .and_then(|author| non_empty(&author.login))
    {
        description.push_str(&format!("\nGitHub author: {author}"));
    }

    let assignees: Vec<_> = issue
        .assignees
        .iter()
        .filter_map(|assignee| non_empty(&assignee.login))
        .collect();
    if assignees.len() > 1 {
        description.push_str(&format!("\nGitHub assignees: {}", assignees.join(", ")));
    }

    if let Some(body) = issue.body.as_deref().and_then(non_empty) {
        description.push_str("\n\n");
        description.push_str(body);
    }

    description
}

// ── Linear import ──────────────────────────────────────────────────

fn get_linear_api_key() -> Result<String> {
    std::env::var("LINEAR_API_KEY").map_err(|_| {
        anyhow::anyhow!(
            "LINEAR_API_KEY environment variable not set.\n\n\
             To get your API key:\n\
             1. Go to https://linear.app/settings/api\n\
             2. Click \"Create key\" under \"Personal API keys\"\n\
             3. Export it in your shell:\n\n\
             \x1b[1m  export LINEAR_API_KEY=lin_api_...\x1b[0m\n\n\
             Then run this command again."
        )
    })
}

fn run_linear(args: LinearArgs) -> Result<()> {
    let api_key = get_linear_api_key()?;

    let team = match args.team {
        Some(t) => t,
        None => {
            // No --team given: list available teams.
            let teams = fetch_linear_teams(&api_key)?;
            if teams.is_empty() {
                println!("No teams found in your Linear workspace.");
                return Ok(());
            }
            println!("Available Linear teams:\n");
            for team in &teams {
                println!(
                    "  \x1b[1m\x1b[36m{}\x1b[0m  {}  \x1b[2m({} open issue{})\x1b[0m",
                    team.key,
                    team.name,
                    team.issue_count,
                    if team.issue_count == 1 { "" } else { "s" },
                );
            }
            println!("\nUsage: ti import linear --team <KEY>");
            return Ok(());
        }
    };

    let store = open_store()?;
    let issues = fetch_linear_issues(&api_key, &team, args.limit)?;
    let seen = existing_linear_identifiers(&store)?;

    let mut imported = 0usize;
    let mut skipped = 0usize;
    let mut imported_tickets = Vec::new();

    for issue in issues {
        if seen.contains(&issue.identifier) {
            skipped += 1;
            continue;
        }

        let created_at = issue.created_at.as_deref().and_then(|s| {
            time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok()
        });
        let opts = NewTicketOpts {
            comment: None,
            tags: linear_issue_tags(&issue),
            assigned: linear_assignee(&issue),
            parent: None,
            created_at,
        };
        let ticket = store.create(&issue.title, opts)?;
        store.set_description(&ticket.id, Some(&linear_description(&issue)))?;

        if let Some(project) = issue.project.as_ref().and_then(|p| non_empty(&p.name)) {
            store.set_milestone(&ticket.id, Some(project))?;
        }

        if let Some(priority) = issue.priority {
            if priority > 0 {
                store.set_points(&ticket.id, Some(priority as i64))?;
            }
        }

        imported_tickets.push(store.load(&ticket.id)?);
        imported += 1;
    }

    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "imported": imported,
                "skipped": skipped,
                "tickets": imported_tickets,
            })
        );
        return Ok(());
    }
    if args.markdown {
        println!(
            "{}",
            render::import_markdown(imported, skipped, &imported_tickets)
        );
        return Ok(());
    }

    println!("Imported {imported} Linear issue(s).");
    if skipped > 0 {
        println!("Skipped {skipped} issue(s) that were already imported.");
    }

    Ok(())
}

const LINEAR_TEAMS_QUERY: &str = r#"
query {
  teams(orderBy: updatedAt) {
    nodes {
      key
      name
      issueCount
    }
  }
}
"#;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinearTeam {
    key: String,
    name: String,
    issue_count: u32,
}

fn fetch_linear_teams(api_key: &str) -> Result<Vec<LinearTeam>> {
    let body = serde_json::json!({
        "query": LINEAR_TEAMS_QUERY,
    });

    let response: serde_json::Value = ureq::post(LINEAR_API_URL)
        .header("Authorization", api_key)
        .send_json(&body)
        .context("calling Linear GraphQL API")?
        .body_mut()
        .read_json()
        .context("parsing Linear API response")?;

    if let Some(errors) = response.get("errors") {
        let msg = errors
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error");
        anyhow::bail!("Linear API error: {msg}");
    }

    let nodes = response
        .get("data")
        .and_then(|d| d.get("teams"))
        .and_then(|t| t.get("nodes"))
        .cloned()
        .unwrap_or_default();

    serde_json::from_value(nodes).context("parsing Linear teams")
}

const LINEAR_QUERY: &str = r#"
query($teamKey: String!, $first: Int!, $after: String) {
  issues(
    filter: {
      team: { key: { eq: $teamKey } }
      state: { type: { nin: ["completed", "canceled"] } }
    }
    first: $first
    after: $after
    orderBy: createdAt
  ) {
    nodes {
      id
      identifier
      title
      description
      url
      priority
      createdAt
      state { name }
      assignee { name email }
      labels { nodes { name } }
      project { name }
    }
    pageInfo {
      hasNextPage
      endCursor
    }
  }
}
"#;

fn fetch_linear_issues(api_key: &str, team: &str, limit: u32) -> Result<Vec<LinearIssue>> {
    let mut all_issues = Vec::new();
    let mut cursor: Option<String> = None;
    let page_size = limit.min(100);

    loop {
        let variables = serde_json::json!({
            "teamKey": team,
            "first": page_size,
            "after": cursor,
        });
        let body = serde_json::json!({
            "query": LINEAR_QUERY,
            "variables": variables,
        });

        let response: serde_json::Value = ureq::post(LINEAR_API_URL)
            .header("Authorization", api_key)
            .send_json(&body)
            .context("calling Linear GraphQL API")?
            .body_mut()
            .read_json()
            .context("parsing Linear API response")?;

        if let Some(errors) = response.get("errors") {
            let msg = errors
                .as_array()
                .and_then(|arr| arr.first())
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            anyhow::bail!("Linear API error: {msg}");
        }

        let issues_data = response
            .get("data")
            .and_then(|d| d.get("issues"))
            .ok_or_else(|| anyhow::anyhow!("unexpected Linear API response structure"))?;

        let nodes: Vec<LinearIssue> =
            serde_json::from_value(issues_data.get("nodes").cloned().unwrap_or_default())
                .context("parsing Linear issues")?;

        all_issues.extend(nodes);

        if all_issues.len() as u32 >= limit {
            all_issues.truncate(limit as usize);
            break;
        }

        let has_next = issues_data
            .get("pageInfo")
            .and_then(|p| p.get("hasNextPage"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !has_next {
            break;
        }

        cursor = issues_data
            .get("pageInfo")
            .and_then(|p| p.get("endCursor"))
            .and_then(|v| v.as_str())
            .map(String::from);
    }

    Ok(all_issues)
}

fn existing_linear_identifiers(store: &TicketStore) -> Result<BTreeSet<String>> {
    let mut identifiers = BTreeSet::new();
    for ticket in store.list()? {
        for tag in &ticket.tags {
            if let Some(id) = tag.strip_prefix(LINEAR_ISSUE_TAG_PREFIX) {
                identifiers.insert(id.to_string());
            }
        }
    }
    Ok(identifiers)
}

fn linear_issue_tags(issue: &LinearIssue) -> Vec<String> {
    let mut tags = vec![
        LINEAR_TAG.to_string(),
        format!("{LINEAR_ISSUE_TAG_PREFIX}{}", issue.identifier),
    ];
    if let Some(labels) = &issue.labels {
        tags.extend(
            labels
                .nodes
                .iter()
                .filter_map(|l| non_empty(&l.name).map(|s| s.to_lowercase())),
        );
    }
    tags
}

fn linear_assignee(issue: &LinearIssue) -> Option<String> {
    issue.assignee.as_ref().and_then(|a| {
        non_empty(&a.email)
            .filter(|e| e.contains('@'))
            .map(ToString::to_string)
    })
}

fn linear_description(issue: &LinearIssue) -> String {
    let mut desc = format!("Linear issue: {}", issue.url);

    if let Some(state) = &issue.state {
        if let Some(name) = non_empty(&state.name) {
            desc.push_str(&format!("\nLinear state: {name}"));
        }
    }
    if let Some(assignee) = &issue.assignee {
        if let Some(name) = assignee.name.as_deref().and_then(non_empty) {
            desc.push_str(&format!("\nLinear assignee: {name}"));
        } else if let Some(email) = non_empty(&assignee.email) {
            desc.push_str(&format!("\nLinear assignee: {email}"));
        }
    }

    if let Some(body) = issue.description.as_deref().and_then(non_empty) {
        desc.push_str("\n\n");
        desc.push_str(body);
    }

    desc
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinearIssue {
    identifier: String,
    title: String,
    description: Option<String>,
    url: String,
    priority: Option<i32>,
    created_at: Option<String>,
    state: Option<LinearState>,
    assignee: Option<LinearAssignee>,
    labels: Option<LinearLabels>,
    project: Option<LinearProject>,
}

#[derive(Debug, Deserialize)]
struct LinearState {
    name: String,
}

#[derive(Debug, Deserialize)]
struct LinearAssignee {
    name: Option<String>,
    email: String,
}

#[derive(Debug, Deserialize)]
struct LinearLabels {
    nodes: Vec<LinearLabel>,
}

#[derive(Debug, Deserialize)]
struct LinearLabel {
    name: String,
}

#[derive(Debug, Deserialize)]
struct LinearProject {
    name: String,
}

// ── Shared helpers ─────────────────────────────────────────────────

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

#[derive(Debug, Deserialize)]
struct GhIssue {
    number: u64,
    title: String,
    body: Option<String>,
    url: String,
    #[serde(default)]
    author: Option<GhUser>,
    #[serde(default)]
    labels: Vec<GhLabel>,
    #[serde(default)]
    assignees: Vec<GhUser>,
    milestone: Option<GhMilestone>,
}

#[derive(Debug, Deserialize)]
struct GhLabel {
    name: String,
}

#[derive(Debug, Deserialize)]
struct GhUser {
    login: String,
}

#[derive(Debug, Deserialize)]
struct GhMilestone {
    title: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue() -> GhIssue {
        GhIssue {
            number: 42,
            title: "import me".to_string(),
            body: Some("body text".to_string()),
            url: "https://github.com/example/repo/issues/42".to_string(),
            author: Some(GhUser {
                login: "monalisa".to_string(),
            }),
            labels: vec![GhLabel {
                name: "bug".to_string(),
            }],
            assignees: vec![
                GhUser {
                    login: "octocat".to_string(),
                },
                GhUser {
                    login: "hubot".to_string(),
                },
            ],
            milestone: Some(GhMilestone {
                title: "v1".to_string(),
            }),
        }
    }

    #[test]
    fn github_issue_tags_include_source_and_labels() {
        assert_eq!(
            issue_tags(&issue()),
            vec![
                "github".to_string(),
                "github-issue-42".to_string(),
                "bug".to_string()
            ]
        );
    }

    #[test]
    fn github_issue_description_preserves_source_body_and_extra_assignees() {
        assert_eq!(
            issue_description(&issue()),
            "GitHub issue: https://github.com/example/repo/issues/42\nGitHub author: monalisa\nGitHub assignees: octocat, hubot\n\nbody text"
        );
    }

    fn linear_issue() -> LinearIssue {
        LinearIssue {
            identifier: "ENG-123".to_string(),
            title: "fix auth flow".to_string(),
            description: Some("Users can't log in after token expiry".to_string()),
            url: "https://linear.app/team/issue/ENG-123".to_string(),
            priority: Some(2),
            created_at: Some("2025-03-15T10:30:00.000Z".to_string()),
            state: Some(LinearState {
                name: "In Progress".to_string(),
            }),
            assignee: Some(LinearAssignee {
                name: Some("Alice".to_string()),
                email: "alice@example.com".to_string(),
            }),
            labels: Some(LinearLabels {
                nodes: vec![LinearLabel {
                    name: "Bug".to_string(),
                }],
            }),
            project: Some(LinearProject {
                name: "Auth Rewrite".to_string(),
            }),
        }
    }

    #[test]
    fn linear_issue_tags_include_source_identifier_and_labels() {
        assert_eq!(
            linear_issue_tags(&linear_issue()),
            vec![
                "linear".to_string(),
                "linear-ENG-123".to_string(),
                "bug".to_string(),
            ]
        );
    }

    #[test]
    fn linear_issue_description_includes_url_state_and_body() {
        assert_eq!(
            linear_description(&linear_issue()),
            "Linear issue: https://linear.app/team/issue/ENG-123\nLinear state: In Progress\nLinear assignee: Alice\n\nUsers can't log in after token expiry"
        );
    }

    #[test]
    fn linear_assignee_prefers_email() {
        assert_eq!(
            linear_assignee(&linear_issue()),
            Some("alice@example.com".to_string())
        );
    }

    #[test]
    fn linear_assignee_skips_non_email() {
        let mut issue = linear_issue();
        issue.assignee = Some(LinearAssignee {
            name: None,
            email: String::new(),
        });
        assert_eq!(linear_assignee(&issue), None);
    }
}
