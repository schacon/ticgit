use anyhow::Result;
use clap::Parser;
use serde_json::Value;

use crate::commands::{open_store, resolve_ticket};
use crate::render;

#[derive(Debug, Parser)]
pub struct Args {
    /// Ticket id (or unique prefix). Defaults to the currently checked-out ticket.
    pub ticket: Option<String>,

    /// Output the ticket as JSON.
    #[arg(long = "json")]
    pub json: bool,

    /// Output only one JSON field, using a small jq-like path (e.g. `.title` or `.comments[0].body`).
    #[arg(long = "filter", num_args = 0..=1)]
    pub filter: Option<Option<String>>,
}

pub fn run(args: Args) -> Result<()> {
    let store = open_store()?;
    let id = resolve_ticket(&store, args.ticket.as_deref())?;
    let ticket = store.load(&id)?;

    if matches!(args.filter, Some(None)) {
        print_filter_help();
        return Ok(());
    }

    if let Some(Some(filter)) = &args.filter {
        let json = serde_json::to_value(&ticket)?;
        let filtered = apply_filter(&json, filter)?;
        println!("{}", render_filtered(filtered)?);
        return Ok(());
    }

    if args.json {
        println!("{}", render::ticket_json(&ticket)?);
    } else {
        print!("{}", render::ticket_detail(&ticket));
    }
    Ok(())
}

fn apply_filter<'a>(value: &'a Value, filter: &str) -> Result<&'a Value> {
    let mut current = value;
    let mut chars = filter.trim().chars().peekable();
    if chars.next() != Some('.') {
        anyhow::bail!("filter must start with `.`");
    }

    while chars.peek().is_some() {
        if matches!(chars.peek(), Some('.')) {
            chars.next();
        }

        let mut field = String::new();
        while let Some(&ch) = chars.peek() {
            if ch == '.' || ch == '[' {
                break;
            }
            field.push(ch);
            chars.next();
        }

        if !field.is_empty() {
            current = current
                .get(&field)
                .ok_or_else(|| anyhow::anyhow!("field `{field}` not found"))?;
        }

        while matches!(chars.peek(), Some('[')) {
            chars.next();
            let mut index = String::new();
            while let Some(&ch) = chars.peek() {
                if ch == ']' {
                    break;
                }
                index.push(ch);
                chars.next();
            }
            if chars.next() != Some(']') {
                anyhow::bail!("unterminated array index in filter `{filter}`");
            }
            let index: usize = index
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid array index `{index}`"))?;
            current = current
                .get(index)
                .ok_or_else(|| anyhow::anyhow!("array index `{index}` not found"))?;
        }
    }

    Ok(current)
}

fn render_filtered(value: &Value) -> Result<String> {
    Ok(match value {
        Value::String(s) => s.clone(),
        _ => serde_json::to_string_pretty(value)?,
    })
}

fn print_filter_help() {
    println!(
        "\
Available filters:
  .id
  .title
  .description
  .state
  .assigned
  .points
  .milestone
  .tags
  .comments
  .created_at
  .created_by

Examples:
  ti show <id> --filter '.title'
  ti show <id> --filter '.tags'
  ti show <id> --filter '.comments[0].body'"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_extracts_nested_array_values() {
        let value = serde_json::json!({
            "title": "bug",
            "comments": [{"body": "first"}],
        });

        assert_eq!(apply_filter(&value, ".title").unwrap(), "bug");
        assert_eq!(apply_filter(&value, ".comments[0].body").unwrap(), "first");
    }
}
