use std::env;
use std::fs;
use std::process::Command;

use anyhow::{Context, Result};
use clap::Parser;

const REPO: &str = "schacon/ticgit";

#[derive(Debug, Parser)]
pub struct Args {
    /// Check for updates without installing.
    #[arg(long)]
    pub check: bool,
}

pub fn run(args: Args) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    let latest = fetch_latest_version()?;

    if latest == current {
        println!("Already up to date (v{current}).");
        return Ok(());
    }

    println!("Update available: v{current} -> v{latest}");

    if args.check {
        return Ok(());
    }

    let target = detect_target()?;
    let url = format!("https://github.com/{REPO}/releases/latest/download/ticgit-{target}.tar.gz");

    println!("Downloading from: {url}");

    let tmp = tempfile::tempdir().context("creating temp dir")?;
    let archive_path = tmp.path().join("ticgit.tar.gz");

    let status = Command::new("curl")
        .args(["-fSL", &url, "-o"])
        .arg(&archive_path)
        .status()
        .context("running curl")?;

    if !status.success() {
        anyhow::bail!("download failed");
    }

    let status = Command::new("tar")
        .args(["xzf"])
        .arg(&archive_path)
        .arg("-C")
        .arg(tmp.path())
        .status()
        .context("extracting archive")?;

    if !status.success() {
        anyhow::bail!("extraction failed");
    }

    let new_binary = tmp.path().join("ti");
    let current_binary = env::current_exe().context("finding current binary path")?;

    // Replace the running binary
    fs::copy(&new_binary, &current_binary)
        .context("replacing binary — you may need to run with sudo")?;

    println!("Updated to v{latest}.");
    Ok(())
}

fn fetch_latest_version() -> Result<String> {
    let output = Command::new("curl")
        .args([
            "-fsSL",
            "-H",
            "Accept: application/json",
            &format!("https://github.com/{REPO}/releases/latest"),
        ])
        .output()
        .context("fetching latest release")?;

    if !output.status.success() {
        anyhow::bail!("failed to check for updates");
    }

    let body = String::from_utf8_lossy(&output.stdout);

    // GitHub redirects to /releases/tag/vX.Y.Z and the JSON response
    // contains "tag_name":"vX.Y.Z"
    let tag = body
        .split("\"tag_name\":\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .ok_or_else(|| anyhow::anyhow!("could not parse latest version from GitHub"))?;

    Ok(tag.trim_start_matches('v').to_string())
}

fn detect_target() -> Result<String> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    let os_target = match os {
        "macos" => "apple-darwin",
        "linux" => "unknown-linux-gnu",
        "windows" => anyhow::bail!("self-update on Windows is not yet supported"),
        other => anyhow::bail!("unsupported OS: {other}"),
    };

    let arch_target = match arch {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => anyhow::bail!("unsupported architecture: {other}"),
    };

    Ok(format!("{arch_target}-{os_target}"))
}
