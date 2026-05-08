//! Terminal output: tables, single-ticket details, and JSON.

use ticgit_lib::Ticket;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// Render a list of tickets as a compact table. `current` (if any) gets a `*`.
pub fn tickets_table(tickets: &[Ticket], current: Option<&uuid::Uuid>) -> String {
    let width = crossterm::terminal::size()
        .map(|(columns, _)| columns as usize)
        .unwrap_or(100)
        .max(40);
    tickets_table_with_width(tickets, current, width)
}

fn tickets_table_with_width(
    tickets: &[Ticket],
    current: Option<&uuid::Uuid>,
    width: usize,
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
    out.push_str(&format!(
        "   {:<ID_WIDTH$} {:<title_width$} {:<STATE_WIDTH$} {:<DATE_WIDTH$} {:<ASSIGNED_WIDTH$} {:<TAGS_WIDTH$}\n",
        "TicId", "Title", "State", "Date", "Assgn", "Tags"
    ));
    out.push_str(&"-".repeat(width));
    out.push('\n');

    for t in tickets {
        let marker = if Some(&t.id) == current { "*" } else { " " };
        let assigned = t.assigned_short().unwrap_or_default();
        let tags = t.tags.iter().cloned().collect::<Vec<_>>().join(",");
        out.push_str(&format!(
            "{marker}  {:<ID_WIDTH$} {:<title_width$} {:<STATE_WIDTH$} {:<DATE_WIDTH$} {:<ASSIGNED_WIDTH$} {:<TAGS_WIDTH$}\n",
            t.short_id(),
            truncate(&flatten(&t.title), title_width),
            t.state.as_str(),
            short_date(t.created_at),
            truncate(&flatten(&assigned), ASSIGNED_WIDTH),
            truncate(&flatten(&tags), TAGS_WIDTH),
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

fn flatten(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn short_date(when: OffsetDateTime) -> String {
    format!("{:02}/{:02}", u8::from(when.month()), when.day())
}
