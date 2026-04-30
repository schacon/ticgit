//! Terminal output: tables, single-ticket details, and JSON.

use comfy_table::{presets::UTF8_FULL, ContentArrangement, Table};
use ticgit_lib::Ticket;
use time::format_description::well_known::Rfc3339;

/// Render a list of tickets as a table. `current` (if any) gets a `*`.
pub fn tickets_table(tickets: &[Ticket], current: Option<&uuid::Uuid>) -> String {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            "", "Id", "Title", "State", "Assigned", "Tags", "Created",
        ]);

    for t in tickets {
        let marker = if Some(&t.id) == current { "*" } else { "" };
        let assigned = t.assigned_short().unwrap_or_default();
        let tags = t.tags.iter().cloned().collect::<Vec<_>>().join(",");
        let created = t.created_at.format(&Rfc3339).unwrap_or_default();
        let title = if t.title.chars().count() > 64 {
            format!("{}...", t.title.chars().take(61).collect::<String>())
        } else {
            t.title.clone()
        };
        table.add_row(vec![
            marker.into(),
            t.short_id(),
            title,
            t.state.to_string(),
            assigned,
            tags,
            created,
        ]);
    }

    table.to_string()
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
    if let Some(description) = &t.description {
        out.push_str("Description:\n");
        out.push_str(&format!("  {}\n", description.replace('\n', "\n  ")));
    }
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
