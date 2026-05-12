use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

/// Run `git meta remote add <url> --init` using the URL from `.git-meta`.
/// Returns `true` if setup was performed, `false` if already configured.
pub fn run() -> Result<()> {
    if has_meta_remote()? {
        println!("git-meta remote already configured.");
        return Ok(());
    }

    match run_setup()? {
        true => println!("git-meta remote configured and metadata fetched."),
        false => println!("No .git-meta file found and no meta remote configured."),
    }
    Ok(())
}

/// Attempt auto-setup: if no meta remote is configured but a `.git-meta`
/// file exists, run `git meta remote add <url> --init`. Returns `true` if
/// setup was performed.
pub fn auto_setup_if_needed() -> Result<bool> {
    if has_meta_remote()? {
        return Ok(false);
    }
    let url = read_git_meta_url()?;
    if url.is_none() {
        return Ok(false);
    }
    eprintln!("Setting up ticgit (fetching ticket metadata)...");
    let result = run_setup()?;
    if result {
        eprintln!("Done. Ticket metadata is ready.");
    }
    Ok(result)
}

fn run_setup() -> Result<bool> {
    let Some(url) = read_git_meta_url()? else {
        return Ok(false);
    };

    let output = Command::new("git")
        .args(["meta", "remote", "add", &url, "--init"])
        .output()
        .context("running git meta remote add")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git meta remote add failed: {}", stderr.trim());
    }

    Ok(true)
}

fn read_git_meta_url() -> Result<Option<String>> {
    let path = Path::new(".git-meta");
    if !path.exists() {
        return Ok(None);
    }

    let contents = std::fs::read_to_string(path).context("reading .git-meta")?;
    for line in contents.lines() {
        let line = line.trim();
        if let Some(url) = line.strip_prefix("url:") {
            let url = url.trim();
            if !url.is_empty() {
                return Ok(Some(url.to_string()));
            }
        }
    }
    Ok(None)
}

fn has_meta_remote() -> Result<bool> {
    let output = Command::new("git")
        .args(["remote"])
        .output()
        .context("running git remote")?;

    if !output.status.success() {
        return Ok(false);
    }

    let remotes: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(ToString::to_string)
        .collect();

    for remote in remotes {
        let output = Command::new("git")
            .args([
                "config",
                "--bool",
                "--get",
                &format!("remote.{remote}.meta"),
            ])
            .output()
            .context("checking remote.*.meta config")?;

        if output.status.success() {
            let val = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if val == "true" {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
