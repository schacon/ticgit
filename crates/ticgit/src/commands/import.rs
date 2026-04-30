use std::collections::BTreeSet;
use std::process::Command;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::Deserialize;
use ticgit_lib::{NewTicketOpts, Ticket, TicketStore};

use crate::commands::open_store;

const GH_ISSUE_FIELDS: &str = "number,title,body,url,labels,assignees,milestone";
const GITHUB_TAG: &str = "github";
const GITHUB_ISSUE_TAG_PREFIX: &str = "github-issue-";

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
}

#[derive(Debug, Parser)]
pub struct GhArgs {
    /// GitHub repository to import from, in OWNER/REPO form.
    #[arg(short = 'R', long = "repo")]
    pub repo: Option<String>,

    /// Maximum number of open issues to request from GitHub.
    #[arg(long = "limit", default_value_t = 1000, value_parser = clap::value_parser!(u32).range(1..))]
    pub limit: u32,
}

pub fn run(args: Args) -> Result<()> {
    match args.source {
        Source::Gh(args) => run_gh(args),
    }
}

fn run_gh(args: GhArgs) -> Result<()> {
    let store = open_store()?;
    let issues = fetch_gh_issues(&args)?;
    let mut seen = existing_github_issue_numbers(&store)?;

    let mut imported = 0usize;
    let mut skipped = 0usize;

    for issue in issues {
        if seen.contains(&issue.number) {
            skipped += 1;
            continue;
        }

        let opts = NewTicketOpts {
            comment: None,
            tags: issue_tags(&issue),
            assigned: primary_assignee(&issue),
        };
        let ticket = store.create(&issue.title, opts)?;
        store.set_description(&ticket.id, Some(&issue_description(&issue)))?;

        if let Some(milestone) = issue.milestone.as_ref().and_then(|m| non_empty(&m.title)) {
            store.set_milestone(&ticket.id, Some(milestone))?;
        }

        seen.insert(issue.number);
        imported += 1;
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
    issue
        .assignees
        .iter()
        .find_map(|assignee| non_empty(&assignee.login).map(ToString::to_string))
}

fn issue_description(issue: &GhIssue) -> String {
    let mut description = format!("GitHub issue: {}", issue.url);

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
            "GitHub issue: https://github.com/example/repo/issues/42\nGitHub assignees: octocat, hubot\n\nbody text"
        );
    }
}
