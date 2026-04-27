//! Filtering and sorting for `ti list`.
//!
//! Mirrors the legacy CLI's `-s STATE`, `-t TAG`, `-a ASSIGNED`, `-T`,
//! `-o ORDER` selectors, with a stable, testable semantics.

use std::cmp::Ordering;

use crate::ticket::{Ticket, TicketState};

/// All knobs `ti list` understands. Build one by parsing CLI flags and
/// pass it through [`apply`].
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub state: Option<TicketState>,
    pub tag: Option<String>,
    pub assigned: Option<String>,
    pub only_tagged: bool,
    pub order: Option<SortOrder>,
}

/// Sort orders accepted by `ti list -o`. Each can be inverted with the
/// `desc` flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
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
            if let Some(tag) = &filter.tag {
                if !t.tags.contains(tag) {
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
            true
        })
        .collect();

    if let Some(order) = filter.order {
        tickets.sort_by(|a, b| compare(a, b, order.key, order.desc));
    } else {
        // Stable default: open tickets first, then by created date desc.
        tickets.sort_by(|a, b| {
            let by_state = state_rank(a.state).cmp(&state_rank(b.state));
            if by_state != Ordering::Equal {
                return by_state;
            }
            b.created_at.cmp(&a.created_at)
        });
    }

    tickets
}

fn state_rank(s: TicketState) -> u8 {
    match s {
        TicketState::Open => 0,
        TicketState::Hold => 1,
        TicketState::Resolved => 2,
        TicketState::Invalid => 3,
    }
}

fn compare(a: &Ticket, b: &Ticket, key: SortKey, desc: bool) -> Ordering {
    let ord = match key {
        SortKey::Title => a.title.cmp(&b.title),
        SortKey::State => state_rank(a.state).cmp(&state_rank(b.state)),
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
    use std::collections::BTreeSet;
    use time::OffsetDateTime;
    use uuid::Uuid;

    fn t(
        title: &str,
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
            state,
            assigned: assigned.map(String::from),
            points: None,
            milestone: None,
            tags,
            comments: vec![],
            created_at: OffsetDateTime::from_unix_timestamp(ts).unwrap(),
            created_by: "tester".into(),
        }
    }

    #[test]
    fn filter_by_state() {
        let input = vec![
            t("a", TicketState::Open, None, None, 1),
            t("b", TicketState::Resolved, None, None, 2),
        ];
        let f = Filter {
            state: Some(TicketState::Open),
            ..Default::default()
        };
        let out = apply(input, &f);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].title, "a");
    }

    #[test]
    fn filter_by_tag() {
        let input = vec![
            t("a", TicketState::Open, Some("bug"), None, 1),
            t("b", TicketState::Open, Some("ui"), None, 2),
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
    fn filter_by_assigned() {
        let input = vec![
            t("a", TicketState::Open, None, Some("alice@x"), 1),
            t("b", TicketState::Open, None, Some("bob@x"), 2),
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
            t("untagged", TicketState::Open, None, None, 1),
            t("tagged", TicketState::Open, Some("bug"), None, 2),
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
    fn default_order_puts_open_first_then_newer_first() {
        let input = vec![
            t("old-open", TicketState::Open, None, None, 1),
            t("new-resolved", TicketState::Resolved, None, None, 100),
            t("new-open", TicketState::Open, None, None, 50),
        ];
        let out = apply(input, &Filter::default());
        assert_eq!(out[0].title, "new-open");
        assert_eq!(out[1].title, "old-open");
        assert_eq!(out[2].title, "new-resolved");
    }

    #[test]
    fn sort_by_title_desc() {
        let input = vec![
            t("alpha", TicketState::Open, None, None, 1),
            t("beta", TicketState::Open, None, None, 2),
            t("gamma", TicketState::Open, None, None, 3),
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
}
