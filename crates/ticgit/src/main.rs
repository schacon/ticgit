use clap::Parser;

mod cli;
mod commands;
mod editor;
mod render;
mod session_state;

fn main() -> anyhow::Result<()> {
    let args = cli::Cli::parse();
    cli::run(args)
}
