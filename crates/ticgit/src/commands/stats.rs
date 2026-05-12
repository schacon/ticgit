use std::collections::BTreeMap;

use anyhow::Result;
use clap::Parser;
use crossterm::terminal;
use time::OffsetDateTime;

use crate::commands::open_store;

#[derive(Debug, Parser)]
pub struct Args {
    /// Output stats as JSON.
    #[arg(long = "json")]
    pub json: bool,

    /// Output stats as Markdown.
    #[arg(long = "markdown", conflicts_with = "json")]
    pub markdown: bool,
}

// ANSI colour helpers
const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const CYAN: &str = "\x1b[36m";
const RED: &str = "\x1b[31m";
const MAGENTA: &str = "\x1b[35m";
const WHITE: &str = "\x1b[97m";
const BG_GREEN: &str = "\x1b[42m";
const BG_CYAN: &str = "\x1b[46m";
const BG_MAGENTA: &str = "\x1b[45m";

pub fn run(args: Args) -> Result<()> {
    let store = open_store()?;
    let tickets = store.list()?;
    let total = tickets.len();

    if total == 0 {
        if args.json {
            println!("{}", serde_json::json!({ "total": 0 }));
        } else {
            println!("No tickets.");
        }
        return Ok(());
    }

    let now = OffsetDateTime::now_utc();
    let week_ago = now - time::Duration::days(7);

    let mut open = 0usize;
    let mut closed = 0usize;
    let mut with_comments = 0usize;
    let mut created_7d = 0usize;
    let mut closed_7d = 0usize;
    let mut states: BTreeMap<String, usize> = BTreeMap::new();
    let mut tags: BTreeMap<String, usize> = BTreeMap::new();
    let mut assignees: BTreeMap<String, usize> = BTreeMap::new();
    let nick_map = crate::render::build_nick_map(&store.list_users().unwrap_or_default());

    // Collect recently closed tickets (by created_at as proxy for activity).
    let mut recently_closed: Vec<(String, String)> = Vec::new(); // (short_id, title)

    for t in &tickets {
        match t.status {
            ticgit_lib::TicketStatus::Open => open += 1,
            ticgit_lib::TicketStatus::Closed => {
                closed += 1;
                recently_closed.push((t.id.to_string().chars().take(6).collect(), t.title.clone()));
            }
        }
        *states.entry(t.state.as_str().to_string()).or_default() += 1;

        if !t.comments.is_empty() {
            with_comments += 1;
        }

        if t.created_at >= week_ago {
            created_7d += 1;
        }
        if t.status == ticgit_lib::TicketStatus::Closed && t.created_at >= week_ago {
            closed_7d += 1;
        }

        for tag in &t.tags {
            *tags.entry(tag.clone()).or_default() += 1;
        }

        if let Some(ref a) = t.assigned {
            let short = if let Some(nick) = nick_map.get(a.as_str()) {
                nick.clone()
            } else {
                a.split_once('@')
                    .map(|(local, _)| local)
                    .unwrap_or(a)
                    .to_string()
            };
            *assignees.entry(short).or_default() += 1;
        }
    }

    if args.json {
        let states_json: serde_json::Value = states
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::json!(v)))
            .collect();
        let tags_json: serde_json::Value = tags
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::json!(v)))
            .collect();
        let assignees_json: serde_json::Value = assignees
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::json!(v)))
            .collect();

        println!(
            "{}",
            serde_json::json!({
                "total": total,
                "open": open,
                "closed": closed,
                "with_comments": with_comments,
                "created_7d": created_7d,
                "closed_7d": closed_7d,
                "states": states_json,
                "tags": tags_json,
                "assignees": assignees_json,
            })
        );
        return Ok(());
    }

    if args.markdown {
        // Pre-sort data.
        let mut state_vec: Vec<_> = states.iter().collect();
        state_vec.sort_by(|a, b| b.1.cmp(a.1));
        let mut tag_vec: Vec<_> = tags.iter().collect();
        tag_vec.sort_by(|a, b| b.1.cmp(a.1));
        let mut assignee_vec: Vec<_> = assignees.iter().collect();
        assignee_vec.sort_by(|a, b| b.1.cmp(a.1));

        println!("# Ticket Stats\n");
        println!("| Metric | Count |");
        println!("| --- | --- |");
        println!("| Total | {total} |");
        println!("| Open | {open} |");
        println!("| Closed | {closed} |");
        println!("| With comments | {with_comments} |");
        if created_7d > 0 {
            println!("| Created (7d) | {created_7d} |");
        }
        if closed_7d > 0 {
            println!("| Closed (7d) | {closed_7d} |");
        }

        if !state_vec.is_empty() {
            println!("\n## States\n");
            println!("| State | Count |");
            println!("| --- | --- |");
            for (state, count) in &state_vec {
                println!("| {state} | {count} |");
            }
        }

        if !tag_vec.is_empty() {
            println!("\n## Tags\n");
            println!("| Tag | Count |");
            println!("| --- | --- |");
            for (tag, count) in &tag_vec {
                println!("| {tag} | {count} |");
            }
        }

        if !assignee_vec.is_empty() {
            println!("\n## Assignees\n");
            println!("| Assignee | Count |");
            println!("| --- | --- |");
            for (name, count) in &assignee_vec {
                println!("| {name} | {count} |");
            }
        }

        if !recently_closed.is_empty() {
            println!("\n## Recently Closed\n");
            for (id, title) in recently_closed.iter().take(10) {
                println!("- `{id}` {title}");
            }
        }

        return Ok(());
    }

    // ── Terminal-aware dashboard ────────────────────────────────

    let (term_width, term_height) = terminal::size()
        .map(|(w, h)| (w as usize, h as usize))
        .unwrap_or((80, 24));

    let width = term_width.min(120);
    let two_col = width >= 70;

    // Pre-sort data.
    let mut state_vec: Vec<_> = states.into_iter().collect();
    state_vec.sort_by(|a, b| b.1.cmp(&a.1));

    let mut tag_vec: Vec<_> = tags.into_iter().collect();
    tag_vec.sort_by(|a, b| b.1.cmp(&a.1));

    let mut assignee_vec: Vec<_> = assignees.into_iter().collect();
    assignee_vec.sort_by(|a, b| b.1.cmp(&a.1));

    // Budget vertical space conservatively.
    // Fixed overhead: blank + divider + title + divider + blank + overview + blank + divider = 8
    // Plus 2 lines buffer for shell prompt.
    let overhead = 10;
    let avail = term_height.saturating_sub(overhead);

    // In two-col mode, left and right share vertical space.
    // Left: states + recent activity. Right: tags + assignees + recently closed.
    // Tag limit: leave room for section headers, assignees, recently closed.
    let tag_limit = if two_col {
        // Right column budget: tags header(1) + tags + gap(1) + assignees header(1) + assignees + gap(1) + closed header(1) + closed
        let right_fixed = 2 + assignee_vec.len().min(3) + 1;
        let closed_budget = 4; // header + up to 3 titles
        avail
            .saturating_sub(right_fixed + closed_budget)
            .min(tag_vec.len())
            .min(6)
    } else {
        avail
            .saturating_sub(state_vec.len() + 8)
            .min(tag_vec.len())
            .min(5)
    };

    let assignee_limit = assignee_vec.len().min(3);

    // Recently closed: show if there's vertical room.
    let recently_closed_limit = if two_col {
        let left_rows = state_vec.len()
            + 1
            + if created_7d > 0 || closed_7d > 0 {
                4
            } else {
                0
            };
        let right_rows = 1
            + tag_limit
            + (if tag_vec.len() > tag_limit { 1 } else { 0 })
            + 1
            + if !assignee_vec.is_empty() {
                1 + assignee_limit + 1
            } else {
                0
            };
        let used = left_rows.max(right_rows);
        avail
            .saturating_sub(used + 1)
            .min(recently_closed.len())
            .min(5)
    } else {
        avail
            .saturating_sub(state_vec.len() + tag_limit + 10)
            .min(recently_closed.len())
            .min(3)
    };

    // ── Build output ──

    let divider = format!("{DIM}{}{RESET}", "─".repeat(width));
    println!();
    println!("{divider}");
    println!("  {BOLD}{WHITE}TICGIT STATS{RESET}");
    println!("{divider}");
    println!();

    // Overview block
    let comment_note = if with_comments > 0 {
        format!("{DIM}, {with_comments} with comments{RESET}")
    } else {
        String::new()
    };
    println!(
        "  {BOLD}{WHITE}{total}{RESET} ticket(s){comment_note}    \
         {GREEN}{BOLD}{open}{RESET}{DIM} open{RESET}  \
         {RED}{BOLD}{closed}{RESET}{DIM} closed{RESET}"
    );
    println!();

    if two_col {
        // ── Two-column layout ──
        let col_width = (width - 4) / 2;
        let bar_width = col_width.saturating_sub(22);

        let mut left_lines: Vec<String> = Vec::new();
        let mut right_lines: Vec<String> = Vec::new();

        // Left: States
        if !state_vec.is_empty() {
            left_lines.push(format!("{CYAN}{BOLD}  States{RESET}"));
            let max_count = state_vec.iter().map(|(_, c)| *c).max().unwrap_or(1);
            for (state, count) in &state_vec {
                let bar = colored_bar(*count, max_count, bar_width, BG_CYAN);
                let color = state_color(state);
                left_lines.push(format!(
                    "    {color}{:<12}{RESET} {:>3}  {bar}",
                    state, count
                ));
            }
            left_lines.push(String::new());
        }

        // Left: Recent activity
        if created_7d > 0 || closed_7d > 0 {
            left_lines.push(format!("{YELLOW}{BOLD}  Recent (7d){RESET}"));
            if created_7d > 0 {
                left_lines.push(format!(
                    "    {GREEN}+{created_7d}{RESET}{DIM} created{RESET}"
                ));
            }
            if closed_7d > 0 {
                left_lines.push(format!("    {RED}-{closed_7d}{RESET}{DIM} closed{RESET}"));
            }
            left_lines.push(String::new());
        }

        // Left: Assignees
        if !assignee_vec.is_empty() {
            left_lines.push(format!("{GREEN}{BOLD}  Assignees{RESET}"));
            let max_a = assignee_vec.first().map(|(_, c)| *c).unwrap_or(1);
            for (name, count) in assignee_vec.iter().take(assignee_limit) {
                let bar = colored_bar(*count, max_a, bar_width, BG_GREEN);
                left_lines.push(format!(
                    "    {GREEN}{:<12}{RESET} {:>3}  {bar}",
                    name, count
                ));
            }
            left_lines.push(String::new());
        }

        // Right: Tags
        if !tag_vec.is_empty() {
            right_lines.push(format!("{MAGENTA}{BOLD}  Tags{RESET}"));
            let max_count = tag_vec.first().map(|(_, c)| *c).unwrap_or(1);
            for (tag, count) in tag_vec.iter().take(tag_limit) {
                let bar = colored_bar(*count, max_count, bar_width, BG_MAGENTA);
                right_lines.push(format!(
                    "    {MAGENTA}{:<12}{RESET} {:>3}  {bar}",
                    tag, count
                ));
            }
            let remaining = tag_vec.len().saturating_sub(tag_limit);
            if remaining > 0 {
                right_lines.push(format!("    {DIM}... and {remaining} more{RESET}"));
            }
            right_lines.push(String::new());
        }

        // Right: Recently Closed
        if recently_closed_limit > 0 {
            right_lines.push(format!("{RED}{BOLD}  Recently Closed{RESET}"));
            for (id, title) in recently_closed.iter().take(recently_closed_limit) {
                let max_title = col_width.saturating_sub(12);
                let display_title = if title.len() > max_title {
                    format!("{}...", &title[..max_title.saturating_sub(3)])
                } else {
                    title.clone()
                };
                right_lines.push(format!("    {DIM}{id}{RESET} {display_title}"));
            }
            let remaining = recently_closed.len().saturating_sub(recently_closed_limit);
            if remaining > 0 {
                right_lines.push(format!("    {DIM}... and {remaining} more{RESET}"));
            }
            right_lines.push(String::new());
        }

        // Merge columns side by side.
        let max_rows = left_lines.len().max(right_lines.len());
        for i in 0..max_rows {
            let left = left_lines.get(i).map(|s| s.as_str()).unwrap_or("");
            let right = right_lines.get(i).map(|s| s.as_str()).unwrap_or("");
            let left_visible = visible_len(left);
            let pad = col_width.saturating_sub(left_visible);
            println!("{left}{}{right}", " ".repeat(pad));
        }
    } else {
        // ── Single-column layout ──
        let bar_width = width.saturating_sub(24);

        if !state_vec.is_empty() {
            println!("  {CYAN}{BOLD}States{RESET}");
            let max_count = state_vec.iter().map(|(_, c)| *c).max().unwrap_or(1);
            for (state, count) in &state_vec {
                let bar = colored_bar(*count, max_count, bar_width, BG_CYAN);
                let color = state_color(state);
                println!("    {color}{:<14}{RESET}{:>3}  {bar}", state, count);
            }
            println!();
        }

        if !tag_vec.is_empty() {
            println!("  {MAGENTA}{BOLD}Top Tags{RESET}");
            let max_count = tag_vec.first().map(|(_, c)| *c).unwrap_or(1);
            for (tag, count) in tag_vec.iter().take(tag_limit) {
                let bar = colored_bar(*count, max_count, bar_width, BG_MAGENTA);
                println!("    {MAGENTA}{:<14}{RESET}{:>3}  {bar}", tag, count);
            }
            let remaining = tag_vec.len().saturating_sub(tag_limit);
            if remaining > 0 {
                println!("    {DIM}... and {remaining} more{RESET}");
            }
            println!();
        }

        if created_7d > 0 || closed_7d > 0 {
            println!("  {YELLOW}{BOLD}Recent Activity (7d){RESET}");
            if created_7d > 0 {
                println!("    {GREEN}+{created_7d}{RESET}{DIM} created{RESET}");
            }
            if closed_7d > 0 {
                println!("    {RED}-{closed_7d}{RESET}{DIM} closed{RESET}");
            }
            println!();
        }

        if !assignee_vec.is_empty() {
            println!("  {GREEN}{BOLD}Assignees{RESET}");
            for (name, count) in assignee_vec.iter().take(assignee_limit) {
                println!("    {GREEN}{:<14}{RESET}{:>3}", name, count);
            }
            println!();
        }

        if recently_closed_limit > 0 {
            println!("  {RED}{BOLD}Recently Closed{RESET}");
            for (id, title) in recently_closed.iter().take(recently_closed_limit) {
                let max_title = width.saturating_sub(12);
                let display_title = if title.len() > max_title {
                    format!("{}...", &title[..max_title.saturating_sub(3)])
                } else {
                    title.clone()
                };
                println!("    {DIM}{id}{RESET} {display_title}");
            }
            println!();
        }
    }

    println!("{divider}");

    Ok(())
}

fn state_color(state: &str) -> &'static str {
    match state {
        "new" => GREEN,
        "open" => GREEN,
        "resolved" => CYAN,
        "invalid" | "wontfix" => DIM,
        _ => WHITE,
    }
}

fn colored_bar(value: usize, max: usize, width: usize, bg: &str) -> String {
    if max == 0 || width == 0 {
        return String::new();
    }
    let filled = ((value as f64 / max as f64) * width as f64).round() as usize;
    let filled = filled.max(if value > 0 { 1 } else { 0 }).min(width);
    format!("{bg}{}{RESET}", " ".repeat(filled))
}

/// Compute the visible length of a string (ignoring ANSI escape codes).
fn visible_len(s: &str) -> usize {
    let mut len = 0;
    let mut in_escape = false;
    for ch in s.chars() {
        if in_escape {
            if ch.is_ascii_alphabetic() {
                in_escape = false;
            }
        } else if ch == '\x1b' {
            in_escape = true;
        } else {
            len += 1;
        }
    }
    len
}
