use std::collections::BTreeSet;

use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteupStatus {
    Open,
    Closed,
}

impl WriteupStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            WriteupStatus::Open => "open",
            WriteupStatus::Closed => "closed",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "open" => Some(WriteupStatus::Open),
            "closed" => Some(WriteupStatus::Closed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WriteupVersion {
    pub author: String,
    pub at: OffsetDateTime,
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct Writeup {
    pub id: Uuid,
    pub title: String,
    pub status: WriteupStatus,
    pub created_at: OffsetDateTime,
    pub created_by: String,
    pub authors: BTreeSet<String>,
    pub tags: BTreeSet<String>,
    pub tickets: BTreeSet<Uuid>,
    pub versions: Vec<WriteupVersion>,
}

impl Writeup {
    #[must_use]
    pub fn short_id(&self) -> String {
        self.id.to_string()[..6].to_string()
    }

    #[must_use]
    pub fn latest_body(&self) -> Option<&str> {
        self.versions.last().map(|version| version.body.as_str())
    }
}

#[derive(Debug, Clone, Default)]
pub struct NewWriteupOpts {
    pub body: Option<String>,
    pub tags: Vec<String>,
    pub created_at: Option<OffsetDateTime>,
}
