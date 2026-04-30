//! `TicketStore` - the bridge between the [`Ticket`] domain model and a
//! git-meta [`Session`].
//!
//! Every read and write goes through a [`SessionTargetHandle`] scoped to
//! the `project` target. There is no separate index; tickets are
//! discovered by prefix-scanning `ticgit:tickets`.

use std::collections::{BTreeMap, BTreeSet};

use git_meta_lib::{ListEntry, MetaValue, Session, Target};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::keys;
use crate::ticket::{Comment, CommentBody, NewTicketOpts, Ticket, TicketState};

/// Wraps a [`Session`] and exposes a ticket-shaped API on top of it.
pub struct TicketStore {
    session: Session,
}

impl TicketStore {
    /// Open a store for the git repo discovered from the current working
    /// directory.
    pub fn discover() -> Result<Self> {
        let session = Session::discover()?;
        Self::ensure_schema(&session)?;
        Ok(Self { session })
    }

    /// Open a store for an already-loaded `gix::Repository` (used in tests
    /// and by host applications that own the repo handle).
    pub fn open(repo: gix::Repository) -> Result<Self> {
        let session = Session::open(repo)?;
        Self::ensure_schema(&session)?;
        Ok(Self { session })
    }

    /// Open a store from an already-built session (lets callers preconfigure
    /// e.g. `with_timestamp` for deterministic tests).
    pub fn from_session(session: Session) -> Result<Self> {
        Self::ensure_schema(&session)?;
        Ok(Self { session })
    }

    /// Borrow the underlying git-meta session.
    #[must_use]
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// The user email this store will record on writes.
    #[must_use]
    pub fn email(&self) -> &str {
        self.session.email()
    }

    fn ensure_schema(session: &Session) -> Result<()> {
        let p = session.target(&Target::project());
        if p.get_value(keys::SCHEMA_VERSION_KEY)?.is_none() {
            p.set(keys::SCHEMA_VERSION_KEY, keys::SCHEMA_VERSION)?;
        }
        Ok(())
    }

    // -------------------------------------------------------------------
    // Ticket creation & loading
    // -------------------------------------------------------------------

    /// Create a new ticket. Returns the freshly-loaded ticket.
    pub fn create(&self, title: &str, opts: NewTicketOpts) -> Result<Ticket> {
        let id = Uuid::new_v4();
        let p = self.session.target(&Target::project());
        let now = OffsetDateTime::now_utc();
        let now_rfc = now
            .format(&Rfc3339)
            .map_err(|e| Error::Time(e.to_string()))?;

        // A ticket's existence is implied by its fields - no separate
        // index to maintain.
        p.set(&keys::ticket_field(&id, "title"), title)?;
        p.set(
            &keys::ticket_field(&id, "state"),
            TicketState::Open.as_str(),
        )?;
        p.set(&keys::ticket_field(&id, "created-at"), now_rfc.as_str())?;
        p.set(&keys::ticket_field(&id, "created-by"), self.session.email())?;

        if let Some(ref a) = opts.assigned {
            if !a.is_empty() {
                p.set(&keys::ticket_field(&id, "assigned"), a.as_str())?;
            }
        }

        if let Some(body) = opts.comment {
            self.push_comment(&p, &id, &body)?;
        }

        for tag in opts.tags {
            let tag = tag.trim();
            if !tag.is_empty() {
                p.set_add(&keys::ticket_field(&id, "tags"), tag)?;
            }
        }

        self.load(&id)
    }

    /// Load every ticket in the project in a single round-trip.
    pub fn list(&self) -> Result<Vec<Ticket>> {
        let p = self.session.target(&Target::project());
        let pairs = p.get_all_values(Some(&keys::tickets_prefix()))?;
        let mut by_id: BTreeMap<Uuid, Vec<(String, MetaValue)>> = BTreeMap::new();
        for (key, value) in pairs {
            if let Some((id, field)) = keys::parse_ticket_field(&key) {
                by_id
                    .entry(id)
                    .or_default()
                    .push((field.to_string(), value));
            }
        }

        let mut out = Vec::with_capacity(by_id.len());
        for (id, fields) in by_id {
            if let Some(t) = build_ticket(id, fields) {
                out.push(t);
            }
        }
        Ok(out)
    }

    /// Load a single ticket by exact UUID.
    pub fn load(&self, id: &Uuid) -> Result<Ticket> {
        let p = self.session.target(&Target::project());
        let pairs = p.get_all_values(Some(&keys::ticket_prefix(id)))?;
        let mut fields = Vec::with_capacity(pairs.len());
        for (key, value) in pairs {
            if let Some((parsed_id, field)) = keys::parse_ticket_field(&key) {
                if parsed_id == *id {
                    fields.push((field.to_string(), value));
                }
            }
        }
        build_ticket(*id, fields).ok_or(Error::NotFound(*id))
    }

    /// Resolve a user-supplied ticket reference (full UUID or unique
    /// prefix, hyphens optional, case-insensitive) into a real UUID.
    pub fn resolve_id(&self, reference: &str) -> Result<Uuid> {
        let needle = reference.trim().to_ascii_lowercase().replace('-', "");
        if needle.is_empty() {
            return Err(Error::NoMatch(reference.to_string()));
        }
        let tickets = self.list()?;
        let mut matches: Vec<Uuid> = tickets
            .iter()
            .filter_map(|t| {
                let hex = t.id.to_string().replace('-', "");
                if hex.starts_with(&needle) {
                    Some(t.id)
                } else {
                    None
                }
            })
            .collect();
        match matches.len() {
            0 => Err(Error::NoMatch(reference.to_string())),
            1 => Ok(matches.remove(0)),
            n => Err(Error::Ambiguous(reference.to_string(), n)),
        }
    }

    // -------------------------------------------------------------------
    // Field mutators
    // -------------------------------------------------------------------

    pub fn set_title(&self, id: &Uuid, title: &str) -> Result<()> {
        self.project_handle()
            .set(&keys::ticket_field(id, "title"), title)?;
        Ok(())
    }

    pub fn set_description(&self, id: &Uuid, description: Option<&str>) -> Result<()> {
        let p = self.project_handle();
        let key = keys::ticket_field(id, "description");
        match description {
            Some(d) if !d.is_empty() => {
                p.set(&key, d)?;
            }
            _ => {
                p.remove(&key)?;
            }
        }
        Ok(())
    }

    pub fn set_state(&self, id: &Uuid, state: TicketState) -> Result<()> {
        self.project_handle()
            .set(&keys::ticket_field(id, "state"), state.as_str())?;
        Ok(())
    }

    pub fn set_assigned(&self, id: &Uuid, who: Option<&str>) -> Result<()> {
        let p = self.project_handle();
        let key = keys::ticket_field(id, "assigned");
        match who {
            Some(w) if !w.is_empty() => {
                p.set(&key, w)?;
            }
            _ => {
                p.remove(&key)?;
            }
        }
        Ok(())
    }

    pub fn set_points(&self, id: &Uuid, points: Option<i64>) -> Result<()> {
        let p = self.project_handle();
        let key = keys::ticket_field(id, "points");
        match points {
            Some(n) => {
                p.set(&key, n.to_string().as_str())?;
            }
            None => {
                p.remove(&key)?;
            }
        }
        Ok(())
    }

    pub fn set_milestone(&self, id: &Uuid, milestone: Option<&str>) -> Result<()> {
        let p = self.project_handle();
        let key = keys::ticket_field(id, "milestone");
        match milestone {
            Some(m) if !m.is_empty() => {
                p.set(&key, m)?;
            }
            _ => {
                p.remove(&key)?;
            }
        }
        Ok(())
    }

    pub fn add_tag(&self, id: &Uuid, tag: &str) -> Result<()> {
        let tag = tag.trim();
        if tag.is_empty() {
            return Ok(());
        }
        self.project_handle()
            .set_add(&keys::ticket_field(id, "tags"), tag)?;
        Ok(())
    }

    pub fn remove_tag(&self, id: &Uuid, tag: &str) -> Result<()> {
        let tag = tag.trim();
        if tag.is_empty() {
            return Ok(());
        }
        self.project_handle()
            .set_remove(&keys::ticket_field(id, "tags"), tag)?;
        Ok(())
    }

    pub fn add_comment(&self, id: &Uuid, body: &str) -> Result<()> {
        let p = self.project_handle();
        self.push_comment(&p, id, body)?;
        Ok(())
    }

    fn push_comment(
        &self,
        handle: &git_meta_lib::SessionTargetHandle<'_>,
        id: &Uuid,
        body: &str,
    ) -> Result<()> {
        let payload = CommentBody {
            author: self.session.email().to_string(),
            body: body.to_string(),
        };
        let json = serde_json::to_string(&payload)?;
        handle.list_push(&keys::ticket_field(id, "comments"), &json)?;
        Ok(())
    }

    fn project_handle(&self) -> git_meta_lib::SessionTargetHandle<'_> {
        self.session.target(&Target::project())
    }

    // -------------------------------------------------------------------
    // Saved views (named, frozen sets of ticket UUIDs)
    // -------------------------------------------------------------------

    /// Save a snapshot of `ids` under the name `name`.
    /// Replaces any existing membership of that view.
    pub fn save_view(&self, name: &str, ids: &BTreeSet<Uuid>) -> Result<()> {
        let p = self.project_handle();
        let key = keys::view(name);
        if let Some(MetaValue::Set(existing)) = p.get_value(&key)? {
            for member in existing {
                p.set_remove(&key, &member)?;
            }
        }
        for id in ids {
            p.set_add(&key, &id.to_string())?;
        }
        Ok(())
    }

    /// Load the UUID set stored under view `name`.
    pub fn load_view(&self, name: &str) -> Result<BTreeSet<Uuid>> {
        let p = self.project_handle();
        match p.get_value(&keys::view(name))? {
            Some(MetaValue::Set(members)) => Ok(members
                .iter()
                .filter_map(|s| Uuid::parse_str(s).ok())
                .collect()),
            _ => Ok(BTreeSet::new()),
        }
    }

    /// List all view names defined on this project, alphabetised.
    pub fn list_views(&self) -> Result<Vec<String>> {
        let p = self.project_handle();
        let pairs = p.get_all_values(Some(&keys::views_prefix()))?;
        let mut names: Vec<String> = pairs
            .into_iter()
            .filter_map(|(k, _)| keys::parse_view_name(&k).map(String::from))
            .collect();
        names.sort();
        names.dedup();
        Ok(names)
    }

    // -------------------------------------------------------------------
    // System-wide ticgit metadata
    // -------------------------------------------------------------------

    pub fn add_owner(&self, who: &str) -> Result<()> {
        self.project_handle()
            .set_add(&keys::system_key("owners"), who.trim())?;
        Ok(())
    }

    pub fn remove_owner(&self, who: &str) -> Result<()> {
        self.project_handle()
            .set_remove(&keys::system_key("owners"), who.trim())?;
        Ok(())
    }

    pub fn list_owners(&self) -> Result<BTreeSet<String>> {
        let p = self.project_handle();
        match p.get_value(&keys::system_key("owners"))? {
            Some(MetaValue::Set(members)) => Ok(members),
            _ => Ok(BTreeSet::new()),
        }
    }

    pub fn schema_version(&self) -> Result<Option<String>> {
        let p = self.project_handle();
        match p.get_value(keys::SCHEMA_VERSION_KEY)? {
            Some(MetaValue::String(s)) => Ok(Some(s)),
            _ => Ok(None),
        }
    }

    // -------------------------------------------------------------------
    // Sync porcelain
    // -------------------------------------------------------------------

    pub fn serialize(&self) -> Result<()> {
        let _ = self.session.serialize()?;
        Ok(())
    }

    pub fn pull(&self, remote: Option<&str>) -> Result<()> {
        let _ = self.session.pull(remote)?;
        Ok(())
    }

    pub fn push(&self, remote: Option<&str>) -> Result<()> {
        let _ = self.session.push_once(remote)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Field-level deserialisation
// ---------------------------------------------------------------------------

fn build_ticket(id: Uuid, fields: Vec<(String, MetaValue)>) -> Option<Ticket> {
    if fields.is_empty() {
        return None;
    }

    let mut title: Option<String> = None;
    let mut description: Option<String> = None;
    let mut state = TicketState::Open;
    let mut assigned: Option<String> = None;
    let mut points: Option<i64> = None;
    let mut milestone: Option<String> = None;
    let mut tags: BTreeSet<String> = BTreeSet::new();
    let mut comments: Vec<Comment> = Vec::new();
    let mut created_at: Option<OffsetDateTime> = None;
    let mut created_by = String::new();

    for (field, value) in fields {
        match (field.as_str(), value) {
            ("title", MetaValue::String(s)) => title = Some(s),
            ("description", MetaValue::String(s)) => description = Some(s),
            ("state", MetaValue::String(s)) => {
                state = TicketState::parse(&s).unwrap_or(TicketState::Open);
            }
            ("assigned", MetaValue::String(s)) => assigned = Some(s),
            ("points", MetaValue::String(s)) => points = s.parse().ok(),
            ("milestone", MetaValue::String(s)) => milestone = Some(s),
            ("tags", MetaValue::Set(members)) => tags = members,
            ("comments", MetaValue::List(entries)) => comments = decode_comments(entries),
            ("created-at", MetaValue::String(s)) => {
                created_at = OffsetDateTime::parse(&s, &Rfc3339).ok();
            }
            ("created-by", MetaValue::String(s)) => created_by = s,
            _ => {}
        }
    }

    let title = title?;
    let created_at = created_at.unwrap_or(OffsetDateTime::UNIX_EPOCH);

    Some(Ticket {
        id,
        title,
        description,
        state,
        assigned,
        points,
        milestone,
        tags,
        comments,
        created_at,
        created_by,
    })
}

fn decode_comments(entries: Vec<ListEntry>) -> Vec<Comment> {
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        let at = OffsetDateTime::from_unix_timestamp_nanos(i128::from(entry.timestamp) * 1_000_000)
            .unwrap_or(OffsetDateTime::UNIX_EPOCH);

        let (author, body) = match serde_json::from_str::<CommentBody>(&entry.value) {
            Ok(c) => (c.author, c.body),
            // Tolerate raw-string bodies (older or hand-pushed entries).
            Err(_) => (String::from("unknown"), entry.value),
        };

        out.push(Comment { author, at, body });
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_store;

    #[test]
    fn create_and_load_round_trips() {
        let (store, _td) = test_store();
        let opts = NewTicketOpts {
            comment: Some("first comment".into()),
            tags: vec!["bug".into(), "ui".into()],
            assigned: Some("scott@example.com".into()),
        };
        let created = store.create("My new ticket", opts).unwrap();
        assert_eq!(created.title, "My new ticket");
        assert_eq!(created.state, TicketState::Open);
        assert_eq!(created.assigned.as_deref(), Some("scott@example.com"));
        assert!(created.tags.contains("bug"));
        assert!(created.tags.contains("ui"));
        assert_eq!(created.comments.len(), 1);
        assert_eq!(created.comments[0].body, "first comment");

        let again = store.load(&created.id).unwrap();
        assert_eq!(created, again);
    }

    #[test]
    fn list_returns_all_created_tickets() {
        let (store, _td) = test_store();
        store.create("first", NewTicketOpts::default()).unwrap();
        store.create("second", NewTicketOpts::default()).unwrap();
        let all = store.list().unwrap();
        assert_eq!(all.len(), 2);
        let titles: BTreeSet<_> = all.iter().map(|t| t.title.clone()).collect();
        assert!(titles.contains("first"));
        assert!(titles.contains("second"));
    }

    #[test]
    fn list_is_empty_for_fresh_repo() {
        let (store, _td) = test_store();
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn state_change_persists() {
        let (store, _td) = test_store();
        let t = store.create("x", NewTicketOpts::default()).unwrap();
        store.set_state(&t.id, TicketState::Resolved).unwrap();
        assert_eq!(store.load(&t.id).unwrap().state, TicketState::Resolved);
    }

    #[test]
    fn tag_add_and_remove() {
        let (store, _td) = test_store();
        let t = store.create("x", NewTicketOpts::default()).unwrap();
        store.add_tag(&t.id, "feature").unwrap();
        store.add_tag(&t.id, "ui").unwrap();
        assert_eq!(
            store.load(&t.id).unwrap().tags,
            ["feature", "ui"].iter().map(|s| s.to_string()).collect()
        );
        store.remove_tag(&t.id, "ui").unwrap();
        assert_eq!(
            store.load(&t.id).unwrap().tags,
            ["feature"].iter().map(|s| s.to_string()).collect()
        );
    }

    #[test]
    fn assigned_set_and_clear() {
        let (store, _td) = test_store();
        let t = store.create("x", NewTicketOpts::default()).unwrap();
        store.set_assigned(&t.id, Some("a@b.co")).unwrap();
        assert_eq!(
            store.load(&t.id).unwrap().assigned.as_deref(),
            Some("a@b.co")
        );
        store.set_assigned(&t.id, None).unwrap();
        assert_eq!(store.load(&t.id).unwrap().assigned, None);
    }

    #[test]
    fn points_set_and_clear() {
        let (store, _td) = test_store();
        let t = store.create("x", NewTicketOpts::default()).unwrap();
        store.set_points(&t.id, Some(5)).unwrap();
        assert_eq!(store.load(&t.id).unwrap().points, Some(5));
        store.set_points(&t.id, None).unwrap();
        assert_eq!(store.load(&t.id).unwrap().points, None);
    }

    #[test]
    fn comments_carry_author_and_arrive_in_order() {
        let (store, _td) = test_store();
        let t = store.create("x", NewTicketOpts::default()).unwrap();
        store.add_comment(&t.id, "one").unwrap();
        store.add_comment(&t.id, "two").unwrap();
        store.add_comment(&t.id, "three").unwrap();
        let loaded = store.load(&t.id).unwrap();
        let bodies: Vec<_> = loaded.comments.iter().map(|c| c.body.clone()).collect();
        assert_eq!(bodies, vec!["one", "two", "three"]);
        for c in &loaded.comments {
            assert_eq!(c.author, store.email());
        }
    }

    #[test]
    fn resolve_id_accepts_unique_prefix() {
        let (store, _td) = test_store();
        let t = store.create("x", NewTicketOpts::default()).unwrap();
        let prefix: String = t.id.to_string().chars().take(6).collect();
        assert_eq!(store.resolve_id(&prefix).unwrap(), t.id);
        // Hyphens optional & case-insensitive.
        let no_hyphen: String = t.id.to_string().replace('-', "").to_ascii_uppercase();
        assert_eq!(store.resolve_id(&no_hyphen).unwrap(), t.id);
    }

    #[test]
    fn resolve_id_reports_no_match() {
        let (store, _td) = test_store();
        store.create("x", NewTicketOpts::default()).unwrap();
        let err = store.resolve_id("ffffffff").unwrap_err();
        assert!(matches!(err, Error::NoMatch(_)));
    }

    #[test]
    fn views_round_trip() {
        let (store, _td) = test_store();
        let a = store.create("a", NewTicketOpts::default()).unwrap();
        let b = store.create("b", NewTicketOpts::default()).unwrap();
        let mut snapshot = BTreeSet::new();
        snapshot.insert(a.id);
        snapshot.insert(b.id);
        store.save_view("everything", &snapshot).unwrap();
        assert_eq!(store.load_view("everything").unwrap(), snapshot);
        assert_eq!(store.list_views().unwrap(), vec!["everything".to_string()]);

        // Saving again with a smaller set replaces, not unions.
        let mut just_a = BTreeSet::new();
        just_a.insert(a.id);
        store.save_view("everything", &just_a).unwrap();
        assert_eq!(store.load_view("everything").unwrap(), just_a);
    }

    #[test]
    fn owners_round_trip() {
        let (store, _td) = test_store();
        store.add_owner("alice@example.com").unwrap();
        store.add_owner("bob@example.com").unwrap();
        let owners = store.list_owners().unwrap();
        assert!(owners.contains("alice@example.com"));
        assert!(owners.contains("bob@example.com"));
        store.remove_owner("alice@example.com").unwrap();
        let owners = store.list_owners().unwrap();
        assert!(!owners.contains("alice@example.com"));
        assert!(owners.contains("bob@example.com"));
    }

    #[test]
    fn schema_version_is_seeded_on_open() {
        let (store, _td) = test_store();
        assert_eq!(
            store.schema_version().unwrap().as_deref(),
            Some(keys::SCHEMA_VERSION),
        );
    }
}
