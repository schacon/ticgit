use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use time::OffsetDateTime;

use crate::commands::{open_store, resolve_ticket, SessionGitDir};

#[derive(Debug, Parser)]
pub struct Args {
    /// Ticket id (or prefix). Defaults to the currently checked-out ticket.
    #[arg(short = 't', long = "ticket")]
    pub ticket: Option<String>,

    /// Maximum number of entries to show.
    #[arg(short = 'n', long = "limit")]
    pub limit: Option<usize>,

    /// Output as JSON.
    #[arg(long = "json")]
    pub json: bool,

    /// Output as Markdown.
    #[arg(long = "markdown", conflicts_with = "json")]
    pub markdown: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
struct HistoryEntry {
    field: String,
    value: String,
    operation: String,
    email: String,
    #[serde(with = "time::serde::rfc3339")]
    at: OffsetDateTime,
}

pub fn run(args: Args) -> Result<()> {
    let store = open_store()?;
    let id = resolve_ticket(&store, args.ticket.as_deref())?;
    let git_dir = store.session().repo_git_dir();
    let db_path = db_path_for(&git_dir)?;

    let entries = query_history(&db_path, &id.to_string(), args.limit)?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }

    if args.markdown {
        print_markdown(&id.to_string(), &entries);
        return Ok(());
    }

    print_terminal(&entries);
    Ok(())
}

fn db_path_for(git_dir: &std::path::Path) -> Result<PathBuf> {
    let path = git_dir.join("git-meta.sqlite");
    anyhow::ensure!(path.exists(), "no git-meta database at {}", path.display());
    Ok(path)
}

fn query_history(
    db_path: &std::path::Path,
    ticket_id: &str,
    limit: Option<usize>,
) -> Result<Vec<HistoryEntry>> {
    let conn =
        rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .context("opening git-meta database")?;

    let prefix = format!("ticgit:tickets:{ticket_id}:");
    let limit_val = limit.unwrap_or(100) as i64;

    let mut stmt = conn.prepare(
        "SELECT key, value, operation, email, timestamp \
         FROM metadata_log \
         WHERE target_type = 'project' AND key LIKE ?1 \
         ORDER BY timestamp DESC \
         LIMIT ?2",
    )?;

    let rows = stmt.query_map(rusqlite::params![format!("{prefix}%"), limit_val], |row| {
        let key: String = row.get(0)?;
        let value: String = row.get(1)?;
        let operation: String = row.get(2)?;
        let email: String = row.get(3)?;
        let timestamp_ms: i64 = row.get(4)?;
        Ok((key, value, operation, email, timestamp_ms))
    })?;

    let mut entries = Vec::new();
    for row in rows {
        let (key, value, operation, email, timestamp_ms) = row?;
        let field = key.strip_prefix(&prefix).unwrap_or(&key).to_string();
        let value = if field == "comments" {
            "(comment added)".to_string()
        } else {
            clean_value(&value)
        };
        let at = OffsetDateTime::from_unix_timestamp(timestamp_ms / 1000)
            .unwrap_or(OffsetDateTime::UNIX_EPOCH);
        entries.push(HistoryEntry {
            field,
            value,
            operation,
            email,
            at,
        });
    }

    Ok(entries)
}

fn clean_value(raw: &str) -> String {
    // Values are stored JSON-encoded (quoted strings, arrays).
    // Try to unwrap simple quoted strings for display.
    if raw.starts_with('"') && raw.ends_with('"') {
        if let Ok(s) = serde_json::from_str::<String>(raw) {
            return s;
        }
    }
    // Arrays like ["bug","feature"] — show as comma-separated.
    if raw.starts_with('[') {
        if let Ok(arr) = serde_json::from_str::<Vec<String>>(raw) {
            return arr.join(", ");
        }
    }
    raw.to_string()
}

const ANSI_RESET: &str = "\x1b[0m";
const ANSI_DIM: &str = "\x1b[2m";
const ANSI_YELLOW: &str = "\x1b[33m";
const ANSI_CYAN: &str = "\x1b[36m";
const ANSI_GREEN: &str = "\x1b[32m";

fn print_terminal(entries: &[HistoryEntry]) {
    if entries.is_empty() {
        println!("(no history)");
        return;
    }

    let mut last_date = String::new();
    for e in entries {
        let date = format!(
            "{:04}-{:02}-{:02}",
            e.at.year(),
            u8::from(e.at.month()),
            e.at.day()
        );
        let time = format!("{:02}:{:02}", e.at.hour(), e.at.minute());

        if date != last_date {
            println!("\n{}{}{}", ANSI_DIM, date, ANSI_RESET);
            last_date = date;
        }

        let verb = match e.operation.as_str() {
            "set" => format!("{}set{}", ANSI_GREEN, ANSI_RESET),
            "remove" => format!("{}removed{}", ANSI_YELLOW, ANSI_RESET),
            "set_add" => format!("{}added{}", ANSI_GREEN, ANSI_RESET),
            "set_remove" | "set_rm" => format!("{}removed{}", ANSI_YELLOW, ANSI_RESET),
            "push" => format!("{}updated{}", ANSI_GREEN, ANSI_RESET),
            other => other.to_string(),
        };

        let short_email = e.email.split('@').next().unwrap_or(&e.email);

        let value_display = if e.value.len() > 60 {
            format!("{}...", &e.value[..57])
        } else {
            e.value.clone()
        };

        println!(
            "  {}{}{} {} {}{}{} → {}{}{}  {}{}{}",
            ANSI_DIM,
            time,
            ANSI_RESET,
            verb,
            ANSI_CYAN,
            e.field,
            ANSI_RESET,
            ANSI_YELLOW,
            value_display,
            ANSI_RESET,
            ANSI_DIM,
            short_email,
            ANSI_RESET,
        );
    }
    println!();
}

fn print_markdown(ticket_id: &str, entries: &[HistoryEntry]) {
    let short: String = ticket_id.chars().take(6).collect();
    println!("# History: {}\n", short);

    if entries.is_empty() {
        println!("_No history._");
        return;
    }

    println!("| Time | Action | Field | Value | By |");
    println!("| --- | --- | --- | --- | --- |");
    for e in entries {
        let dt = format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            e.at.year(),
            u8::from(e.at.month()),
            e.at.day(),
            e.at.hour(),
            e.at.minute()
        );
        let value_display = if e.value.len() > 50 {
            format!("{}...", &e.value[..47])
        } else {
            e.value.clone()
        };
        let short_email = e.email.split('@').next().unwrap_or(&e.email);
        println!(
            "| {} | {} | `{}` | {} | {} |",
            dt,
            e.operation,
            e.field,
            value_display.replace('|', "\\|"),
            short_email
        );
    }
}
