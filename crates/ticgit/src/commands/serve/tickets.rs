//! The ticket half of `ti serve`.
//!
//! The list at `/`, a detail page at `/t/<id>`, and `/tickets.json` for
//! scripting. Page chrome, escaping and the HTTP types all come from the
//! parent module.

use anyhow::Result;
use ticgit_lib::{Filter, SearchFilter, SortOrder, Ticket, TicketLifecycle, TicketStatus};
use time::format_description::well_known::Rfc3339;

use super::{
    document, error_page, escape, filter_chip, flatten, hidden_input, percent_encode, tag_hue,
    Page, Request, Response,
};
use crate::commands::open_store;
use crate::render;
use crate::timefmt::relative_time;

// -- responses -------------------------------------------------------------

pub(super) fn list_response(request: &Request) -> Result<Response> {
    let store = open_store()?;
    let query = ListQuery::from_request(request);
    let tickets = ticgit_lib::query::apply(store.list()?, &query.filter()?);
    let page = Page::new(&store)?;
    Ok(Response::html(200, list_page(&page, &query, &tickets)))
}

pub(super) fn json_response(request: &Request) -> Result<Response> {
    let store = open_store()?;
    let query = ListQuery::from_request(request);
    let tickets = ticgit_lib::query::apply(store.list()?, &query.filter()?);
    Ok(Response::new(
        200,
        "application/json; charset=utf-8",
        render::tickets_json(&tickets)?.into_bytes(),
    ))
}

pub(super) fn detail_response(reference: &str) -> Result<Response> {
    let store = open_store()?;
    let id = match store.resolve_id(reference) {
        Ok(id) => id,
        Err(err) => {
            return Ok(Response::html(
                404,
                error_page("404 - no such ticket", &err.to_string()),
            ))
        }
    };
    let ticket = store.load(&id)?;
    let page = Page::new(&store)?;
    Ok(Response::html(200, detail_page(&page, &ticket)))
}

// -- query -----------------------------------------------------------------

/// The list filters we accept as query params. Mirrors `ti list`'s flags.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ListQuery {
    status: Option<String>,
    state: Option<String>,
    tags: Vec<String>,
    assigned: Option<String>,
    search: Option<String>,
    order: Option<String>,
    all: bool,
    subissues: bool,
}

impl ListQuery {
    fn from_request(request: &Request) -> Self {
        let clean = |value: Option<&str>| {
            value
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        };
        Self {
            status: clean(request.param("status")),
            state: clean(request.param("state")),
            tags: request
                .param_values("tag")
                .into_iter()
                .filter(|tag| !tag.trim().is_empty())
                .collect(),
            assigned: clean(request.param("assigned")),
            search: clean(request.param("q")),
            order: clean(request.param("order")),
            all: request.flag("all"),
            subissues: request.flag("subissues"),
        }
    }

    /// Translate into a `ticgit-lib` filter, defaulting to open tickets
    /// the way `ti list` and the TUI's Default view do.
    fn filter(&self) -> Result<Filter> {
        let mut status = match self.status.as_deref() {
            Some("all") => None,
            Some(spec) => Some(TicketStatus::parse(spec)?),
            None if self.all || self.state.is_some() => None,
            None => Some(TicketStatus::Open),
        };
        let mut state = None;
        if let Some(spec) = self.state.as_deref() {
            let lifecycle = TicketLifecycle::parse(spec)?;
            status = Some(lifecycle.status);
            if TicketStatus::parse(spec).is_err() {
                state = Some(lifecycle.state);
            }
        }
        let order = match self.order.as_deref() {
            Some(spec) => Some(
                SortOrder::parse(spec)
                    .ok_or_else(|| anyhow::anyhow!("unknown sort order `{spec}`"))?,
            ),
            None => None,
        };
        let search = match self.search.as_deref() {
            Some(spec) => Some(SearchFilter::parse(spec).map_err(|e| anyhow::anyhow!(e))?),
            None => None,
        };
        Ok(Filter {
            status,
            state,
            tag: self.tags.first().cloned(),
            tags: self.tags.clone(),
            tag_match_all: true,
            assigned: self.assigned.clone(),
            only_tagged: false,
            search,
            order,
            hide_subissues: !(self.subissues || self.all),
        })
    }

    /// Rebuild the query string, optionally replacing the sort order.
    fn href(&self, order: Option<&str>) -> String {
        let mut pairs: Vec<(&str, String)> = Vec::new();
        if let Some(status) = &self.status {
            pairs.push(("status", status.clone()));
        }
        if let Some(state) = &self.state {
            pairs.push(("state", state.clone()));
        }
        for tag in &self.tags {
            pairs.push(("tag", tag.clone()));
        }
        if let Some(assigned) = &self.assigned {
            pairs.push(("assigned", assigned.clone()));
        }
        if let Some(search) = &self.search {
            pairs.push(("q", search.clone()));
        }
        if self.all {
            pairs.push(("all", "1".to_string()));
        }
        if self.subissues {
            pairs.push(("subissues", "1".to_string()));
        }
        let order = match order {
            Some(order) => Some(order.to_string()),
            None => self.order.clone(),
        };
        if let Some(order) = order {
            pairs.push(("order", order));
        }
        if pairs.is_empty() {
            return "/".to_string();
        }
        let query = pairs
            .iter()
            .map(|(key, value)| format!("{key}={}", percent_encode(value)))
            .collect::<Vec<_>>()
            .join("&");
        format!("/?{query}")
    }

    /// Toggle direction when re-sorting by the column already in use.
    fn order_href(&self, key: &str) -> String {
        let next = match self.order.as_deref() {
            Some(current) if current == key => format!("{key}.desc"),
            Some(current) if current == format!("{key}.desc") => key.to_string(),
            _ => key.to_string(),
        };
        self.href(Some(&next))
    }

    fn order_marker(&self, key: &str) -> &'static str {
        match self.order.as_deref() {
            Some(current) if current == key => " \u{2191}",
            Some(current) if current == format!("{key}.desc") => " \u{2193}",
            _ => "",
        }
    }

    /// True when the list can contain closed tickets, in which case we
    /// show a state column (the TUI's closed views do the same).
    fn shows_closed(&self) -> bool {
        self.all
            || self.status.as_deref() != Some("open") && self.status.is_some()
            || self.state.is_some()
    }
}

// -- HTML ------------------------------------------------------------------

fn list_page(page: &Page, query: &ListQuery, tickets: &[Ticket]) -> String {
    let mut body = String::new();
    body.push_str(&header(page, query));

    if tickets.is_empty() {
        body.push_str("<p class=\"empty\">No tickets match this view.</p>");
    } else {
        let show_state = query.shows_closed();
        body.push_str("<table class=\"tickets\"><thead><tr>");
        body.push_str(&format!(
            "<th class=\"id\">Id</th><th class=\"age\"><a href=\"{}\">Age{}</a></th>\
             <th class=\"prio\"><a href=\"{}\">P{}</a></th>",
            escape(&query.order_href("created")),
            query.order_marker("created"),
            escape(&query.order_href("priority")),
            query.order_marker("priority"),
        ));
        if show_state {
            body.push_str(&format!(
                "<th class=\"state\"><a href=\"{}\">State{}</a></th>",
                escape(&query.order_href("state")),
                query.order_marker("state"),
            ));
        }
        body.push_str(&format!(
            "<th class=\"title\"><a href=\"{}\">Title{}</a></th>\
             <th class=\"who\"><a href=\"{}\">Assigned{}</a></th><th class=\"tags\">Tags</th>",
            escape(&query.order_href("title")),
            query.order_marker("title"),
            escape(&query.order_href("assigned")),
            query.order_marker("assigned"),
        ));
        body.push_str("</tr></thead><tbody>");

        for ticket in tickets {
            body.push_str(&row(page, query, ticket, show_state));
        }
        body.push_str("</tbody></table>");
    }

    body.push_str(&format!(
        "<p class=\"count\">{} ticket{} \u{b7} <a href=\"{}\">JSON</a></p>",
        tickets.len(),
        if tickets.len() == 1 { "" } else { "s" },
        escape(&query.href(None).replacen('/', "/tickets.json", 1)),
    ));
    document(&format!("{} tickets", page.repo), &body)
}

fn row(page: &Page, query: &ListQuery, ticket: &Ticket, show_state: bool) -> String {
    let assigned = ticket
        .assigned
        .as_deref()
        .map(|email| render::display_name(email, Some(&page.nicks)))
        .unwrap_or_default();
    let mine = ticket.assigned.as_deref() == Some(page.current_user.as_str());
    let priority = ticket
        .priority
        .map(|priority| format!("p{priority}"))
        .unwrap_or_default();
    let children = if ticket.children.is_empty() {
        String::new()
    } else {
        format!(
            " <span class=\"children\">[+{}]</span>",
            ticket.children.len()
        )
    };

    let mut out = format!(
        "<tr class=\"{}\"><td class=\"id\"><a href=\"/t/{}\">{}</a></td>\
         <td class=\"age\">{}</td><td class=\"prio\">{}</td>",
        if ticket.status == TicketStatus::Closed {
            "closed"
        } else {
            "open"
        },
        escape(&ticket.short_id()),
        escape(&ticket.short_id()),
        escape(&relative_time(ticket.created_at, page.now)),
        escape(&priority),
    );
    if show_state {
        out.push_str(&format!(
            "<td class=\"state\"><span class=\"badge state-{}\">{}</span></td>",
            escape(ticket.state.as_str()),
            escape(ticket.state.as_str()),
        ));
    }
    out.push_str(&format!(
        "<td class=\"title\"><a href=\"/t/{}\">{}</a>{}</td>\
         <td class=\"who{}\">{}</td><td class=\"tags\">{}</td></tr>",
        escape(&ticket.short_id()),
        escape(&flatten(&ticket.title)),
        children,
        if mine { " mine" } else { "" },
        escape(&assigned),
        tag_chips(query, ticket),
    ));
    out
}

fn tag_chips(query: &ListQuery, ticket: &Ticket) -> String {
    ticket
        .tags
        .iter()
        .map(|tag| {
            let mut scoped = query.clone();
            if !scoped.tags.contains(tag) {
                scoped.tags.push(tag.clone());
            }
            format!(
                "<a class=\"tag tag-{}\" href=\"{}\">{}</a>",
                tag_hue(tag),
                escape(&scoped.href(None)),
                escape(tag)
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn header(page: &Page, query: &ListQuery) -> String {
    let views: [(&str, String); 4] = [
        ("Open", ListQuery::default().href(None)),
        (
            "Mine",
            ListQuery {
                assigned: Some(page.current_user.clone()),
                ..Default::default()
            }
            .href(None),
        ),
        (
            "Closed",
            ListQuery {
                status: Some("closed".to_string()),
                order: Some("created.desc".to_string()),
                ..Default::default()
            }
            .href(None),
        ),
        (
            "All",
            ListQuery {
                all: true,
                subissues: true,
                ..Default::default()
            }
            .href(None),
        ),
    ];
    let current = query.href(None);
    let nav = views
        .iter()
        .map(|(label, href)| {
            format!(
                "<a class=\"view{}\" href=\"{}\">{label}</a>",
                if *href == current { " active" } else { "" },
                escape(href)
            )
        })
        .collect::<Vec<_>>()
        .join("");

    let mut hidden = String::new();
    if let Some(status) = &query.status {
        hidden.push_str(&hidden_input("status", status));
    }
    if let Some(state) = &query.state {
        hidden.push_str(&hidden_input("state", state));
    }
    for tag in &query.tags {
        hidden.push_str(&hidden_input("tag", tag));
    }
    if let Some(assigned) = &query.assigned {
        hidden.push_str(&hidden_input("assigned", assigned));
    }
    if query.all {
        hidden.push_str(&hidden_input("all", "1"));
    }
    if query.subissues {
        hidden.push_str(&hidden_input("subissues", "1"));
    }

    format!(
        "<header><h1><a href=\"/\">{}</a></h1><nav>{nav}</nav>\
         <form method=\"get\" action=\"/\">{hidden}\
         <input type=\"search\" name=\"q\" placeholder=\"search\" value=\"{}\"></form></header>{}",
        escape(&page.repo),
        escape(query.search.as_deref().unwrap_or_default()),
        active_filters(query),
    )
}

/// Chips for whatever narrowing is active, each linking to itself removed.
fn active_filters(query: &ListQuery) -> String {
    let mut chips: Vec<String> = Vec::new();
    for tag in &query.tags {
        let mut without = query.clone();
        without.tags.retain(|t| t != tag);
        chips.push(filter_chip(&format!("tag:{tag}"), &without.href(None)));
    }
    if let Some(assigned) = &query.assigned {
        let mut without = query.clone();
        without.assigned = None;
        chips.push(filter_chip(
            &format!("assigned:{assigned}"),
            &without.href(None),
        ));
    }
    if let Some(search) = &query.search {
        let mut without = query.clone();
        without.search = None;
        chips.push(filter_chip(
            &format!("search:{search}"),
            &without.href(None),
        ));
    }
    if chips.is_empty() {
        return String::new();
    }
    format!("<div class=\"filters\">{}</div>", chips.join(""))
}

fn detail_page(page: &Page, ticket: &Ticket) -> String {
    let mut body = String::new();
    body.push_str(&format!(
        "<header class=\"detail\"><a class=\"back\" href=\"/\">\u{2190} all tickets</a>\
         <h1>{}</h1><p class=\"subtitle\"><span class=\"badge state-{}\">{}</span> \
         <code>{}</code> \u{b7} opened {} ago by {}</p></header>",
        escape(&ticket.title),
        escape(ticket.state.as_str()),
        escape(ticket.state.as_str()),
        escape(&ticket.short_id()),
        escape(&relative_time(ticket.created_at, page.now)),
        escape(&render::display_name(&ticket.created_by, Some(&page.nicks))),
    ));

    let mut fields: Vec<(&str, String)> = Vec::new();
    fields.push(("Status", ticket.status.as_str().to_string()));
    if let Some(assigned) = &ticket.assigned {
        fields.push((
            "Assigned",
            render::display_name(assigned, Some(&page.nicks)),
        ));
    }
    if let Some(priority) = ticket.priority {
        fields.push(("Priority", priority.to_string()));
    }
    if let Some(points) = ticket.points {
        fields.push(("Points", points.to_string()));
    }
    if let Some(milestone) = &ticket.milestone {
        fields.push(("Milestone", milestone.clone()));
    }
    if let Some(code) = &ticket.code {
        fields.push(("Code", code.clone()));
    }
    if !ticket.tags.is_empty() {
        fields.push((
            "Tags",
            ticket.tags.iter().cloned().collect::<Vec<_>>().join(", "),
        ));
    }
    if let Some(parent) = ticket.parent {
        fields.push(("Parent", short_uuid(&parent)));
    }
    if !ticket.children.is_empty() {
        fields.push(("Sub-issues", join_uuids(&ticket.children)));
    }
    if !ticket.depends_on.is_empty() {
        fields.push(("Depends on", join_uuids(&ticket.depends_on)));
    }
    if !ticket.blocks.is_empty() {
        fields.push(("Blocks", join_uuids(&ticket.blocks)));
    }
    fields.push((
        "Created",
        ticket
            .created_at
            .format(&Rfc3339)
            .unwrap_or_else(|_| ticket.created_at.to_string()),
    ));
    for (key, value) in &ticket.meta {
        fields.push((key.as_str(), value.clone()));
    }

    body.push_str("<dl class=\"fields\">");
    for (label, value) in fields {
        body.push_str(&format!(
            "<div><dt>{}</dt><dd>{}</dd></div>",
            escape(label),
            escape(&value)
        ));
    }
    body.push_str("</dl>");

    if let Some(description) = ticket
        .description
        .as_deref()
        .filter(|d| !d.trim().is_empty())
    {
        body.push_str(&format!(
            "<section><h2>Description</h2><pre class=\"prose\">{}</pre></section>",
            escape(description)
        ));
    }
    if let Some(spec) = ticket.spec.as_deref().filter(|s| !s.trim().is_empty()) {
        body.push_str(&format!(
            "<section><h2>Spec</h2><pre class=\"prose\">{}</pre></section>",
            escape(spec)
        ));
    }
    if !ticket.comments.is_empty() {
        body.push_str(&format!(
            "<section><h2>Comments ({})</h2>",
            ticket.comments.len()
        ));
        for comment in &ticket.comments {
            body.push_str(&format!(
                "<article class=\"comment\"><p class=\"byline\">{} \u{b7} {} ago</p>\
                 <pre class=\"prose\">{}</pre></article>",
                escape(&render::display_name(&comment.author, Some(&page.nicks))),
                escape(&relative_time(comment.at, page.now)),
                escape(&comment.body),
            ));
        }
        body.push_str("</section>");
    }

    document(
        &format!("{} \u{b7} {}", ticket.short_id(), ticket.title),
        &body,
    )
}

fn short_uuid(id: &uuid::Uuid) -> String {
    id.to_string().chars().take(6).collect()
}

fn join_uuids(ids: &std::collections::BTreeSet<uuid::Uuid>) -> String {
    ids.iter().map(short_uuid).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::NickMap;
    use std::collections::{BTreeMap, BTreeSet};
    use ticgit_lib::TicketState;
    use time::OffsetDateTime;
    use uuid::Uuid;

    fn ticket(id: &str, title: &str, state: TicketState) -> Ticket {
        Ticket {
            id: Uuid::parse_str(id).unwrap(),
            title: title.to_string(),
            description: None,
            spec: None,
            status: state.status(),
            state,
            assigned: None,
            closed_by: None,
            priority: None,
            points: None,
            milestone: None,
            code: None,
            parent: None,
            children: BTreeSet::new(),
            depends_on: BTreeSet::new(),
            blocks: BTreeSet::new(),
            tags: BTreeSet::new(),
            meta: BTreeMap::new(),
            comments: vec![],
            created_at: OffsetDateTime::UNIX_EPOCH,
            created_by: "tester@example.com".into(),
        }
    }

    fn page() -> Page {
        Page {
            repo: "ticgit".to_string(),
            current_user: "tester@example.com".to_string(),
            nicks: NickMap::new(),
            now: OffsetDateTime::UNIX_EPOCH,
        }
    }

    fn request(target: &str) -> Request {
        super::super::parse_request_line(&format!("GET {target} HTTP/1.1\r\n")).unwrap()
    }

    #[test]
    fn query_defaults_to_open_tickets_without_subissues() {
        let filter = ListQuery::from_request(&request("/")).filter().unwrap();
        assert_eq!(filter.status, Some(TicketStatus::Open));
        assert!(filter.hide_subissues);
    }

    #[test]
    fn query_all_clears_status_and_shows_subissues() {
        let filter = ListQuery::from_request(&request("/?all=1&subissues=1"))
            .filter()
            .unwrap();
        assert_eq!(filter.status, None);
        assert!(!filter.hide_subissues);
    }

    #[test]
    fn query_state_narrows_status_and_state() {
        let filter = ListQuery::from_request(&request("/?state=blocked"))
            .filter()
            .unwrap();
        assert_eq!(filter.status, Some(TicketStatus::Open));
        assert_eq!(filter.state, Some(TicketState::Blocked));
    }

    #[test]
    fn query_rejects_unknown_status() {
        assert!(ListQuery::from_request(&request("/?status=frob"))
            .filter()
            .is_err());
    }

    #[test]
    fn href_round_trips_through_the_request_parser() {
        let query = ListQuery::from_request(&request("/?tag=bug&q=parser+bug&order=priority"));
        let reparsed = ListQuery::from_request(&request(&query.href(None)));
        assert_eq!(query, reparsed);
    }

    #[test]
    fn order_href_toggles_direction_for_the_active_column() {
        let query = ListQuery::from_request(&request("/?order=priority"));
        assert!(query.order_href("priority").contains("order=priority.desc"));
        let desc = ListQuery::from_request(&request("/?order=priority.desc"));
        assert!(desc.order_href("priority").ends_with("order=priority"));
    }

    #[test]
    fn list_page_renders_rows_and_links_to_detail() {
        let mut open = ticket(
            "d7f2d8f6-d6ec-3da1-a180-0a33fb090d59",
            "fix parser",
            TicketState::New,
        );
        open.priority = Some(2);
        open.tags.insert("bug".to_string());
        let html = list_page(&page(), &ListQuery::default(), &[open]);
        assert!(html.contains("href=\"/t/d7f2d8\""));
        assert!(html.contains("fix parser"));
        assert!(html.contains("p2"));
        assert!(html.contains(">bug</a>"));
        assert!(html.contains("1 ticket "));
    }

    #[test]
    fn list_page_shows_state_column_only_when_closed_tickets_can_appear() {
        let t = ticket(
            "d7f2d8f6-d6ec-3da1-a180-0a33fb090d59",
            "x",
            TicketState::New,
        );
        let open_view = list_page(&page(), &ListQuery::default(), std::slice::from_ref(&t));
        assert!(!open_view.contains("class=\"state\""));

        let all = ListQuery {
            all: true,
            ..Default::default()
        };
        assert!(list_page(&page(), &all, &[t]).contains("class=\"state\""));
    }

    #[test]
    fn html_is_escaped_in_titles_and_tags() {
        let mut t = ticket(
            "d7f2d8f6-d6ec-3da1-a180-0a33fb090d59",
            "<script>alert(1)</script>",
            TicketState::New,
        );
        t.tags.insert("a\"b".to_string());
        let html = list_page(&page(), &ListQuery::default(), &[t]);
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("a&quot;b"));
    }

    #[test]
    fn detail_page_shows_fields_description_and_comments() {
        let mut t = ticket(
            "d7f2d8f6-d6ec-3da1-a180-0a33fb090d59",
            "fix parser",
            TicketState::InProgress,
        );
        t.description = Some("a longer\nexplanation".to_string());
        t.assigned = Some("tester@example.com".to_string());
        t.comments.push(ticgit_lib::Comment {
            author: "other@example.com".to_string(),
            at: OffsetDateTime::UNIX_EPOCH,
            body: "on it".to_string(),
        });
        let html = detail_page(&page(), &t);
        assert!(html.contains("fix parser"));
        assert!(html.contains("in-progress"));
        assert!(html.contains("a longer\nexplanation"));
        assert!(html.contains("Comments (1)"));
        assert!(html.contains("on it"));
    }
}
