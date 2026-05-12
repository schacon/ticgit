//! Filtering and sorting for `ti list`.
//!
//! Mirrors the legacy CLI's `-s STATE`, `-t TAG`, `-a ASSIGNED`, `-T`,
//! `-o ORDER` selectors, with a stable, testable semantics.

use std::cmp::Ordering;

use crate::ticket::{Ticket, TicketState, TicketStatus};

/// All knobs `ti list` understands. Build one by parsing CLI flags and
/// pass it through [`apply`].
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub status: Option<TicketStatus>,
    pub state: Option<TicketState>,
    pub tag: Option<String>,
    pub tags: Vec<String>,
    pub tag_match_all: bool,
    pub assigned: Option<String>,
    pub only_tagged: bool,
    pub search: Option<SearchFilter>,
    pub order: Option<SortOrder>,
    /// When true, exclude tickets that have a parent (i.e. sub-issues).
    pub hide_subissues: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchFilter {
    pub scope: SearchScope,
    pub needle: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchScope {
    Any,
    Title,
    Description,
    Comments,
}

/// Sort orders accepted by `ti list -o`. Each can be inverted with the
/// `desc` flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Priority,
    Title,
    State,
    Assigned,
    Created,
}

#[derive(Debug, Clone, Copy)]
pub struct SortOrder {
    pub key: SortKey,
    pub desc: bool,
}

impl SortKey {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "priority" | "prio" => Some(SortKey::Priority),
            "title" => Some(SortKey::Title),
            "state" => Some(SortKey::State),
            "assigned" => Some(SortKey::Assigned),
            "created" | "date" | "time" => Some(SortKey::Created),
            _ => None,
        }
    }
}

impl SortOrder {
    /// Parse a `key[.desc]` spec like `state.desc` or `created`.
    pub fn parse(spec: &str) -> Option<Self> {
        let (key_str, desc) = match spec.split_once('.') {
            Some((k, "desc")) => (k, true),
            Some((k, "asc")) => (k, false),
            _ => (spec, false),
        };
        Some(SortOrder {
            key: SortKey::parse(key_str)?,
            desc,
        })
    }
}

impl SearchFilter {
    pub fn parse(spec: &str) -> Result<Self, String> {
        let spec = spec.trim();
        if spec.is_empty() {
            return Ok(SearchFilter {
                scope: SearchScope::Any,
                needle: String::new(),
            });
        }

        if let Some((scope, needle)) = spec.split_once(':') {
            if let Some(scope) = SearchScope::parse(scope) {
                return Ok(SearchFilter {
                    scope,
                    needle: needle.to_ascii_lowercase(),
                });
            }
        }

        Ok(SearchFilter {
            scope: SearchScope::Any,
            needle: spec.to_ascii_lowercase(),
        })
    }

    fn matches(&self, ticket: &Ticket) -> bool {
        if self.needle.is_empty() {
            return true;
        }

        match self.scope {
            SearchScope::Any => {
                contains(&ticket.title, &self.needle)
                    || ticket
                        .description
                        .as_deref()
                        .is_some_and(|description| contains(description, &self.needle))
                    || ticket
                        .comments
                        .iter()
                        .any(|comment| contains(&comment.body, &self.needle))
            }
            SearchScope::Title => contains(&ticket.title, &self.needle),
            SearchScope::Description => ticket
                .description
                .as_deref()
                .is_some_and(|description| contains(description, &self.needle)),
            SearchScope::Comments => ticket
                .comments
                .iter()
                .any(|comment| contains(&comment.body, &self.needle)),
        }
    }
}

impl SearchScope {
    fn parse(scope: &str) -> Option<Self> {
        match scope.trim().to_ascii_lowercase().as_str() {
            "title" => Some(SearchScope::Title),
            "description" | "desc" => Some(SearchScope::Description),
            "comment" | "comments" => Some(SearchScope::Comments),
            _ => None,
        }
    }
}

/// Filter and sort `tickets` according to `filter`. Returns a new vec.
pub fn apply(tickets: Vec<Ticket>, filter: &Filter) -> Vec<Ticket> {
    let mut tickets: Vec<Ticket> = tickets
        .into_iter()
        .filter(|t| {
            if let Some(state) = filter.state {
                if t.state != state {
                    return false;
                }
            }
            if let Some(status) = filter.status {
                if t.status != status {
                    return false;
                }
            }
            let tags = filter_tags(filter);
            if !tags.is_empty() {
                let matches = if filter.tag_match_all {
                    tags.iter().all(|tag| t.tags.contains(*tag))
                } else {
                    tags.iter().any(|tag| t.tags.contains(*tag))
                };
                if !matches {
                    return false;
                }
            }
            if let Some(assigned) = &filter.assigned {
                if t.assigned.as_deref() != Some(assigned.as_str()) {
                    return false;
                }
            }
            if filter.only_tagged && t.tags.is_empty() {
                return false;
            }
            if let Some(search) = &filter.search {
                if !search.matches(t) {
                    return false;
                }
            }
            if filter.hide_subissues && t.parent.is_some() {
                return false;
            }
            true
        })
        .collect();

    if let Some(order) = filter.order {
        tickets.sort_by(|a, b| compare(a, b, order.key, order.desc));
    } else {
        // Stable default: open first, then by priority (lower = more important),
        // then by recency (newer first).
        tickets.sort_by(|a, b| {
            let by_status = status_rank(a.status).cmp(&status_rank(b.status));
            if by_status != Ordering::Equal {
                return by_status;
            }
            let by_priority = priority_rank(a.priority).cmp(&priority_rank(b.priority));
            if by_priority != Ordering::Equal {
                return by_priority;
            }
            b.created_at.cmp(&a.created_at)
        });
    }

    tickets
}

fn filter_tags(filter: &Filter) -> Vec<&String> {
    let mut tags = Vec::new();
    if let Some(tag) = &filter.tag {
        tags.push(tag);
    }
    for tag in &filter.tags {
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    tags
}

fn contains(haystack: &str, needle: &str) -> bool {
    haystack.to_ascii_lowercase().contains(needle)
}

fn state_rank(s: TicketState) -> u8 {
    match s {
        TicketState::New => 0,
        TicketState::Assigned => 1,
        TicketState::InProgress => 2,
        TicketState::Blocked => 3,
        TicketState::Review => 4,
        TicketState::Resolved => 5,
        TicketState::Wontfix => 6,
        TicketState::Duplicate => 7,
        TicketState::Invalid => 8,
    }
}

/// Tickets with a priority sort before those without; among prioritised
/// tickets, lower numbers come first (1 = most important).
fn priority_rank(p: Option<i64>) -> (u8, i64) {
    match p {
        Some(v) => (0, v),
        None => (1, 0),
    }
}

fn status_rank(s: TicketStatus) -> u8 {
    match s {
        TicketStatus::Open => 0,
        TicketStatus::Closed => 1,
    }
}

fn compare(a: &Ticket, b: &Ticket, key: SortKey, desc: bool) -> Ordering {
    let ord = match key {
        SortKey::Priority => priority_rank(a.priority)
            .cmp(&priority_rank(b.priority))
            .then_with(|| b.created_at.cmp(&a.created_at))
            .then_with(|| a.id.cmp(&b.id)),
        SortKey::Title => a.title.cmp(&b.title),
        SortKey::State => status_rank(a.status)
            .cmp(&status_rank(b.status))
            .then_with(|| state_rank(a.state).cmp(&state_rank(b.state))),
        SortKey::Assigned => a
            .assigned
            .as_deref()
            .unwrap_or("")
            .cmp(b.assigned.as_deref().unwrap_or("")),
        SortKey::Created => a.created_at.cmp(&b.created_at),
    };
    if desc {
        ord.reverse()
    } else {
        ord
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ticket::Comment;
    use std::collections::{BTreeMap, BTreeSet};
    use time::OffsetDateTime;
    use uuid::Uuid;

    fn t(
        title: &str,
        status: TicketStatus,
        state: TicketState,
        tag: Option<&str>,
        assigned: Option<&str>,
        ts: i64,
    ) -> Ticket {
        let mut tags = BTreeSet::new();
        if let Some(s) = tag {
            tags.insert(s.to_string());
        }
        Ticket {
            id: Uuid::new_v4(),
            title: title.into(),
            description: None,
            spec: None,
            status,
            state,
            assigned: assigned.map(String::from),
            closed_by: None,
            priority: None,
            points: None,
            milestone: None,
            code: None,
            parent: None,
            children: BTreeSet::new(),
            depends_on: BTreeSet::new(),
            blocks: BTreeSet::new(),
            tags,
            meta: BTreeMap::new(),
            comments: vec![],
            created_at: OffsetDateTime::from_unix_timestamp(ts).unwrap(),
            created_by: "tester".into(),
        }
    }

    #[test]
    fn filter_by_state() {
        let input = vec![
            t("a", TicketStatus::Open, TicketState::New, None, None, 1),
            t(
                "b",
                TicketStatus::Closed,
                TicketState::Resolved,
                None,
                None,
                2,
            ),
        ];
        let f = Filter {
            state: Some(TicketState::New),
            ..Default::default()
        };
        let out = apply(input, &f);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].title, "a");
    }

    #[test]
    fn filter_by_status() {
        let input = vec![
            t("a", TicketStatus::Open, TicketState::Blocked, None, None, 1),
            t(
                "b",
                TicketStatus::Closed,
                TicketState::Resolved,
                None,
                None,
                2,
            ),
        ];
        let f = Filter {
            status: Some(TicketStatus::Open),
            ..Default::default()
        };
        let out = apply(input, &f);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].title, "a");
    }

    #[test]
    fn filter_by_tag() {
        let input = vec![
            t(
                "a",
                TicketStatus::Open,
                TicketState::New,
                Some("bug"),
                None,
                1,
            ),
            t(
                "b",
                TicketStatus::Open,
                TicketState::New,
                Some("ui"),
                None,
                2,
            ),
        ];
        let f = Filter {
            tag: Some("ui".into()),
            ..Default::default()
        };
        let out = apply(input, &f);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].title, "b");
    }

    #[test]
    fn filter_by_any_tag() {
        let mut bug = t(
            "bug",
            TicketStatus::Open,
            TicketState::New,
            Some("bug"),
            None,
            1,
        );
        bug.tags.insert("cli".into());
        let ui = t(
            "ui",
            TicketStatus::Open,
            TicketState::New,
            Some("ui"),
            None,
            2,
        );
        let docs = t(
            "docs",
            TicketStatus::Open,
            TicketState::New,
            Some("docs"),
            None,
            3,
        );
        let f = Filter {
            tags: vec!["bug".into(), "ui".into()],
            tag_match_all: false,
            ..Default::default()
        };
        let out = apply(vec![bug, ui, docs], &f);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].title, "ui");
        assert_eq!(out[1].title, "bug");
    }

    #[test]
    fn filter_by_all_tags() {
        let mut both = t(
            "both",
            TicketStatus::Open,
            TicketState::New,
            Some("bug"),
            None,
            1,
        );
        both.tags.insert("ui".into());
        let bug = t(
            "bug",
            TicketStatus::Open,
            TicketState::New,
            Some("bug"),
            None,
            2,
        );
        let f = Filter {
            tags: vec!["bug".into(), "ui".into()],
            tag_match_all: true,
            ..Default::default()
        };
        let out = apply(vec![both, bug], &f);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].title, "both");
    }

    #[test]
    fn filter_by_assigned() {
        let input = vec![
            t(
                "a",
                TicketStatus::Open,
                TicketState::New,
                None,
                Some("alice@x"),
                1,
            ),
            t(
                "b",
                TicketStatus::Open,
                TicketState::New,
                None,
                Some("bob@x"),
                2,
            ),
        ];
        let f = Filter {
            assigned: Some("bob@x".into()),
            ..Default::default()
        };
        let out = apply(input, &f);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].title, "b");
    }

    #[test]
    fn only_tagged_filters_untagged() {
        let input = vec![
            t(
                "untagged",
                TicketStatus::Open,
                TicketState::New,
                None,
                None,
                1,
            ),
            t(
                "tagged",
                TicketStatus::Open,
                TicketState::New,
                Some("bug"),
                None,
                2,
            ),
        ];
        let f = Filter {
            only_tagged: true,
            ..Default::default()
        };
        let out = apply(input, &f);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].title, "tagged");
    }

    #[test]
    fn default_order_puts_open_first_then_priority_then_newer_first() {
        let mut high_pri = t(
            "high-pri",
            TicketStatus::Open,
            TicketState::New,
            None,
            None,
            1,
        );
        high_pri.priority = Some(1);
        let mut low_pri = t(
            "low-pri",
            TicketStatus::Open,
            TicketState::New,
            None,
            None,
            50,
        );
        low_pri.priority = Some(3);
        let no_pri_new = t(
            "no-pri-new",
            TicketStatus::Open,
            TicketState::New,
            None,
            None,
            80,
        );
        let no_pri_old = t(
            "no-pri-old",
            TicketStatus::Open,
            TicketState::New,
            None,
            None,
            10,
        );
        let closed = t(
            "closed",
            TicketStatus::Closed,
            TicketState::Resolved,
            None,
            None,
            100,
        );
        let input = vec![
            no_pri_old.clone(),
            closed.clone(),
            low_pri.clone(),
            no_pri_new.clone(),
            high_pri.clone(),
        ];
        let out = apply(input, &Filter::default());
        // Open before closed, priority 1 before 3, prioritised before unprioritised,
        // then newer before older.
        assert_eq!(out[0].title, "high-pri");
        assert_eq!(out[1].title, "low-pri");
        assert_eq!(out[2].title, "no-pri-new");
        assert_eq!(out[3].title, "no-pri-old");
        assert_eq!(out[4].title, "closed");
    }

    #[test]
    fn sort_by_title_desc() {
        let input = vec![
            t("alpha", TicketStatus::Open, TicketState::New, None, None, 1),
            t("beta", TicketStatus::Open, TicketState::New, None, None, 2),
            t("gamma", TicketStatus::Open, TicketState::New, None, None, 3),
        ];
        let f = Filter {
            order: Some(SortOrder {
                key: SortKey::Title,
                desc: true,
            }),
            ..Default::default()
        };
        let out = apply(input, &f);
        assert_eq!(out[0].title, "gamma");
        assert_eq!(out[2].title, "alpha");
    }

    #[test]
    fn sort_order_parse() {
        assert!(matches!(
            SortOrder::parse("title").unwrap().key,
            SortKey::Title
        ));
        let o = SortOrder::parse("state.desc").unwrap();
        assert_eq!(o.key, SortKey::State);
        assert!(o.desc);
        assert!(SortOrder::parse("nonsense").is_none());
    }

    #[test]
    fn search_matches_title_description_and_comments() {
        let title = t(
            "parser panic",
            TicketStatus::Open,
            TicketState::New,
            None,
            None,
            1,
        );
        let mut description = t("docs", TicketStatus::Open, TicketState::New, None, None, 2);
        description.description = Some("explain parser recovery".into());
        let mut comment = t("ui", TicketStatus::Open, TicketState::New, None, None, 3);
        comment.comments.push(Comment {
            author: "tester".into(),
            at: OffsetDateTime::UNIX_EPOCH,
            body: "parser fails on empty input".into(),
        });

        let out = apply(
            vec![title.clone(), description.clone(), comment.clone()],
            &Filter {
                search: Some(SearchFilter::parse("parser").unwrap()),
                ..Default::default()
            },
        );
        assert_eq!(out.len(), 3);

        let out = apply(
            vec![title.clone(), description.clone(), comment.clone()],
            &Filter {
                search: Some(SearchFilter::parse("title:parser").unwrap()),
                ..Default::default()
            },
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].title, title.title);

        let out = apply(
            vec![title, description, comment.clone()],
            &Filter {
                search: Some(SearchFilter::parse("comments:empty").unwrap()),
                ..Default::default()
            },
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].title, comment.title);
    }
}
