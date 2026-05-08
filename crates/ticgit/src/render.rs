//! Terminal output: tables, single-ticket details, and JSON.

use ticgit_lib::Ticket;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// Render a list of tickets as a table. `current` (if any) gets a `*`.
pub fn tickets_table(tickets: &[Ticket], _current: Option<&uuid::Uuid>) -> String {
    let width = crossterm::terminal::size()
        .map(|(columns, _)| columns as usize)
        .unwrap_or(100)
        .max(40);
    tickets_table_with_width(tickets, width, OffsetDateTime::now_utc())
}

fn tickets_table_with_width(tickets: &[Ticket], width: usize, now: OffsetDateTime) -> String {
    const ID_WIDTH: usize = 6;
    const STATE_WIDTH: usize = 8;
    const OPENED_WIDTH: usize = 8;
    const COMMENTS_WIDTH: usize = 4;
    const LAST_WIDTH: usize = 8;
    const GAPS: usize = 10;
    const MIN_TITLE_WIDTH: usize = 8;

    let fixed_width = ID_WIDTH + STATE_WIDTH + OPENED_WIDTH + COMMENTS_WIDTH + LAST_WIDTH + GAPS;
    let title_width = width.saturating_sub(fixed_width).max(MIN_TITLE_WIDTH);

    let mut out = String::new();
    out.push_str(&format!(
        "{:<ID_WIDTH$}  {:<title_width$}  {:<STATE_WIDTH$}  {:>OPENED_WIDTH$}  {:>COMMENTS_WIDTH$}  {:>LAST_WIDTH$}\n",
        "ID", "Title", "State", "Opened", "Com", "Last"
    ));

    for t in tickets {
        let opened = relative_time(t.created_at, now);
        let last_comment = t
            .comments
            .last()
            .map(|comment| relative_time(comment.at, now))
            .unwrap_or_else(|| "-".to_string());
        out.push_str(&format!(
            "{:<ID_WIDTH$}  {:<title_width$}  {:<STATE_WIDTH$}  {:>OPENED_WIDTH$}  {:>COMMENTS_WIDTH$}  {:>LAST_WIDTH$}\n",
            t.short_id(),
            truncate(&t.title, title_width),
            t.state.as_str(),
            opened,
            t.comments.len(),
            last_comment,
        ));
    }

    out
}

/// Render a single ticket and its comments.
pub fn ticket_detail(t: &Ticket) -> String {
    let mut out = String::new();
    let title_bar = "-".repeat(t.title.chars().count().max(20));
    out.push_str(&format!("{title_bar}\nTitle    : {}\n", t.title));
    out.push_str(&format!("Id       : {}\n", t.id));
    out.push_str(&format!(
        "Created  : {}  by {}\n",
        t.created_at.format(&Rfc3339).unwrap_or_default(),
        t.created_by
    ));
    out.push_str(&format!("State    : {}\n", t.state));
    if let Some(a) = &t.assigned {
        out.push_str(&format!("Assigned : {a}\n"));
    }
    if let Some(p) = t.points {
        out.push_str(&format!("Points   : {p}\n"));
    }
    if let Some(m) = &t.milestone {
        out.push_str(&format!("Milestone: {m}\n"));
    }
    if !t.tags.is_empty() {
        let tags: Vec<_> = t.tags.iter().cloned().collect();
        out.push_str(&format!("Tags     : {}\n", tags.join(", ")));
    }
    out.push_str(&title_bar);
    out.push('\n');

    if t.comments.is_empty() {
        out.push_str("(no comments)\n");
    } else {
        for c in &t.comments {
            out.push_str(&format!(
                "\n{} - {}\n  {}\n",
                c.author,
                c.at.format(&Rfc3339).unwrap_or_default(),
                c.body.replace('\n', "\n  "),
            ));
        }
    }
    out
}

/// Render a single ticket as JSON (for scripting).
pub fn ticket_json(t: &Ticket) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(t)
}

pub fn tickets_json(t: &[Ticket]) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(t)
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    if max_chars > 3 {
        let mut out: String = value.chars().take(max_chars - 3).collect();
        out.push_str("...");
        out
    } else {
        ".".repeat(max_chars)
    }
}

fn relative_time(then: OffsetDateTime, now: OffsetDateTime) -> String {
    let duration = now - then;
    let seconds = duration.whole_seconds().max(0);
    if seconds < 60 {
        return "now".to_string();
    }

    let (amount, unit) = if seconds < 60 * 60 {
        (seconds / 60, "m")
    } else if seconds < 60 * 60 * 24 {
        (seconds / (60 * 60), "h")
    } else if seconds < 60 * 60 * 24 * 30 {
        (seconds / (60 * 60 * 24), "d")
    } else if seconds < 60 * 60 * 24 * 365 {
        (seconds / (60 * 60 * 24 * 30), "mo")
    } else {
        (seconds / (60 * 60 * 24 * 365), "y")
    };

    format!("{amount}{unit}")
}
