use std::process::Command;

use anyhow::{Context, Result};

use crate::commands::open_store;

pub fn run() -> Result<()> {
    let setup = ensure_git_meta_defaults()?;
    let store = open_store()?;
    if let Some(remote) = setup.configured_remote {
        println!("Configured git-meta remote '{remote}' with defaults.");
    }
    println!(
        "ticgit initialised on this repository (schema {}).",
        store.schema_version()?.unwrap_or_else(|| "?".into())
    );
    println!("Identity for new metadata: {}", store.email());
    Ok(())
}

#[derive(Debug, Default)]
struct GitMetaSetup {
    configured_remote: Option<String>,
}

fn ensure_git_meta_defaults() -> Result<GitMetaSetup> {
    if git_config_get("meta.namespace")?.is_none() {
        git_config_set(&["meta.namespace", "meta"])?;
    }

    if has_meta_remote()? {
        return Ok(GitMetaSetup::default());
    }

    let Some(remote) = default_remote()? else {
        return Ok(GitMetaSetup::default());
    };

    let namespace = git_config_get("meta.namespace")?.unwrap_or_else(|| "meta".to_string());
    let fetch_refspec = format!("+refs/{namespace}/main:refs/{namespace}/remotes/main");
    let fetch_key = format!("remote.{remote}.fetch");
    let existing_fetch = git_config_get_all(&fetch_key)?;
    if !existing_fetch.iter().any(|value| value == &fetch_refspec) {
        git_config_set(&["--add", &fetch_key, &fetch_refspec])?;
    }

    let prefix = format!("remote.{remote}");
    git_config_set(&[&format!("{prefix}.meta"), "true"])?;
    git_config_set(&[&format!("{prefix}.promisor"), "true"])?;
    git_config_set(&[&format!("{prefix}.partialclonefilter"), "blob:none"])?;

    Ok(GitMetaSetup {
        configured_remote: Some(remote),
    })
}

fn has_meta_remote() -> Result<bool> {
    for remote in git_remotes()? {
        if git_config_get_bool(&format!("remote.{remote}.meta"))? == Some(true) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn default_remote() -> Result<Option<String>> {
    let remotes = git_remotes()?;
    if remotes.is_empty() {
        return Ok(None);
    }
    if remotes.iter().any(|remote| remote == "meta") {
        return Ok(Some("meta".to_string()));
    }
    if remotes.iter().any(|remote| remote == "origin") {
        return Ok(Some("origin".to_string()));
    }
    Ok(remotes.into_iter().next())
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

fn git_config_get_all(key: &str) -> Result<Vec<String>> {
    Ok(optional_git_output(&["config", "--get-all", key])?
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect())
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

fn git_config_set(args: &[&str]) -> Result<()> {
    let mut git_args = Vec::with_capacity(args.len() + 1);
    git_args.push("config");
    git_args.extend_from_slice(args);
    git_output(&git_args).map(|_| ())
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
