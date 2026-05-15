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
use crate::ticket::{Comment, CommentBody, NewTicketOpts, Ticket, TicketState, TicketStatus};
use crate::writeup::{NewWriteupOpts, Writeup, WriteupStatus, WriteupVersion};

/// Basic email format check: must contain exactly one `@` with non-empty
/// local and domain parts.
fn validate_email(email: &str) -> Result<()> {
    let email = email.trim();
    let parts: Vec<&str> = email.split('@').collect();
    if parts.len() == 2
        && !parts[0].is_empty()
        && parts[1].contains('.')
        && !parts[1].starts_with('.')
        && !parts[1].ends_with('.')
    {
        Ok(())
    } else {
        Err(Error::InvalidValue(format!(
            "`{email}` is not a valid email address (expected user@domain)"
        )))
    }
}

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
        let session = Session::open(repo.path())?;
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
        let now = opts.created_at.unwrap_or_else(OffsetDateTime::now_utc);
        let now_rfc = now
            .format(&Rfc3339)
            .map_err(|e| Error::Time(e.to_string()))?;

        // A ticket's existence is implied by its fields - no separate
        // index to maintain.
        p.set(&keys::ticket_field(&id, "title"), title)?;
        p.set(
            &keys::ticket_field(&id, "status"),
            TicketStatus::Open.as_str(),
        )?;
        p.set(&keys::ticket_field(&id, "state"), TicketState::New.as_str())?;
        p.set(&keys::ticket_field(&id, "created-at"), now_rfc.as_str())?;
        p.set(&keys::ticket_field(&id, "created-by"), self.session.email())?;

        if let Some(ref a) = opts.assigned {
            if !a.is_empty() {
                let resolved = self.resolve_user(a)?;
                validate_email(&resolved)?;
                p.set(&keys::ticket_field(&id, "assigned"), resolved.as_str())?;
            }
        }

        if let Some(parent_id) = opts.parent {
            // Validate parent exists
            self.load(&parent_id)?;
            p.set(
                &keys::ticket_field(&id, "parent"),
                parent_id.to_string().as_str(),
            )?;
            // Denormalize: add child to parent's children set
            p.set_add(&keys::ticket_field(&parent_id, "children"), &id.to_string())?;
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
        let matches: Vec<&Ticket> = tickets
            .iter()
            .filter(|t| {
                let hex = t.id.to_string().replace('-', "");
                hex.starts_with(&needle)
            })
            .collect();
        match matches.len() {
            0 => Err(Error::NoMatch(reference.to_string())),
            1 => Ok(matches[0].id),
            n => {
                let open_matches: Vec<&Ticket> = matches
                    .iter()
                    .copied()
                    .filter(|t| t.status == TicketStatus::Open)
                    .collect();
                if open_matches.len() == 1 {
                    Ok(open_matches[0].id)
                } else {
                    Err(Error::Ambiguous(reference.to_string(), n))
                }
            }
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

    pub fn set_spec(&self, id: &Uuid, spec: Option<&str>) -> Result<()> {
        let p = self.project_handle();
        let key = keys::ticket_field(id, "spec");
        match spec {
            Some(s) if !s.is_empty() => {
                p.set(&key, s)?;
            }
            _ => {
                p.remove(&key)?;
            }
        }
        Ok(())
    }

    pub fn set_state(&self, id: &Uuid, state: TicketState) -> Result<()> {
        self.set_lifecycle(id, state.status(), state)
    }

    pub fn set_lifecycle(&self, id: &Uuid, status: TicketStatus, state: TicketState) -> Result<()> {
        if state.status() != status {
            return Err(Error::InvalidState(format!(
                "{}:{}",
                status.as_str(),
                state.as_str()
            )));
        }
        let p = self.project_handle();
        p.set(&keys::ticket_field(id, "status"), status.as_str())?;
        p.set(&keys::ticket_field(id, "state"), state.as_str())?;
        if status == TicketStatus::Closed {
            p.set(&keys::ticket_field(id, "closed-by"), self.session.email())?;
        } else {
            p.remove(&keys::ticket_field(id, "closed-by"))?;
        }
        Ok(())
    }

    pub fn set_closed_by(&self, id: &Uuid, who: Option<&str>) -> Result<()> {
        let p = self.project_handle();
        let key = keys::ticket_field(id, "closed-by");
        match who {
            Some(w) if !w.is_empty() => {
                let resolved = self.resolve_user(w)?;
                validate_email(&resolved)?;
                p.set(&key, resolved.as_str())?;
            }
            _ => {
                p.remove(&key)?;
            }
        }
        Ok(())
    }

    pub fn set_assigned(&self, id: &Uuid, who: Option<&str>) -> Result<()> {
        let p = self.project_handle();
        let key = keys::ticket_field(id, "assigned");
        match who {
            Some(w) if !w.is_empty() => {
                let resolved = self.resolve_user(w)?;
                validate_email(&resolved)?;
                p.set(&key, resolved.as_str())?;
            }
            _ => {
                p.remove(&key)?;
            }
        }
        Ok(())
    }

    pub fn set_priority(&self, id: &Uuid, priority: Option<i64>) -> Result<()> {
        let p = self.project_handle();
        let key = keys::ticket_field(id, "priority");
        match priority {
            Some(n) => {
                p.set(&key, n.to_string().as_str())?;
            }
            None => {
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

    pub fn set_code(&self, id: &Uuid, code: Option<&str>) -> Result<()> {
        let p = self.project_handle();
        let key = keys::ticket_field(id, "code");
        match code {
            Some(c) if !c.is_empty() => {
                crate::ticket::validate_code_uri(c)?;
                p.set(&key, c)?;
            }
            _ => {
                p.remove(&key)?;
            }
        }
        Ok(())
    }

    /// Set the parent of a ticket. Validates that the parent exists and
    /// prevents self-reference and circular chains.
    pub fn set_parent(&self, child_id: &Uuid, parent_id: &Uuid) -> Result<()> {
        if child_id == parent_id {
            return Err(Error::InvalidValue(
                "a ticket cannot be its own parent".to_string(),
            ));
        }

        // Validate parent exists
        let parent = self.load(parent_id)?;

        // Prevent circular chains: walk ancestors up to depth 20
        let mut ancestor_id = parent.parent;
        let mut depth = 0;
        while let Some(aid) = ancestor_id {
            if aid == *child_id {
                return Err(Error::InvalidValue(
                    "circular parent chain detected".to_string(),
                ));
            }
            depth += 1;
            if depth > 20 {
                break;
            }
            ancestor_id = self.load(&aid).ok().and_then(|t| t.parent);
        }

        let p = self.project_handle();

        // Remove from old parent's children set if any
        let child = self.load(child_id)?;
        if let Some(old_parent) = child.parent {
            p.set_remove(
                &keys::ticket_field(&old_parent, "children"),
                &child_id.to_string(),
            )?;
        }

        // Set the parent field on the child
        p.set(
            &keys::ticket_field(child_id, "parent"),
            parent_id.to_string().as_str(),
        )?;

        // Add to new parent's children set
        p.set_add(
            &keys::ticket_field(parent_id, "children"),
            &child_id.to_string(),
        )?;

        Ok(())
    }

    /// Remove the parent of a ticket.
    pub fn clear_parent(&self, child_id: &Uuid) -> Result<()> {
        let child = self.load(child_id)?;
        let p = self.project_handle();

        if let Some(old_parent) = child.parent {
            p.set_remove(
                &keys::ticket_field(&old_parent, "children"),
                &child_id.to_string(),
            )?;
        }

        p.remove(&keys::ticket_field(child_id, "parent"))?;
        Ok(())
    }

    /// Add a dependency: `id` depends on `dependency_id`.
    /// The dependency ticket must exist. Circular dependencies are rejected.
    pub fn add_dependency(&self, id: &Uuid, dependency_id: &Uuid) -> Result<()> {
        if id == dependency_id {
            return Err(Error::InvalidValue(
                "a ticket cannot depend on itself".to_string(),
            ));
        }
        // Validate dependency exists
        self.load(dependency_id)?;

        // Check for circular deps: walk the dependency chain from dependency_id
        let mut visited = std::collections::HashSet::new();
        visited.insert(*id);
        let mut stack = vec![*dependency_id];
        while let Some(current) = stack.pop() {
            if !visited.insert(current) {
                continue;
            }
            let t = self.load(&current)?;
            for dep in &t.depends_on {
                if dep == id {
                    return Err(Error::InvalidValue(
                        "circular dependency detected".to_string(),
                    ));
                }
                stack.push(*dep);
            }
        }

        let p = self.project_handle();
        // id depends_on dependency_id
        p.set_add(
            &keys::ticket_field(id, "depends_on"),
            &dependency_id.to_string(),
        )?;
        // dependency_id blocks id (denormalized reverse)
        p.set_add(
            &keys::ticket_field(dependency_id, "blocks"),
            &id.to_string(),
        )?;
        Ok(())
    }

    /// Remove a dependency: `id` no longer depends on `dependency_id`.
    pub fn remove_dependency(&self, id: &Uuid, dependency_id: &Uuid) -> Result<()> {
        let p = self.project_handle();
        p.set_remove(
            &keys::ticket_field(id, "depends_on"),
            &dependency_id.to_string(),
        )?;
        p.set_remove(
            &keys::ticket_field(dependency_id, "blocks"),
            &id.to_string(),
        )?;
        Ok(())
    }

    pub fn set_meta(&self, id: &Uuid, field: &str, value: &str) -> Result<()> {
        let field = field.trim();
        if field.is_empty() {
            return Err(Error::InvalidValue(
                "metadata field cannot be empty".to_string(),
            ));
        }
        if field.contains(':') {
            return Err(Error::InvalidValue(
                "metadata field cannot contain `:`".to_string(),
            ));
        }

        self.project_handle()
            .set(&keys::ticket_meta_field(id, field), value)?;
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

    pub fn add_writeup_tag(&self, id: &Uuid, tag: &str) -> Result<()> {
        self.load_writeup(id)?;
        let tag = tag.trim();
        if tag.is_empty() {
            return Ok(());
        }
        self.project_handle()
            .set_add(&keys::writeup_field(id, "tags"), tag)?;
        Ok(())
    }

    pub fn remove_writeup_tag(&self, id: &Uuid, tag: &str) -> Result<()> {
        self.load_writeup(id)?;
        let tag = tag.trim();
        if tag.is_empty() {
            return Ok(());
        }
        self.project_handle()
            .set_remove(&keys::writeup_field(id, "tags"), tag)?;
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
        let email = self.session.email().to_string();
        validate_email(&email)?;
        let payload = CommentBody {
            author: email,
            body: body.to_string(),
        };
        let json = serde_json::to_string(&payload)?;
        handle.list_push(&keys::ticket_field(id, "comments"), &json)?;
        Ok(())
    }

    // -------------------------------------------------------------------
    // Writeups
    // -------------------------------------------------------------------

    pub fn create_writeup(&self, title: &str, opts: NewWriteupOpts) -> Result<Writeup> {
        let title = title.trim();
        if title.is_empty() {
            return Err(Error::InvalidValue(
                "writeup title cannot be empty".to_string(),
            ));
        }

        let id = Uuid::new_v4();
        let p = self.project_handle();
        let now = opts.created_at.unwrap_or_else(OffsetDateTime::now_utc);
        let now_rfc = now
            .format(&Rfc3339)
            .map_err(|e| Error::Time(e.to_string()))?;
        let author = self.session.email();
        validate_email(author)?;

        p.set(&keys::writeup_field(&id, "title"), title)?;
        p.set(
            &keys::writeup_field(&id, "status"),
            WriteupStatus::Open.as_str(),
        )?;
        p.set(&keys::writeup_field(&id, "created-at"), now_rfc.as_str())?;
        p.set(&keys::writeup_field(&id, "created-by"), author)?;
        p.set_add(&keys::writeup_field(&id, "authors"), author)?;

        for tag in opts.tags {
            let tag = tag.trim();
            if !tag.is_empty() {
                p.set_add(&keys::writeup_field(&id, "tags"), tag)?;
            }
        }

        if let Some(body) = opts.body {
            self.push_writeup_version(&p, &id, &body)?;
        }

        self.load_writeup(&id)
    }

    pub fn list_writeups(&self) -> Result<Vec<Writeup>> {
        let p = self.project_handle();
        let pairs = p.get_all_values(Some(&keys::writeups_prefix()))?;
        let mut by_id: BTreeMap<Uuid, Vec<(String, MetaValue)>> = BTreeMap::new();
        for (key, value) in pairs {
            if let Some((id, field)) = keys::parse_writeup_field(&key) {
                by_id
                    .entry(id)
                    .or_default()
                    .push((field.to_string(), value));
            }
        }

        let mut out = Vec::with_capacity(by_id.len());
        for (id, fields) in by_id {
            if let Some(writeup) = build_writeup(id, fields) {
                out.push(writeup);
            }
        }
        out.sort_by(|a, b| {
            metadata_priority_sort_key(a.priority)
                .cmp(&metadata_priority_sort_key(b.priority))
                .then_with(|| {
                    b.created_at
                        .cmp(&a.created_at)
                        .then_with(|| a.title.cmp(&b.title))
                })
        });
        Ok(out)
    }

    pub fn load_writeup(&self, id: &Uuid) -> Result<Writeup> {
        let p = self.project_handle();
        let pairs = p.get_all_values(Some(&keys::writeup_prefix(id)))?;
        let mut fields = Vec::with_capacity(pairs.len());
        for (key, value) in pairs {
            if let Some((parsed_id, field)) = keys::parse_writeup_field(&key) {
                if parsed_id == *id {
                    fields.push((field.to_string(), value));
                }
            }
        }
        build_writeup(*id, fields).ok_or(Error::NotFound(*id))
    }

    pub fn resolve_writeup_id(&self, reference: &str) -> Result<Uuid> {
        let needle = reference.trim().to_ascii_lowercase().replace('-', "");
        if needle.is_empty() {
            return Err(Error::NoMatch(reference.to_string()));
        }
        let writeups = self.list_writeups()?;
        let matches: Vec<&Writeup> = writeups
            .iter()
            .filter(|writeup| {
                let hex = writeup.id.to_string().replace('-', "");
                hex.starts_with(&needle)
            })
            .collect();
        match matches.len() {
            0 => Err(Error::NoMatch(reference.to_string())),
            1 => Ok(matches[0].id),
            n => {
                let open_matches: Vec<&Writeup> = matches
                    .iter()
                    .copied()
                    .filter(|writeup| writeup.status == WriteupStatus::Open)
                    .collect();
                if open_matches.len() == 1 {
                    Ok(open_matches[0].id)
                } else {
                    Err(Error::Ambiguous(reference.to_string(), n))
                }
            }
        }
    }

    pub fn append_writeup_version(&self, id: &Uuid, body: &str) -> Result<()> {
        self.load_writeup(id)?;
        let p = self.project_handle();
        self.push_writeup_version(&p, id, body)?;
        Ok(())
    }

    pub fn set_writeup_title(&self, id: &Uuid, title: &str) -> Result<()> {
        self.load_writeup(id)?;
        let title = title.trim();
        if title.is_empty() {
            return Err(Error::InvalidValue(
                "writeup title cannot be empty".to_string(),
            ));
        }
        self.project_handle()
            .set(&keys::writeup_field(id, "title"), title)?;
        Ok(())
    }

    fn push_writeup_version(
        &self,
        handle: &git_meta_lib::SessionTargetHandle<'_>,
        id: &Uuid,
        body: &str,
    ) -> Result<()> {
        let body = body.trim();
        if body.is_empty() {
            return Err(Error::InvalidValue(
                "writeup version body cannot be empty".to_string(),
            ));
        }
        let author = self.session.email();
        validate_email(author)?;
        let now = OffsetDateTime::now_utc();
        let now_rfc = now
            .format(&Rfc3339)
            .map_err(|e| Error::Time(e.to_string()))?;
        let doc = format!("---\nauthor: {author}\ndate: {now_rfc}\n---\n\n{body}");
        handle.list_push(&keys::writeup_field(id, "versions"), &doc)?;
        handle.set_add(&keys::writeup_field(id, "authors"), author)?;
        Ok(())
    }

    pub fn set_writeup_status(&self, id: &Uuid, status: WriteupStatus) -> Result<()> {
        self.load_writeup(id)?;
        self.project_handle()
            .set(&keys::writeup_field(id, "status"), status.as_str())?;
        Ok(())
    }

    pub fn set_writeup_priority(&self, id: &Uuid, priority: Option<i64>) -> Result<()> {
        self.load_writeup(id)?;
        let p = self.project_handle();
        let key = keys::writeup_field(id, "priority");
        match priority {
            Some(n) => {
                p.set(&key, n.to_string().as_str())?;
            }
            None => {
                p.remove(&key)?;
            }
        }
        Ok(())
    }

    pub fn link_writeup_ticket(&self, writeup_id: &Uuid, ticket_id: &Uuid) -> Result<()> {
        self.load_writeup(writeup_id)?;
        self.load(ticket_id)?;
        let p = self.project_handle();
        p.set_add(
            &keys::writeup_field(writeup_id, "tickets"),
            &ticket_id.to_string(),
        )?;
        p.set_add(
            &keys::ticket_field(ticket_id, "writeups"),
            &writeup_id.to_string(),
        )?;
        Ok(())
    }

    pub fn unlink_writeup_ticket(&self, writeup_id: &Uuid, ticket_id: &Uuid) -> Result<()> {
        self.load_writeup(writeup_id)?;
        self.load(ticket_id)?;
        let p = self.project_handle();
        p.set_remove(
            &keys::writeup_field(writeup_id, "tickets"),
            &ticket_id.to_string(),
        )?;
        p.set_remove(
            &keys::ticket_field(ticket_id, "writeups"),
            &writeup_id.to_string(),
        )?;
        Ok(())
    }

    pub fn promote_writeup(&self, writeup_id: &Uuid) -> Result<Ticket> {
        let writeup = self.load_writeup(writeup_id)?;
        let body = writeup.latest_body().unwrap_or("").trim().to_string();
        let ticket = self.create(
            &writeup.title,
            NewTicketOpts {
                tags: writeup.tags.iter().cloned().collect(),
                ..Default::default()
            },
        )?;
        if !body.is_empty() {
            self.set_description(&ticket.id, Some(&body))?;
        }
        if writeup.priority.is_some() {
            self.set_priority(&ticket.id, writeup.priority)?;
        }
        self.link_writeup_ticket(writeup_id, &ticket.id)?;
        self.load(&ticket.id)
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
        validate_email(who)?;
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

    // -------------------------------------------------------------------
    // User nick → email map (shared mailmap)
    // -------------------------------------------------------------------

    /// List all user nicks and their email sets.
    pub fn list_users(&self) -> Result<BTreeMap<String, BTreeSet<String>>> {
        let p = self.project_handle();
        let pairs = p.get_all_values(Some(&keys::users_prefix()))?;
        let mut users = BTreeMap::new();
        for (key, value) in pairs {
            if let Some(nick) = keys::parse_user_nick(&key) {
                if let MetaValue::Set(emails) = value {
                    users.insert(nick.to_string(), emails);
                }
            }
        }
        Ok(users)
    }

    /// Get the email set for a nick.
    pub fn get_user(&self, nick: &str) -> Result<BTreeSet<String>> {
        let p = self.project_handle();
        match p.get_value(&keys::user_key(nick))? {
            Some(MetaValue::Set(emails)) => Ok(emails),
            _ => Ok(BTreeSet::new()),
        }
    }

    /// Add an email to a nick's set.
    pub fn add_user_email(&self, nick: &str, email: &str) -> Result<()> {
        validate_email(email)?;
        self.project_handle()
            .set_add(&keys::user_key(nick), email)?;
        Ok(())
    }

    /// Remove an email from a nick's set. If the set becomes empty, remove the key.
    pub fn remove_user_email(&self, nick: &str, email: &str) -> Result<()> {
        let p = self.project_handle();
        p.set_remove(&keys::user_key(nick), email)?;
        // Check if empty and clean up.
        if let Ok(emails) = self.get_user(nick) {
            if emails.is_empty() {
                p.remove(&keys::user_key(nick))?;
            }
        }
        Ok(())
    }

    /// Remove a user nick entirely (all emails).
    pub fn remove_user(&self, nick: &str) -> Result<()> {
        self.project_handle().remove(&keys::user_key(nick))?;
        Ok(())
    }

    /// Resolve a nick or email to an email address.
    /// If `input` contains `@`, treat it as an email and return as-is.
    /// Otherwise, look up as a nick and return the first email.
    pub fn resolve_user(&self, input: &str) -> Result<String> {
        if input.contains('@') {
            return Ok(input.to_string());
        }
        let emails = self.get_user(input)?;
        emails
            .into_iter()
            .next()
            .ok_or_else(|| Error::InvalidValue(format!("unknown user nick `{input}`")))
    }

    /// Reverse-lookup: given an email, find the nick (if any).
    pub fn nick_for_email(&self, email: &str) -> Result<Option<String>> {
        let users = self.list_users()?;
        for (nick, emails) in &users {
            if emails.contains(email) {
                return Ok(Some(nick.clone()));
            }
        }
        Ok(None)
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
    let mut spec: Option<String> = None;
    let mut status: Option<TicketStatus> = None;
    let mut state: Option<TicketState> = None;
    let mut legacy_status: Option<TicketStatus> = None;
    let mut legacy_state: Option<TicketState> = None;
    let mut assigned: Option<String> = None;
    let mut closed_by: Option<String> = None;
    let mut priority: Option<i64> = None;
    let mut points: Option<i64> = None;
    let mut milestone: Option<String> = None;
    let mut code: Option<String> = None;
    let mut parent: Option<Uuid> = None;
    let mut children: BTreeSet<Uuid> = BTreeSet::new();
    let mut depends_on: BTreeSet<Uuid> = BTreeSet::new();
    let mut blocks: BTreeSet<Uuid> = BTreeSet::new();
    let mut tags: BTreeSet<String> = BTreeSet::new();
    let mut meta: BTreeMap<String, String> = BTreeMap::new();
    let mut comments: Vec<Comment> = Vec::new();
    let mut created_at: Option<OffsetDateTime> = None;
    let mut created_by = String::new();

    for (field, value) in fields {
        match (field.as_str(), value) {
            ("title", MetaValue::String(s)) => title = Some(s),
            ("description", MetaValue::String(s)) => description = Some(s),
            ("spec", MetaValue::String(s)) => spec = Some(s),
            ("status", MetaValue::String(s)) => {
                status = TicketStatus::parse(&s).ok();
            }
            ("state", MetaValue::String(s)) => match s.as_str() {
                "open" => {
                    legacy_status = Some(TicketStatus::Open);
                    legacy_state = Some(TicketState::New);
                }
                "hold" => {
                    legacy_status = Some(TicketStatus::Open);
                    legacy_state = Some(TicketState::Blocked);
                }
                "resolved" => {
                    legacy_status = Some(TicketStatus::Closed);
                    legacy_state = Some(TicketState::Resolved);
                }
                "invalid" => {
                    legacy_status = Some(TicketStatus::Closed);
                    legacy_state = Some(TicketState::Invalid);
                }
                _ => {
                    state = TicketState::parse(&s).ok();
                }
            },
            ("assigned", MetaValue::String(s)) => assigned = Some(s),
            ("closed-by", MetaValue::String(s)) => closed_by = Some(s),
            ("priority", MetaValue::String(s)) => priority = s.parse().ok(),
            ("points", MetaValue::String(s)) => points = s.parse().ok(),
            ("milestone", MetaValue::String(s)) => milestone = Some(s),
            ("code", MetaValue::String(s)) => code = Some(s),
            ("parent", MetaValue::String(s)) => parent = Uuid::parse_str(&s).ok(),
            ("children", MetaValue::Set(members)) => {
                children = members
                    .iter()
                    .filter_map(|s| Uuid::parse_str(s).ok())
                    .collect();
            }
            ("depends_on", MetaValue::Set(members)) => {
                depends_on = members
                    .iter()
                    .filter_map(|s| Uuid::parse_str(s).ok())
                    .collect();
            }
            ("blocks", MetaValue::Set(members)) => {
                blocks = members
                    .iter()
                    .filter_map(|s| Uuid::parse_str(s).ok())
                    .collect();
            }
            ("tags", MetaValue::Set(members)) => tags = members,
            ("comments", MetaValue::List(entries)) => comments = decode_comments(entries),
            (field, MetaValue::String(s)) if field.starts_with("meta:") => {
                let key = field.trim_start_matches("meta:");
                if !key.is_empty() {
                    meta.insert(key.to_string(), s);
                }
            }
            ("created-at", MetaValue::String(s)) => {
                created_at = OffsetDateTime::parse(&s, &Rfc3339).ok();
            }
            ("created-by", MetaValue::String(s)) => created_by = s,
            _ => {}
        }
    }

    let title = title?;
    let created_at = created_at.unwrap_or(OffsetDateTime::UNIX_EPOCH);
    let state = state.or(legacy_state).unwrap_or(TicketState::New);
    let status = status
        .or(legacy_status)
        .filter(|status| *status == state.status())
        .unwrap_or_else(|| state.status());

    Some(Ticket {
        id,
        title,
        description,
        spec,
        status,
        state,
        assigned,
        closed_by,
        priority,
        points,
        milestone,
        code,
        parent,
        children,
        depends_on,
        blocks,
        tags,
        meta,
        comments,
        created_at,
        created_by,
    })
}

fn build_writeup(id: Uuid, fields: Vec<(String, MetaValue)>) -> Option<Writeup> {
    if fields.is_empty() {
        return None;
    }

    let mut title: Option<String> = None;
    let mut status = WriteupStatus::Open;
    let mut priority: Option<i64> = None;
    let mut created_at: Option<OffsetDateTime> = None;
    let mut created_by = String::new();
    let mut authors = BTreeSet::new();
    let mut tags = BTreeSet::new();
    let mut tickets = BTreeSet::new();
    let mut versions = Vec::new();

    for (field, value) in fields {
        match (field.as_str(), value) {
            ("title", MetaValue::String(s)) => title = Some(s),
            ("status", MetaValue::String(s)) => {
                status = WriteupStatus::parse(&s).unwrap_or(WriteupStatus::Open);
            }
            ("priority", MetaValue::String(s)) => priority = s.parse().ok(),
            ("created-at", MetaValue::String(s)) => {
                created_at = OffsetDateTime::parse(&s, &Rfc3339).ok();
            }
            ("created-by", MetaValue::String(s)) => created_by = s,
            ("authors", MetaValue::Set(members)) => authors = members,
            ("tags", MetaValue::Set(members)) => tags = members,
            ("tickets", MetaValue::Set(members)) => {
                tickets = members
                    .iter()
                    .filter_map(|s| Uuid::parse_str(s).ok())
                    .collect();
            }
            ("versions", MetaValue::List(entries)) => versions = decode_writeup_versions(entries),
            _ => {}
        }
    }

    let title = title?;
    let created_at = created_at.unwrap_or(OffsetDateTime::UNIX_EPOCH);
    if created_by.is_empty() {
        created_by = authors.iter().next().cloned().unwrap_or_default();
    }

    Some(Writeup {
        id,
        title,
        status,
        priority,
        created_at,
        created_by,
        authors,
        tags,
        tickets,
        versions,
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

fn decode_writeup_versions(entries: Vec<ListEntry>) -> Vec<WriteupVersion> {
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        let fallback_at =
            OffsetDateTime::from_unix_timestamp_nanos(i128::from(entry.timestamp) * 1_000_000)
                .unwrap_or(OffsetDateTime::UNIX_EPOCH);
        out.push(decode_writeup_version(&entry.value, fallback_at));
    }
    out
}

fn decode_writeup_version(raw: &str, fallback_at: OffsetDateTime) -> WriteupVersion {
    let Some(rest) = raw.strip_prefix("---\n") else {
        return WriteupVersion {
            author: "unknown".to_string(),
            at: fallback_at,
            body: raw.to_string(),
        };
    };
    let Some((frontmatter, body)) = rest.split_once("\n---\n") else {
        return WriteupVersion {
            author: "unknown".to_string(),
            at: fallback_at,
            body: raw.to_string(),
        };
    };

    let mut author = "unknown".to_string();
    let mut at = fallback_at;
    for line in frontmatter.lines() {
        if let Some(value) = line.strip_prefix("author:") {
            let value = value.trim();
            if !value.is_empty() {
                author = value.to_string();
            }
        } else if let Some(value) = line.strip_prefix("date:") {
            if let Ok(parsed) = OffsetDateTime::parse(value.trim(), &Rfc3339) {
                at = parsed;
            }
        }
    }

    WriteupVersion {
        author,
        at,
        body: body.trim_start_matches(['\r', '\n']).to_string(),
    }
}

fn metadata_priority_sort_key(priority: Option<i64>) -> (u8, i64) {
    match priority {
        Some(value) => (0, value),
        None => (1, 0),
    }
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
            ..Default::default()
        };
        let created = store.create("My new ticket", opts).unwrap();
        assert_eq!(created.title, "My new ticket");
        assert_eq!(created.status, TicketStatus::Open);
        assert_eq!(created.state, TicketState::New);
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
        let loaded = store.load(&t.id).unwrap();
        assert_eq!(loaded.status, TicketStatus::Closed);
        assert_eq!(loaded.state, TicketState::Resolved);
        assert_eq!(loaded.closed_by.as_deref(), Some(store.email()));
        store.set_state(&t.id, TicketState::New).unwrap();
        assert_eq!(store.load(&t.id).unwrap().closed_by, None);
    }

    #[test]
    fn lifecycle_change_persists_status_and_state() {
        let (store, _td) = test_store();
        let t = store.create("x", NewTicketOpts::default()).unwrap();
        store
            .set_lifecycle(&t.id, TicketStatus::Open, TicketState::Blocked)
            .unwrap();
        let loaded = store.load(&t.id).unwrap();
        assert_eq!(loaded.status, TicketStatus::Open);
        assert_eq!(loaded.state, TicketState::Blocked);
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
    fn resolve_id_accepts_prefix_unique_among_open_tickets() {
        let (store, _td) = test_store();
        let open = Uuid::parse_str("d7f2d8f6-d6ec-3da1-a180-0a33fb090d59").unwrap();
        let closed = Uuid::parse_str("d7f99999-d6ec-3da1-a180-0a33fb090d59").unwrap();
        insert_ticket(&store, open, "open", TicketStatus::Open, TicketState::New);
        insert_ticket(
            &store,
            closed,
            "closed",
            TicketStatus::Closed,
            TicketState::Resolved,
        );

        assert_eq!(store.resolve_id("d7f").unwrap(), open);
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
    fn writeups_round_trip_versions_and_status() {
        let (store, _td) = test_store();
        let writeup = store
            .create_writeup(
                "Design note",
                NewWriteupOpts {
                    body: Some("first draft".to_string()),
                    tags: vec!["design".to_string()],
                    ..Default::default()
                },
            )
            .unwrap();
        store
            .append_writeup_version(&writeup.id, "second draft")
            .unwrap();
        store
            .set_writeup_status(&writeup.id, WriteupStatus::Closed)
            .unwrap();
        store.set_writeup_priority(&writeup.id, Some(2)).unwrap();
        store.add_writeup_tag(&writeup.id, "review").unwrap();
        store.remove_writeup_tag(&writeup.id, "design").unwrap();

        let loaded = store.load_writeup(&writeup.id).unwrap();
        assert_eq!(loaded.title, "Design note");
        assert_eq!(loaded.status, WriteupStatus::Closed);
        assert_eq!(loaded.priority, Some(2));
        assert!(!loaded.tags.contains("design"));
        assert!(loaded.tags.contains("review"));
        assert!(loaded.authors.contains(store.email()));
        assert_eq!(loaded.versions.len(), 2);
        assert_eq!(loaded.versions[0].author, store.email());
        assert_eq!(loaded.versions[0].body, "first draft");
        assert_eq!(loaded.versions[1].body, "second draft");
        assert_eq!(
            store.resolve_writeup_id(&writeup.short_id()).unwrap(),
            writeup.id
        );
    }

    #[test]
    fn writeups_sort_by_priority_before_recency() {
        let (store, _td) = test_store();
        let old_high = store
            .create_writeup(
                "old high",
                NewWriteupOpts {
                    created_at: Some(
                        OffsetDateTime::from_unix_timestamp(1_000).expect("valid timestamp"),
                    ),
                    ..Default::default()
                },
            )
            .unwrap();
        let recent_low = store
            .create_writeup(
                "recent low",
                NewWriteupOpts {
                    created_at: Some(
                        OffsetDateTime::from_unix_timestamp(2_000).expect("valid timestamp"),
                    ),
                    ..Default::default()
                },
            )
            .unwrap();
        let no_priority = store
            .create_writeup(
                "no priority",
                NewWriteupOpts {
                    created_at: Some(
                        OffsetDateTime::from_unix_timestamp(3_000).expect("valid timestamp"),
                    ),
                    ..Default::default()
                },
            )
            .unwrap();
        store.set_writeup_priority(&old_high.id, Some(1)).unwrap();
        store.set_writeup_priority(&recent_low.id, Some(5)).unwrap();

        let writeups = store.list_writeups().unwrap();

        assert_eq!(
            writeups
                .iter()
                .map(|writeup| writeup.id)
                .collect::<Vec<_>>(),
            vec![old_high.id, recent_low.id, no_priority.id]
        );
    }

    #[test]
    fn writeups_link_unlink_and_promote() {
        let (store, _td) = test_store();
        let ticket = store.create("existing", NewTicketOpts::default()).unwrap();
        let writeup = store
            .create_writeup(
                "Promotable",
                NewWriteupOpts {
                    body: Some("make this actionable".to_string()),
                    tags: vec!["feature".to_string()],
                    ..Default::default()
                },
            )
            .unwrap();
        store.set_writeup_priority(&writeup.id, Some(3)).unwrap();

        store.link_writeup_ticket(&writeup.id, &ticket.id).unwrap();
        assert!(store
            .load_writeup(&writeup.id)
            .unwrap()
            .tickets
            .contains(&ticket.id));
        store
            .unlink_writeup_ticket(&writeup.id, &ticket.id)
            .unwrap();
        assert!(!store
            .load_writeup(&writeup.id)
            .unwrap()
            .tickets
            .contains(&ticket.id));

        let promoted = store.promote_writeup(&writeup.id).unwrap();
        assert_eq!(promoted.title, "Promotable");
        assert_eq!(
            promoted.description.as_deref(),
            Some("make this actionable")
        );
        assert_eq!(promoted.priority, Some(3));
        assert!(promoted.tags.contains("feature"));
        assert!(store
            .load_writeup(&writeup.id)
            .unwrap()
            .tickets
            .contains(&promoted.id));
    }

    fn insert_ticket(
        store: &TicketStore,
        id: Uuid,
        title: &str,
        status: TicketStatus,
        state: TicketState,
    ) {
        let p = store.project_handle();
        let created = OffsetDateTime::UNIX_EPOCH.format(&Rfc3339).unwrap();
        p.set(&keys::ticket_field(&id, "title"), title).unwrap();
        p.set(&keys::ticket_field(&id, "status"), status.as_str())
            .unwrap();
        p.set(&keys::ticket_field(&id, "state"), state.as_str())
            .unwrap();
        p.set(&keys::ticket_field(&id, "created-at"), created.as_str())
            .unwrap();
        p.set(&keys::ticket_field(&id, "created-by"), store.email())
            .unwrap();
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

    #[test]
    fn parent_child_round_trips() {
        let (store, _td) = test_store();
        let parent = store.create("epic", NewTicketOpts::default()).unwrap();
        let child = store
            .create(
                "sub-task",
                NewTicketOpts {
                    parent: Some(parent.id),
                    ..Default::default()
                },
            )
            .unwrap();

        assert_eq!(child.parent, Some(parent.id));
        let parent = store.load(&parent.id).unwrap();
        assert!(parent.children.contains(&child.id));
    }

    #[test]
    fn set_parent_and_clear_parent() {
        let (store, _td) = test_store();
        let epic = store.create("epic", NewTicketOpts::default()).unwrap();
        let task = store.create("task", NewTicketOpts::default()).unwrap();

        // Set parent
        store.set_parent(&task.id, &epic.id).unwrap();
        let task = store.load(&task.id).unwrap();
        assert_eq!(task.parent, Some(epic.id));
        let epic = store.load(&epic.id).unwrap();
        assert!(epic.children.contains(&task.id));

        // Clear parent
        store.clear_parent(&task.id).unwrap();
        let task = store.load(&task.id).unwrap();
        assert_eq!(task.parent, None);
        let epic = store.load(&epic.id).unwrap();
        assert!(!epic.children.contains(&task.id));
    }

    #[test]
    fn set_parent_rejects_self_reference() {
        let (store, _td) = test_store();
        let t = store.create("x", NewTicketOpts::default()).unwrap();
        assert!(store.set_parent(&t.id, &t.id).is_err());
    }

    #[test]
    fn set_parent_rejects_circular_chain() {
        let (store, _td) = test_store();
        let a = store.create("a", NewTicketOpts::default()).unwrap();
        let b = store.create("b", NewTicketOpts::default()).unwrap();
        let c = store.create("c", NewTicketOpts::default()).unwrap();

        store.set_parent(&b.id, &a.id).unwrap();
        store.set_parent(&c.id, &b.id).unwrap();
        // a -> b -> c, now trying c -> a would be circular
        assert!(store.set_parent(&a.id, &c.id).is_err());
    }

    #[test]
    fn reparent_moves_child_between_parents() {
        let (store, _td) = test_store();
        let p1 = store.create("parent1", NewTicketOpts::default()).unwrap();
        let p2 = store.create("parent2", NewTicketOpts::default()).unwrap();
        let child = store
            .create(
                "child",
                NewTicketOpts {
                    parent: Some(p1.id),
                    ..Default::default()
                },
            )
            .unwrap();

        // Move child from p1 to p2
        store.set_parent(&child.id, &p2.id).unwrap();
        let p1 = store.load(&p1.id).unwrap();
        let p2 = store.load(&p2.id).unwrap();
        assert!(!p1.children.contains(&child.id));
        assert!(p2.children.contains(&child.id));
    }
}
