use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::commands::open_store;

#[derive(Debug, Parser)]
pub struct Args {
    #[command(subcommand)]
    pub action: Option<Action>,

    /// Output as JSON.
    #[arg(long = "json")]
    pub json: bool,

    /// Output as Markdown.
    #[arg(long = "markdown", conflicts_with = "json")]
    pub markdown: bool,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Add an email to a user nick.
    Add(AddArgs),
    /// Remove a user or an email from a user.
    Rm(RmArgs),
}

#[derive(Debug, Parser)]
pub struct AddArgs {
    /// User nick (e.g. "scott").
    pub nick: String,
    /// Email address to associate.
    pub email: String,
}

#[derive(Debug, Parser)]
pub struct RmArgs {
    /// User nick to remove (or remove an email from).
    pub nick: String,
    /// Optional: remove only this email. Without it, removes the entire user.
    pub email: Option<String>,
}

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const GREEN: &str = "\x1b[32m";
const CYAN: &str = "\x1b[36m";

pub fn run(args: Args) -> Result<()> {
    let store = open_store()?;

    match args.action {
        Some(Action::Add(add)) => {
            store.add_user_email(&add.nick, &add.email)?;
            if args.json {
                let emails = store.get_user(&add.nick)?;
                println!(
                    "{}",
                    serde_json::json!({
                        "nick": add.nick,
                        "emails": emails,
                    })
                );
            } else {
                println!("Added {} to user {}", add.email, add.nick);
            }
        }
        Some(Action::Rm(rm)) => {
            if let Some(email) = &rm.email {
                store.remove_user_email(&rm.nick, email)?;
                if args.json {
                    let emails = store.get_user(&rm.nick)?;
                    println!(
                        "{}",
                        serde_json::json!({
                            "nick": rm.nick,
                            "emails": emails,
                        })
                    );
                } else {
                    println!("Removed {} from user {}", email, rm.nick);
                }
            } else {
                store.remove_user(&rm.nick)?;
                if args.json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "nick": rm.nick,
                            "removed": true,
                        })
                    );
                } else {
                    println!("Removed user {}", rm.nick);
                }
            }
        }
        None => {
            let users = store.list_users()?;

            if args.json {
                let json: serde_json::Value = users
                    .iter()
                    .map(|(nick, emails)| (nick.clone(), serde_json::json!(emails)))
                    .collect();
                println!("{}", serde_json::to_string_pretty(&json)?);
                return Ok(());
            }

            if args.markdown {
                println!("# Users\n");
                if users.is_empty() {
                    println!("_No users configured._");
                } else {
                    println!("| Nick | Emails |");
                    println!("| --- | --- |");
                    for (nick, emails) in &users {
                        let emails_str: Vec<&str> = emails.iter().map(|s| s.as_str()).collect();
                        println!("| {} | {} |", nick, emails_str.join(", "));
                    }
                }
                return Ok(());
            }

            if users.is_empty() {
                println!("No users configured. Add one with: ti users add <nick> <email>");
                return Ok(());
            }

            for (nick, emails) in &users {
                let emails_str: Vec<&str> = emails.iter().map(|s| s.as_str()).collect();
                println!(
                    "  {GREEN}{BOLD}{nick}{RESET}  {CYAN}{}{RESET}",
                    emails_str.join(&format!("{DIM},{RESET} {CYAN}"))
                );
            }
        }
    }

    Ok(())
}
