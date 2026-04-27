use anyhow::Result;
use clap::Parser;

use crate::commands::open_store;

#[derive(Debug, Parser)]
pub struct Args {
    /// Remote to sync with. Defaults to git-meta's first configured meta remote.
    #[arg(short = 'r', long = "remote")]
    pub remote: Option<String>,
}

#[derive(Debug, Parser)]
pub struct PullArgs {
    /// Remote to pull from. Defaults to git-meta's first configured meta remote.
    #[arg(short = 'r', long = "remote")]
    pub remote: Option<String>,
}

#[derive(Debug, Parser)]
pub struct PushArgs {
    /// Remote to push to. Defaults to git-meta's first configured meta remote.
    #[arg(short = 'r', long = "remote")]
    pub remote: Option<String>,
}

pub fn run_sync(args: Args) -> Result<()> {
    let store = open_store()?;
    store.pull(args.remote.as_deref())?;
    store.push(args.remote.as_deref())?;
    println!("Synced ticgit metadata.");
    Ok(())
}

pub fn run_pull(args: PullArgs) -> Result<()> {
    let store = open_store()?;
    store.pull(args.remote.as_deref())?;
    println!("Pulled ticgit metadata.");
    Ok(())
}

pub fn run_push(args: PushArgs) -> Result<()> {
    let store = open_store()?;
    store.push(args.remote.as_deref())?;
    println!("Pushed ticgit metadata.");
    Ok(())
}
