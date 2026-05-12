use std::collections::BTreeMap;
use std::process::Command;

use anyhow::Context;
use anyhow::Result;
use clap::Parser;
use uuid::Uuid;

use crate::commands::open_store;

#[derive(Debug, Parser)]
pub struct Args {
    /// Remote to sync with. Defaults to git-meta's first configured meta remote.
    #[arg(short = 'r', long = "remote")]
    pub remote: Option<String>,
}

pub fn run_sync(args: Args) -> Result<()> {
    let store = open_store()?;
    let remote = sync_remote(args.remote.as_deref())?;
    let namespace = meta_namespace()?;
    let remote_url = remote
        .as_deref()
        .map(remote_url)
        .transpose()?
        .unwrap_or_else(|| "(none)".to_string());

    if let Some(remote) = &remote {
        println!("Remote: {remote}");
    }
    println!("Ref: refs/{namespace}/main");
    println!("URL: {remote_url}");
    if let Some(web_url) = ssh_project_web_url(&remote_url) {
        println!("Web URL: {web_url}");
    }

    // Snapshot before pull
    let before: BTreeMap<Uuid, String> = store
        .list()?
        .into_iter()
        .map(|t| (t.id, t.title.clone()))
        .collect();

    store.pull(args.remote.as_deref())?;

    // Snapshot after pull to find new tickets
    let after: BTreeMap<Uuid, String> = store
        .list()?
        .into_iter()
        .map(|t| (t.id, t.title.clone()))
        .collect();

    let new_tickets: Vec<(&Uuid, &String)> = after
        .iter()
        .filter(|(id, _)| !before.contains_key(id))
        .collect();

    if new_tickets.is_empty() {
        println!("Pull: no new tickets.");
    } else {
        println!("Pull: {} new ticket(s):", new_tickets.len());
        for (id, title) in new_tickets.iter().take(5) {
            let short: String = id.to_string().chars().take(6).collect();
            println!("  {short}  {title}");
        }
        if new_tickets.len() > 5 {
            println!("  ... and {} more", new_tickets.len() - 5);
        }
    }

    store.push(args.remote.as_deref())?;

    let total = after.len();
    println!("Push: {total} ticket(s) synced.");
    println!("Done.");
    Ok(())
}

fn sync_remote(explicit: Option<&str>) -> Result<Option<String>> {
    if let Some(remote) = explicit {
        return Ok(Some(remote.to_string()));
    }

    for remote in git_remotes()? {
        if git_config_get_bool(&format!("remote.{remote}.meta"))? == Some(true) {
            return Ok(Some(remote));
        }
    }

    let remotes = git_remotes()?;
    if remotes.iter().any(|remote| remote == "origin") {
        return Ok(Some("origin".to_string()));
    }
    Ok(remotes.into_iter().next())
}

fn meta_namespace() -> Result<String> {
    Ok(git_config_get("meta.namespace")?.unwrap_or_else(|| "meta".to_string()))
}

fn remote_url(remote: &str) -> Result<String> {
    git_output(&["remote", "get-url", remote])
}

fn ssh_project_web_url(url: &str) -> Option<String> {
    let (host, path) = if let Some(rest) = url.strip_prefix("git@") {
        rest.split_once(':')?
    } else if let Some(rest) = url.strip_prefix("ssh://git@") {
        rest.split_once('/')?
    } else {
        return None;
    };

    if !matches!(host, "github.com" | "gitlab.com") {
        return None;
    }

    let path = path.trim_end_matches(".git").trim_matches('/');
    if path.is_empty() {
        return None;
    }
    Some(format!("https://{host}/{path}"))
}

fn git_remotes() -> Result<Vec<String>> {
    let output = git_output(&["remote"])?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect())
}

fn git_config_get(key: &str) -> Result<Option<String>> {
    optional_git_output(&["config", "--get", key])
}

fn git_config_get_bool(key: &str) -> Result<Option<bool>> {
    Ok(
        optional_git_output(&["config", "--bool", "--get", key])?.and_then(|value| {
            match value.trim() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            }
        }),
    )
}

fn optional_git_output(args: &[&str]) -> Result<Option<String>> {
    let output = Command::new("git")
        .args(args)
        .output()
        .with_context(|| format!("running git {}", args.join(" ")))?;

    if output.status.success() {
        return Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        ));
    }

    if output.status.code() == Some(1) {
        return Ok(None);
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    anyhow::bail!("git {} failed: {}", args.join(" "), stderr.trim());
}

fn git_output(args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .with_context(|| format!("running git {}", args.join(" ")))?;

    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    anyhow::bail!("git {} failed: {}", args.join(" "), stderr.trim());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_project_web_url_handles_github_and_gitlab() {
        assert_eq!(
            ssh_project_web_url("git@github.com:owner/repo.git").as_deref(),
            Some("https://github.com/owner/repo")
        );
        assert_eq!(
            ssh_project_web_url("ssh://git@gitlab.com/group/project.git").as_deref(),
            Some("https://gitlab.com/group/project")
        );
    }

    #[test]
    fn ssh_project_web_url_ignores_non_ssh_or_unknown_hosts() {
        assert_eq!(ssh_project_web_url("https://github.com/owner/repo"), None);
        assert_eq!(ssh_project_web_url("git@example.com:owner/repo.git"), None);
    }
}
