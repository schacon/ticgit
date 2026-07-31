//! The writeup half of `ti serve`.
//!
//! Mirrors the ticket pages: a filterable list at `/writeups`, a detail
//! page at `/w/<id>` that can show any single version or the whole
//! history, and `/writeups.json` for scripting. Page chrome, escaping
//! and the HTTP types all come from the parent module so both halves of
//! the site look and behave the same.

use anyhow::Result;
use serde_json::json;
use ticgit_lib::{Writeup, WriteupStatus};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use super::{
    document, error_page, escape, filter_chip, flatten, hidden_input, percent_encode, section_link,
    tag_hue, Page, Request, Response,
};
use crate::commands::open_store;
use crate::render;
use crate::timefmt::relative_time;

// -- responses -------------------------------------------------------------

pub(super) fn list_response(request: &Request) -> Result<Response> {
    let store = open_store()?;
    let query = WriteupQuery::from_request(request);
    let writeups = query.apply(store.list_writeups()?)?;
    let page = Page::new(&store)?;
    Ok(Response::html(200, list_page(&page, &query, &writeups)))
}

pub(super) fn json_response(request: &Request) -> Result<Response> {
    let store = open_store()?;
    let query = WriteupQuery::from_request(request);
    let writeups = query.apply(store.list_writeups()?)?;
    let json: Vec<serde_json::Value> = writeups.iter().map(writeup_json).collect();
    Ok(Response::new(
        200,
        "application/json; charset=utf-8",
        serde_json::to_string_pretty(&json)?.into_bytes(),
    ))
}

pub(super) fn detail_response(request: &Request, reference: &str) -> Result<Response> {
    let store = open_store()?;
    let id = match store.resolve_writeup_id(reference) {
        Ok(id) => id,
        Err(err) => {
            return Ok(Response::html(
                404,
                error_page("404 - no such writeup", &err.to_string()),
            ))
        }
    };
    let writeup = store.load_writeup(&id)?;
    let page = Page::new(&store)?;
    Ok(Response::html(
        200,
        detail_page(&page, &writeup, VersionView::from_request(request)),
    ))
}

// -- query -----------------------------------------------------------------

/// The list filters we accept as query params. Mirrors `ti writeup list`,
/// plus the tag/author/search narrowing the ticket list already has.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct WriteupQuery {
    status: Option<String>,
    tags: Vec<String>,
    author: Option<String>,
    search: Option<String>,
    order: Option<String>,
    all: bool,
}

impl WriteupQuery {
    fn from_request(request: &Request) -> Self {
        let clean = |value: Option<&str>| {
            value
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        };
        Self {
            status: clean(request.param("status")),
            tags: request
                .param_values("tag")
                .into_iter()
                .filter(|tag| !tag.trim().is_empty())
                .collect(),
            author: clean(request.param("author")),
            search: clean(request.param("q")),
            order: clean(request.param("order")),
            all: request.flag("all"),
        }
    }

    /// Filter then sort. `ti writeup list` hides closed writeups unless
    /// asked, so the default view does too.
    fn apply(&self, writeups: Vec<Writeup>) -> Result<Vec<Writeup>> {
        let status = self.status_filter()?;
        let kept = writeups
            .into_iter()
            .filter(|writeup| self.matches(writeup, status))
            .collect();
        self.sorted(kept)
    }

    /// `None` means "any status".
    fn status_filter(&self) -> Result<Option<WriteupStatus>> {
        match self.status.as_deref() {
            Some("all") => Ok(None),
            Some(spec) => WriteupStatus::parse(spec)
                .map(Some)
                .ok_or_else(|| anyhow::anyhow!("unknown writeup status `{spec}`")),
            None if self.all => Ok(None),
            None => Ok(Some(WriteupStatus::Open)),
        }
    }

    fn matches(&self, writeup: &Writeup, status: Option<WriteupStatus>) -> bool {
        if let Some(status) = status {
            if writeup.status != status {
                return false;
            }
        }
        if !self.tags.iter().all(|tag| writeup.tags.contains(tag)) {
            return false;
        }
        if let Some(author) = &self.author {
            let mine = writeup.created_by.eq_ignore_ascii_case(author)
                || writeup
                    .authors
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(author));
            if !mine {
                return false;
            }
        }
        if let Some(search) = &self.search {
            if !haystack(writeup).contains(&search.to_lowercase()) {
                return false;
            }
        }
        true
    }

    fn sorted(&self, mut writeups: Vec<Writeup>) -> Result<Vec<Writeup>> {
        // No explicit order keeps the store's own ordering: priority
        // first, then newest.
        let Some(spec) = self.order.as_deref() else {
            return Ok(writeups);
        };
        let (key, desc) = match spec.strip_suffix(".desc") {
            Some(key) => (key, true),
            None => (spec, false),
        };
        match key {
            "created" => writeups.sort_by_key(|writeup| writeup.created_at),
            "updated" => writeups.sort_by_key(updated_at),
            "title" => writeups.sort_by_key(|writeup| writeup.title.to_lowercase()),
            "status" => writeups.sort_by_key(|writeup| writeup.status.as_str()),
            "versions" => writeups.sort_by_key(|writeup| writeup.versions.len()),
            "priority" => {
                writeups.sort_by_key(|writeup| (writeup.priority.is_none(), writeup.priority))
            }
            _ => anyhow::bail!("unknown sort order `{spec}`"),
        }
        if desc {
            writeups.reverse();
        }
        Ok(writeups)
    }

    /// Rebuild the query string, optionally replacing the sort order.
    fn href(&self, order: Option<&str>) -> String {
        let mut pairs: Vec<(&str, String)> = Vec::new();
        if let Some(status) = &self.status {
            pairs.push(("status", status.clone()));
        }
        for tag in &self.tags {
            pairs.push(("tag", tag.clone()));
        }
        if let Some(author) = &self.author {
            pairs.push(("author", author.clone()));
        }
        if let Some(search) = &self.search {
            pairs.push(("q", search.clone()));
        }
        if self.all {
            pairs.push(("all", "1".to_string()));
        }
        let order = match order {
            Some(order) => Some(order.to_string()),
            None => self.order.clone(),
        };
        if let Some(order) = order {
            pairs.push(("order", order));
        }
        if pairs.is_empty() {
            return "/writeups".to_string();
        }
        let query = pairs
            .iter()
            .map(|(key, value)| format!("{key}={}", percent_encode(value)))
            .collect::<Vec<_>>()
            .join("&");
        format!("/writeups?{query}")
    }

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

    /// True when the list can contain closed writeups, in which case we
    /// show a status column.
    fn shows_closed(&self) -> bool {
        self.all
            || self
                .status
                .as_deref()
                .is_some_and(|status| status != "open")
    }
}

/// Everything a search can match, lowercased once per writeup.
fn haystack(writeup: &Writeup) -> String {
    let mut out = writeup.title.to_lowercase();
    for tag in &writeup.tags {
        out.push(' ');
        out.push_str(&tag.to_lowercase());
    }
    for author in &writeup.authors {
        out.push(' ');
        out.push_str(&author.to_lowercase());
    }
    out.push(' ');
    out.push_str(&writeup.created_by.to_lowercase());
    for version in &writeup.versions {
        out.push(' ');
        out.push_str(&version.body.to_lowercase());
    }
    out
}

fn updated_at(writeup: &Writeup) -> OffsetDateTime {
    writeup
        .versions
        .last()
        .map_or(writeup.created_at, |version| version.at)
}

/// Which version(s) the detail page should render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VersionView {
    Latest,
    All,
    /// 1-based, as shown in the UI.
    One(usize),
}

impl VersionView {
    fn from_request(request: &Request) -> Self {
        if request.flag("all") {
            return VersionView::All;
        }
        match request.param("v").and_then(|v| v.parse::<usize>().ok()) {
            Some(n) if n > 0 => VersionView::One(n),
            _ => VersionView::Latest,
        }
    }
}

// -- HTML ------------------------------------------------------------------

fn list_page(page: &Page, query: &WriteupQuery, writeups: &[Writeup]) -> String {
    let mut body = String::new();
    body.push_str(&header(page, query));

    if writeups.is_empty() {
        body.push_str("<p class=\"empty\">No writeups match this view.</p>");
    } else {
        let show_status = query.shows_closed();
        body.push_str("<table class=\"tickets\"><thead><tr>");
        body.push_str(&format!(
            "<th class=\"id\">Id</th><th class=\"age\"><a href=\"{}\">Age{}</a></th>\
             <th class=\"prio\"><a href=\"{}\">P{}</a></th>",
            escape(&query.order_href("created")),
            query.order_marker("created"),
            escape(&query.order_href("priority")),
            query.order_marker("priority"),
        ));
        if show_status {
            body.push_str(&format!(
                "<th class=\"state\"><a href=\"{}\">Status{}</a></th>",
                escape(&query.order_href("status")),
                query.order_marker("status"),
            ));
        }
        body.push_str(&format!(
            "<th class=\"title\"><a href=\"{}\">Title{}</a></th>\
             <th class=\"who\">Authors</th>\
             <th class=\"vers\"><a href=\"{}\">V{}</a></th><th class=\"tags\">Tags</th>",
            escape(&query.order_href("title")),
            query.order_marker("title"),
            escape(&query.order_href("versions")),
            query.order_marker("versions"),
        ));
        body.push_str("</tr></thead><tbody>");
        for writeup in writeups {
            body.push_str(&row(page, query, writeup, show_status));
        }
        body.push_str("</tbody></table>");
    }

    body.push_str(&format!(
        "<p class=\"count\">{} writeup{} \u{b7} <a href=\"{}\">JSON</a></p>",
        writeups.len(),
        if writeups.len() == 1 { "" } else { "s" },
        escape(&query.href(None).replacen("/writeups", "/writeups.json", 1)),
    ));
    document(&format!("{} writeups", page.repo), &body)
}

fn row(page: &Page, query: &WriteupQuery, writeup: &Writeup, show_status: bool) -> String {
    let mine =
        writeup.authors.contains(&page.current_user) || writeup.created_by == page.current_user;
    let priority = writeup
        .priority
        .map(|priority| format!("p{priority}"))
        .unwrap_or_default();
    let tickets = if writeup.tickets.is_empty() {
        String::new()
    } else {
        format!(
            " <span class=\"children\">[\u{2192}{}]</span>",
            writeup.tickets.len()
        )
    };

    let mut out = format!(
        "<tr class=\"{}\"><td class=\"id\"><a href=\"/w/{}\">{}</a></td>\
         <td class=\"age\">{}</td><td class=\"prio\">{}</td>",
        if writeup.status == WriteupStatus::Closed {
            "closed"
        } else {
            "open"
        },
        escape(&writeup.short_id()),
        escape(&writeup.short_id()),
        escape(&relative_time(writeup.created_at, page.now)),
        escape(&priority),
    );
    if show_status {
        out.push_str(&format!(
            "<td class=\"state\"><span class=\"badge state-{}\">{}</span></td>",
            escape(writeup.status.as_str()),
            escape(writeup.status.as_str()),
        ));
    }
    out.push_str(&format!(
        "<td class=\"title\"><a href=\"/w/{}\">{}</a>{}</td>\
         <td class=\"who{}\">{}</td><td class=\"vers\">v{}</td><td class=\"tags\">{}</td></tr>",
        escape(&writeup.short_id()),
        escape(&flatten(&writeup.title)),
        tickets,
        if mine { " mine" } else { "" },
        escape(&authors_summary(page, writeup)),
        writeup.versions.len(),
        tag_chips(query, writeup),
    ));
    out
}

/// One name plus a count, so the column stays narrow on shared writeups.
fn authors_summary(page: &Page, writeup: &Writeup) -> String {
    let mut authors = writeup.authors.iter();
    let Some(first) = authors.next() else {
        return render::display_name(&writeup.created_by, Some(&page.nicks));
    };
    let name = render::display_name(first, Some(&page.nicks));
    match writeup.authors.len() {
        1 => name,
        n => format!("{name} +{}", n - 1),
    }
}

fn tag_chips(query: &WriteupQuery, writeup: &Writeup) -> String {
    writeup
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

fn header(page: &Page, query: &WriteupQuery) -> String {
    let views: [(&str, String); 3] = [
        ("Open", WriteupQuery::default().href(None)),
        (
            "Mine",
            WriteupQuery {
                author: Some(page.current_user.clone()),
                all: true,
                ..Default::default()
            }
            .href(None),
        ),
        (
            "All",
            WriteupQuery {
                all: true,
                ..Default::default()
            }
            .href(None),
        ),
    ];
    let current = query.href(None);
    let mut nav = views
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
    nav.push_str(&section_link("/", "Tickets"));

    let mut hidden = String::new();
    if let Some(status) = &query.status {
        hidden.push_str(&hidden_input("status", status));
    }
    for tag in &query.tags {
        hidden.push_str(&hidden_input("tag", tag));
    }
    if let Some(author) = &query.author {
        hidden.push_str(&hidden_input("author", author));
    }
    if query.all {
        hidden.push_str(&hidden_input("all", "1"));
    }

    format!(
        "<header><h1><a href=\"/writeups\">{} writeups</a></h1><nav>{nav}</nav>\
         <form method=\"get\" action=\"/writeups\">{hidden}\
         <input type=\"search\" name=\"q\" placeholder=\"search\" value=\"{}\"></form></header>{}",
        escape(&page.repo),
        escape(query.search.as_deref().unwrap_or_default()),
        active_filters(query),
    )
}

fn active_filters(query: &WriteupQuery) -> String {
    let mut chips: Vec<String> = Vec::new();
    for tag in &query.tags {
        let mut without = query.clone();
        without.tags.retain(|t| t != tag);
        chips.push(filter_chip(&format!("tag:{tag}"), &without.href(None)));
    }
    if let Some(author) = &query.author {
        let mut without = query.clone();
        without.author = None;
        chips.push(filter_chip(
            &format!("author:{author}"),
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

fn detail_page(page: &Page, writeup: &Writeup, view: VersionView) -> String {
    let mut body = String::new();
    body.push_str(&format!(
        "<header class=\"detail\"><a class=\"back\" href=\"/writeups\">\u{2190} all writeups</a>\
         <h1>{}</h1><p class=\"subtitle\"><span class=\"badge state-{}\">{}</span> \
         <code>{}</code> \u{b7} started {} ago by {}</p></header>",
        escape(&writeup.title),
        escape(writeup.status.as_str()),
        escape(writeup.status.as_str()),
        escape(&writeup.short_id()),
        escape(&relative_time(writeup.created_at, page.now)),
        escape(&render::display_name(
            &writeup.created_by,
            Some(&page.nicks)
        )),
    ));

    let mut fields: Vec<(&str, String)> = Vec::new();
    fields.push(("Status", writeup.status.as_str().to_string()));
    if let Some(priority) = writeup.priority {
        fields.push(("Priority", priority.to_string()));
    }
    fields.push((
        "Authors",
        writeup
            .authors
            .iter()
            .map(|author| render::display_name(author, Some(&page.nicks)))
            .collect::<Vec<_>>()
            .join(", "),
    ));
    if !writeup.tags.is_empty() {
        fields.push((
            "Tags",
            writeup.tags.iter().cloned().collect::<Vec<_>>().join(", "),
        ));
    }
    fields.push(("Versions", writeup.versions.len().to_string()));
    fields.push((
        "Created",
        writeup
            .created_at
            .format(&Rfc3339)
            .unwrap_or_else(|_| writeup.created_at.to_string()),
    ));
    if let Some(version) = writeup.versions.last() {
        fields.push((
            "Updated",
            format!("{} ago", relative_time(version.at, page.now)),
        ));
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

    if !writeup.tickets.is_empty() {
        body.push_str(&format!(
            "<section><h2>Tickets ({})</h2><ul class=\"links\">",
            writeup.tickets.len()
        ));
        for id in &writeup.tickets {
            let short: String = id.to_string().chars().take(6).collect();
            body.push_str(&format!(
                "<li><a href=\"/t/{}\"><code>{}</code></a></li>",
                escape(&short),
                escape(&short),
            ));
        }
        body.push_str("</ul></section>");
    }

    body.push_str(&versions_section(page, writeup, view));
    document(
        &format!("{} \u{b7} {}", writeup.short_id(), writeup.title),
        &body,
    )
}

fn versions_section(page: &Page, writeup: &Writeup, view: VersionView) -> String {
    if writeup.versions.is_empty() {
        return "<section><h2>Body</h2><p class=\"empty\">No versions yet.</p></section>"
            .to_string();
    }

    let latest = writeup.versions.len();
    let selected = match view {
        VersionView::All => None,
        VersionView::Latest => Some(latest),
        // Out-of-range asks fall back to the latest rather than 404ing.
        VersionView::One(n) => Some(n.min(latest)),
    };

    let mut out = String::from("<section><h2>Versions</h2>");
    if latest > 1 {
        out.push_str("<div class=\"versions\">");
        for number in 1..=latest {
            out.push_str(&format!(
                "<a class=\"vtab{}\" href=\"/w/{}?v={number}\">v{number}</a>",
                if selected == Some(number) {
                    " active"
                } else {
                    ""
                },
                escape(&writeup.short_id()),
            ));
        }
        out.push_str(&format!(
            "<a class=\"vtab{}\" href=\"/w/{}?all=1\">all</a>",
            if selected.is_none() { " active" } else { "" },
            escape(&writeup.short_id()),
        ));
        out.push_str("</div>");
    }

    let shown: Vec<usize> = match selected {
        Some(number) => vec![number],
        None => (1..=latest).collect(),
    };
    for number in shown {
        let version = &writeup.versions[number - 1];
        out.push_str(&format!(
            "<article class=\"comment\"><p class=\"byline\">v{number} \u{b7} {} \u{b7} {} ago</p>\
             <pre class=\"prose\">{}</pre></article>",
            escape(&render::display_name(&version.author, Some(&page.nicks))),
            escape(&relative_time(version.at, page.now)),
            escape(&version.body),
        ));
    }
    out.push_str("</section>");
    out
}

// -- JSON ------------------------------------------------------------------

fn writeup_json(writeup: &Writeup) -> serde_json::Value {
    json!({
        "id": writeup.id.to_string(),
        "short_id": writeup.short_id(),
        "title": writeup.title,
        "status": writeup.status.as_str(),
        "priority": writeup.priority,
        "created_at": timestamp(writeup.created_at),
        "created_by": writeup.created_by,
        "updated_at": timestamp(updated_at(writeup)),
        "authors": writeup.authors,
        "tags": writeup.tags,
        "tickets": writeup
            .tickets
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        "versions": writeup
            .versions
            .iter()
            .map(|version| json!({
                "author": version.author,
                "at": timestamp(version.at),
                "body": version.body,
            }))
            .collect::<Vec<_>>(),
    })
}

fn timestamp(at: OffsetDateTime) -> String {
    at.format(&Rfc3339).unwrap_or_else(|_| at.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::NickMap;
    use std::collections::BTreeSet;
    use ticgit_lib::WriteupVersion;
    use uuid::Uuid;

    fn writeup(id: &str, title: &str, status: WriteupStatus) -> Writeup {
        Writeup {
            id: Uuid::parse_str(id).unwrap(),
            title: title.to_string(),
            status,
            priority: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
            created_by: "tester@example.com".into(),
            authors: BTreeSet::from(["tester@example.com".to_string()]),
            tags: BTreeSet::new(),
            tickets: BTreeSet::new(),
            versions: vec![],
        }
    }

    fn version(body: &str) -> WriteupVersion {
        WriteupVersion {
            author: "tester@example.com".to_string(),
            at: OffsetDateTime::UNIX_EPOCH,
            body: body.to_string(),
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

    fn sample() -> Vec<Writeup> {
        let mut open = writeup(
            "11111111-1111-4111-8111-111111111111",
            "caching plan",
            WriteupStatus::Open,
        );
        open.tags.insert("perf".to_string());
        open.versions.push(version("first pass at the cache"));
        let mut closed = writeup(
            "22222222-2222-4222-8222-222222222222",
            "old idea",
            WriteupStatus::Closed,
        );
        closed.created_by = "other@example.com".into();
        closed.authors = BTreeSet::from(["other@example.com".to_string()]);
        vec![open, closed]
    }

    #[test]
    fn list_defaults_to_open_writeups() {
        let query = WriteupQuery::from_request(&request("/writeups"));
        let shown = query.apply(sample()).unwrap();
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].title, "caching plan");
    }

    #[test]
    fn all_includes_closed_writeups() {
        let query = WriteupQuery::from_request(&request("/writeups?all=1"));
        assert_eq!(query.apply(sample()).unwrap().len(), 2);
    }

    #[test]
    fn status_filter_selects_closed_and_rejects_garbage() {
        let closed = WriteupQuery::from_request(&request("/writeups?status=closed"));
        let shown = closed.apply(sample()).unwrap();
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].title, "old idea");

        let bogus = WriteupQuery::from_request(&request("/writeups?status=frob"));
        assert!(bogus.apply(sample()).is_err());
    }

    #[test]
    fn tag_author_and_search_narrow_the_list() {
        let tagged = WriteupQuery::from_request(&request("/writeups?tag=perf"));
        assert_eq!(tagged.apply(sample()).unwrap().len(), 1);

        let other =
            WriteupQuery::from_request(&request("/writeups?all=1&author=other@example.com"));
        assert_eq!(other.apply(sample()).unwrap()[0].title, "old idea");

        // Search reaches into version bodies, not just the title.
        let body = WriteupQuery::from_request(&request("/writeups?q=first+pass"));
        assert_eq!(body.apply(sample()).unwrap().len(), 1);
        let miss = WriteupQuery::from_request(&request("/writeups?q=nothing+here"));
        assert!(miss.apply(sample()).unwrap().is_empty());
    }

    #[test]
    fn order_sorts_and_rejects_unknown_keys() {
        let by_title = WriteupQuery::from_request(&request("/writeups?all=1&order=title"));
        let sorted = by_title.apply(sample()).unwrap();
        assert_eq!(sorted[0].title, "caching plan");

        let desc = WriteupQuery::from_request(&request("/writeups?all=1&order=title.desc"));
        assert_eq!(desc.apply(sample()).unwrap()[0].title, "old idea");

        let bogus = WriteupQuery::from_request(&request("/writeups?order=frob"));
        assert!(bogus.apply(sample()).is_err());
    }

    #[test]
    fn href_round_trips_through_the_request_parser() {
        let query =
            WriteupQuery::from_request(&request("/writeups?tag=perf&q=cache+plan&order=created"));
        let reparsed = WriteupQuery::from_request(&request(&query.href(None)));
        assert_eq!(query, reparsed);
        assert_eq!(WriteupQuery::default().href(None), "/writeups");
    }

    #[test]
    fn order_href_toggles_direction_for_the_active_column() {
        let query = WriteupQuery::from_request(&request("/writeups?order=created"));
        assert!(query.order_href("created").contains("order=created.desc"));
        let desc = WriteupQuery::from_request(&request("/writeups?order=created.desc"));
        assert!(desc.order_href("created").ends_with("order=created"));
    }

    #[test]
    fn list_page_renders_rows_and_links_to_detail() {
        let query = WriteupQuery::default();
        let html = list_page(&page(), &query, &query.apply(sample()).unwrap());
        assert!(html.contains("href=\"/w/111111\""));
        assert!(html.contains("caching plan"));
        assert!(html.contains(">perf</a>"));
        assert!(html.contains("v1"));
        assert!(html.contains("1 writeup "));
        // Status column only appears where closed writeups can show up.
        assert!(!html.contains("class=\"state\""));

        let all = WriteupQuery::from_request(&request("/writeups?all=1"));
        let html = list_page(&page(), &all, &all.apply(sample()).unwrap());
        assert!(html.contains("class=\"state\""));
        assert!(html.contains("2 writeups "));
    }

    #[test]
    fn html_is_escaped_in_titles_and_tags() {
        let mut w = writeup(
            "11111111-1111-4111-8111-111111111111",
            "<script>alert(1)</script>",
            WriteupStatus::Open,
        );
        w.tags.insert("a\"b".to_string());
        let html = list_page(&page(), &WriteupQuery::default(), &[w]);
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("a&quot;b"));
    }

    #[test]
    fn detail_page_shows_latest_version_by_default() {
        let mut w = writeup(
            "11111111-1111-4111-8111-111111111111",
            "caching plan",
            WriteupStatus::Open,
        );
        w.versions.push(version("first draft"));
        w.versions.push(version("second draft"));
        w.tickets
            .insert(Uuid::parse_str("d7f2d8f6-d6ec-3da1-a180-0a33fb090d59").unwrap());

        let html = detail_page(&page(), &w, VersionView::Latest);
        assert!(html.contains("caching plan"));
        assert!(html.contains("second draft"));
        assert!(!html.contains("first draft"));
        assert!(html.contains("href=\"/t/d7f2d8\""));
        assert!(html.contains("v2</a>"));
    }

    #[test]
    fn detail_page_can_show_one_version_or_all_of_them() {
        let mut w = writeup(
            "11111111-1111-4111-8111-111111111111",
            "caching plan",
            WriteupStatus::Open,
        );
        w.versions.push(version("first draft"));
        w.versions.push(version("second draft"));

        let first = detail_page(&page(), &w, VersionView::One(1));
        assert!(first.contains("first draft"));
        assert!(!first.contains("second draft"));

        let all = detail_page(&page(), &w, VersionView::All);
        assert!(all.contains("first draft"));
        assert!(all.contains("second draft"));

        // Out-of-range version numbers clamp to the latest.
        let clamped = detail_page(&page(), &w, VersionView::One(99));
        assert!(clamped.contains("second draft"));
        assert!(!clamped.contains("first draft"));
    }

    #[test]
    fn version_view_reads_the_query_string() {
        assert_eq!(
            VersionView::from_request(&request("/w/111111")),
            VersionView::Latest
        );
        assert_eq!(
            VersionView::from_request(&request("/w/111111?all=1")),
            VersionView::All
        );
        assert_eq!(
            VersionView::from_request(&request("/w/111111?v=3")),
            VersionView::One(3)
        );
        assert_eq!(
            VersionView::from_request(&request("/w/111111?v=nope")),
            VersionView::Latest
        );
    }

    #[test]
    fn json_carries_versions_and_links() {
        let mut w = writeup(
            "11111111-1111-4111-8111-111111111111",
            "caching plan",
            WriteupStatus::Open,
        );
        w.versions.push(version("first draft"));
        let json = writeup_json(&w);
        assert_eq!(json["short_id"], "111111");
        assert_eq!(json["status"], "open");
        assert_eq!(json["versions"][0]["body"], "first draft");
        assert_eq!(json["created_at"], "1970-01-01T00:00:00Z");
    }
}
