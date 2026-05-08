//! Terminal output: tables, single-ticket details, and JSON.

use ticgit_lib::Ticket;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Render a list of tickets as a compact table. `current` (if any) gets a `*`.
pub fn tickets_table(tickets: &[Ticket], current: Option<&uuid::Uuid>) -> String {
    let width = crossterm::terminal::size()
        .map(|(columns, _)| columns as usize)
        .unwrap_or(100)
        .max(40);
    tickets_table_with_width(tickets, current, width, OffsetDateTime::now_utc())
}

fn tickets_table_with_width(
    tickets: &[Ticket],
    current: Option<&uuid::Uuid>,
    width: usize,
    now: OffsetDateTime,
) -> String {
    const ID_WIDTH: usize = 6;
    const STATE_WIDTH: usize = 5;
    const DATE_WIDTH: usize = 5;
    const ASSIGNED_WIDTH: usize = 8;
    const TAGS_WIDTH: usize = 20;
    const GAPS_AND_MARKER: usize = 15;
    const MIN_TITLE_WIDTH: usize = 12;

    let fixed_width =
        ID_WIDTH + STATE_WIDTH + DATE_WIDTH + ASSIGNED_WIDTH + TAGS_WIDTH + GAPS_AND_MARKER;
    let title_width = width.saturating_sub(fixed_width).max(MIN_TITLE_WIDTH);

    let mut out = String::new();
    out.push_str("   ");
    out.push_str(&fit("TicId", ID_WIDTH));
    out.push(' ');
    out.push_str(&fit("Title", title_width));
    out.push(' ');
    out.push_str(&fit("State", STATE_WIDTH));
    out.push(' ');
    out.push_str(&fit("Date", DATE_WIDTH));
    out.push(' ');
    out.push_str(&fit("Assgn", ASSIGNED_WIDTH));
    out.push(' ');
    out.push_str(&fit("Tags", TAGS_WIDTH));
    out.push('\n');
    out.push_str(&"-".repeat(width));
    out.push('\n');

    for t in tickets {
        let marker = if Some(&t.id) == current { "*" } else { " " };
        let assigned = t.assigned_short().unwrap_or_default();
        let tags = t.tags.iter().cloned().collect::<Vec<_>>().join(",");
        out.push_str(marker);
        out.push_str("  ");
        out.push_str(&fit(&t.short_id(), ID_WIDTH));
        out.push(' ');
        out.push_str(&fit(&flatten(&t.title), title_width));
        out.push(' ');
        out.push_str(&fit(t.state.as_str(), STATE_WIDTH));
        out.push(' ');
        out.push_str(&fit(&relative_date(t.created_at, now), DATE_WIDTH));
        out.push(' ');
        out.push_str(&fit(&flatten(&assigned), ASSIGNED_WIDTH));
        out.push(' ');
        out.push_str(&fit(&flatten(&tags), TAGS_WIDTH));
        out.push('\n');
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

fn fit(value: &str, width: usize) -> String {
    let truncated = truncate_display(value, width);
    let padding = width.saturating_sub(UnicodeWidthStr::width(truncated.as_str()));
    format!("{truncated}{}", " ".repeat(padding))
}

fn truncate_display(value: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(value) <= max_width {
        return value.to_string();
    }

    let ellipsis = if max_width > 3 { "..." } else { "." };
    let ellipsis_width = UnicodeWidthStr::width(ellipsis);
    let content_width = max_width.saturating_sub(ellipsis_width);
    let mut out = String::new();
    let mut width = 0;
    for ch in value.chars() {
        let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + char_width > content_width {
            break;
        }
        out.push(ch);
        width += char_width;
    }
    out.push_str(ellipsis);
    out
}

fn flatten(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn relative_date(then: OffsetDateTime, now: OffsetDateTime) -> String {
    let seconds = (now - then).whole_seconds().max(0);
    if seconds < 60 * 60 {
        return "0d".to_string();
    }
    if seconds < 60 * 60 * 24 {
        return format!("{}h", seconds / (60 * 60));
    }
    if seconds < 60 * 60 * 24 * 30 {
        return format!("{}d", seconds / (60 * 60 * 24));
    }
    if seconds < 60 * 60 * 24 * 365 {
        return format!("{}mo", seconds / (60 * 60 * 24 * 30));
    }
    format!("{}y", seconds / (60 * 60 * 24 * 365))
}
