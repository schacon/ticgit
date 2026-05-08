//! Domain types for tickets, comments, and ticket states.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::{Error, Result};

/// All valid ticket states. Mirrors the legacy ticgit set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TicketState {
    Open,
    Resolved,
    Invalid,
    Hold,
}

impl TicketState {
    pub const ALL: &'static [TicketState] = &[
        TicketState::Open,
        TicketState::Resolved,
        TicketState::Invalid,
        TicketState::Hold,
    ];

    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            TicketState::Open => "open",
            TicketState::Resolved => "resolved",
            TicketState::Invalid => "invalid",
            TicketState::Hold => "hold",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "open" => Ok(TicketState::Open),
            "resolved" => Ok(TicketState::Resolved),
            "invalid" => Ok(TicketState::Invalid),
            "hold" => Ok(TicketState::Hold),
            other => Err(Error::InvalidState(other.to_string())),
        }
    }
}

impl fmt::Display for TicketState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single comment on a ticket.
///
/// `at` and `author` are recovered from the underlying git-meta `ListEntry`'s
/// timestamp and the JSON body we store in `ListEntry::value`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    pub author: String,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    pub body: String,
}

/// On-the-wire shape of a comment list entry. We JSON-encode this as
/// the `value` of a git-meta `ListEntry`; the timestamp lives on the
/// `ListEntry` itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CommentBody {
    pub author: String,
    pub body: String,
}

/// A ticket, fully hydrated from project-target metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ticket {
    pub id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub state: TicketState,
    pub assigned: Option<String>,
    pub points: Option<i64>,
    pub milestone: Option<String>,
    pub tags: BTreeSet<String>,
    pub comments: Vec<Comment>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    pub created_by: String,
}

impl Ticket {
    /// Short 6-char form of the UUID used in table output and as a
    /// human-friendly handle (e.g. `d7f2d8`).
    #[must_use]
    pub fn short_id(&self) -> String {
        let s = self.id.to_string();
        s.chars().take(6).collect()
    }

    /// The "@user" portion of an email-style assigned handle, or the
    /// raw value if it doesn't look like an email.
    #[must_use]
    pub fn assigned_short(&self) -> Option<String> {
        self.assigned.as_ref().map(|a| {
            a.split_once('@')
                .map(|(local, _)| local.to_string())
                .unwrap_or_else(|| a.clone())
        })
    }
}

/// Options accepted by [`crate::store::TicketStore::create`].
#[derive(Debug, Clone, Default)]
pub struct NewTicketOpts {
    pub comment: Option<String>,
    pub tags: Vec<String>,
    pub assigned: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticket_state_parse_round_trip() {
        for state in TicketState::ALL {
            assert_eq!(TicketState::parse(state.as_str()).unwrap(), *state);
        }
    }

    #[test]
    fn ticket_state_parse_rejects_garbage() {
        assert!(TicketState::parse("frob").is_err());
        assert!(TicketState::parse("").is_err());
    }

    #[test]
    fn short_id_is_six_chars() {
        let t = Ticket {
            id: Uuid::parse_str("d7f2d8f6-d6ec-3da1-a180-0a33fb090d59").unwrap(),
            title: "x".into(),
            description: None,
            state: TicketState::Open,
            assigned: None,
            points: None,
            milestone: None,
            tags: BTreeSet::new(),
            comments: vec![],
            created_at: OffsetDateTime::UNIX_EPOCH,
            created_by: "x".into(),
        };
        assert_eq!(t.short_id(), "d7f2d8");
    }

    #[test]
    fn assigned_short_strips_email_domain() {
        let t = Ticket {
            id: Uuid::nil(),
            title: "x".into(),
            description: None,
            state: TicketState::Open,
            assigned: Some("jeff.welling@gmail.com".into()),
            points: None,
            milestone: None,
            tags: BTreeSet::new(),
            comments: vec![],
            created_at: OffsetDateTime::UNIX_EPOCH,
            created_by: "x".into(),
        };
        assert_eq!(t.assigned_short().as_deref(), Some("jeff.welling"));
    }

    #[test]
    fn assigned_short_passes_through_non_email() {
        let t = Ticket {
            id: Uuid::nil(),
            title: "x".into(),
            description: None,
            state: TicketState::Open,
            assigned: Some("jdoe".into()),
            points: None,
            milestone: None,
            tags: BTreeSet::new(),
            comments: vec![],
            created_at: OffsetDateTime::UNIX_EPOCH,
            created_by: "x".into(),
        };
        assert_eq!(t.assigned_short().as_deref(), Some("jdoe"));
    }
}
