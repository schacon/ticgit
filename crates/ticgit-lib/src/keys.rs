//! Centralised key-string layout for ticgit metadata.
//!
//! This module is the single source of truth for the on-the-wire key
//! shape. Every read or write in [`crate::store`] formats keys through
//! these helpers so the layout can be evolved in one place.
//!
//! Layout:
//!
//! ```text
//! target = project
//! keys:
//!   ticgit:<system-key>             # ticketing-system metadata
//!   ticgit:tickets:<uuid>:<field>   # per-ticket fields
//!   ticgit:writeups:<uuid>:<field>  # per-writeup fields
//!   ticgit:views:<name>             # saved selections (set of UUIDs)
//! ```

use uuid::Uuid;

/// Top-level namespace for every ticgit-managed key.
pub const NS: &str = "ticgit";

/// Schema-version system key. The current value is `"1"`; bumped only
/// when the layout changes in a way that requires migration.
pub const SCHEMA_VERSION_KEY: &str = "ticgit:schema-version";

/// Current schema version written by this implementation.
pub const SCHEMA_VERSION: &str = "1";

/// Prefix for the per-ticket field keyspace; pass to
/// `SessionTargetHandle::get_all_values` for project-wide ticket scans.
#[must_use]
pub fn tickets_prefix() -> String {
    format!("{NS}:tickets")
}

/// All keys for a single ticket share this prefix; used for one-ticket
/// scans.
#[must_use]
pub fn ticket_prefix(id: &Uuid) -> String {
    format!("{NS}:tickets:{id}")
}

/// A specific field on a specific ticket, e.g. `ticgit:tickets:<uuid>:title`.
#[must_use]
pub fn ticket_field(id: &Uuid, field: &str) -> String {
    format!("{NS}:tickets:{id}:{field}")
}

/// Prefix for the per-writeup field keyspace; pass to
/// `SessionTargetHandle::get_all_values` for project-wide writeup scans.
#[must_use]
pub fn writeups_prefix() -> String {
    format!("{NS}:writeups")
}

/// All keys for a single writeup share this prefix; used for one-writeup
/// scans.
#[must_use]
pub fn writeup_prefix(id: &Uuid) -> String {
    format!("{NS}:writeups:{id}")
}

/// A specific field on a specific writeup, e.g. `ticgit:writeups:<uuid>:title`.
#[must_use]
pub fn writeup_field(id: &Uuid, field: &str) -> String {
    format!("{NS}:writeups:{id}:{field}")
}

/// A specific metadata field on a ticket, e.g. `ticgit:tickets:<uuid>:meta:branch`.
#[must_use]
pub fn ticket_meta_field(id: &Uuid, field: &str) -> String {
    ticket_field(id, &format!("meta:{field}"))
}

/// A bare project-level system key, e.g. `ticgit:owners`.
#[must_use]
pub fn system_key(name: &str) -> String {
    format!("{NS}:{name}")
}

/// Prefix for saved-view keys; used to enumerate all views.
#[must_use]
pub fn views_prefix() -> String {
    format!("{NS}:views")
}

/// A single saved view, e.g. `ticgit:views:not_mine`.
#[must_use]
pub fn view(name: &str) -> String {
    format!("{NS}:views:{name}")
}

/// Prefix for user-map keys; used to enumerate all users.
#[must_use]
pub fn users_prefix() -> String {
    format!("{NS}:users")
}

/// A single user key, e.g. `ticgit:users:scott`.
#[must_use]
pub fn user_key(nick: &str) -> String {
    format!("{NS}:users:{nick}")
}

/// If `key` is a user key, returns the nick.
#[must_use]
pub fn parse_user_nick(key: &str) -> Option<&str> {
    let prefix = format!("{NS}:users:");
    let name = key.strip_prefix(&prefix)?;
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// If `key` is a per-ticket field key, returns `(ticket_uuid, field_name)`.
/// Returns `None` for system keys, view keys, or anything malformed.
#[must_use]
pub fn parse_ticket_field(key: &str) -> Option<(Uuid, &str)> {
    let prefix = format!("{NS}:tickets:");
    let rest = key.strip_prefix(&prefix)?;
    let (uuid_part, field) = rest.split_once(':')?;
    let uuid = Uuid::parse_str(uuid_part).ok()?;
    if field.is_empty() {
        return None;
    }
    Some((uuid, field))
}

/// If `key` is a per-writeup field key, returns `(writeup_uuid, field_name)`.
/// Returns `None` for system keys, ticket keys, or anything malformed.
#[must_use]
pub fn parse_writeup_field(key: &str) -> Option<(Uuid, &str)> {
    let prefix = format!("{NS}:writeups:");
    let rest = key.strip_prefix(&prefix)?;
    let (uuid_part, field) = rest.split_once(':')?;
    let uuid = Uuid::parse_str(uuid_part).ok()?;
    if field.is_empty() {
        return None;
    }
    Some((uuid, field))
}

/// If `key` is a saved-view key, returns the view name.
#[must_use]
pub fn parse_view_name(key: &str) -> Option<&str> {
    let prefix = format!("{NS}:views:");
    let name = key.strip_prefix(&prefix)?;
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_uuid() -> Uuid {
        Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap()
    }

    #[test]
    fn key_layout_is_stable() {
        let id = fixed_uuid();
        assert_eq!(tickets_prefix(), "ticgit:tickets");
        assert_eq!(
            ticket_prefix(&id),
            "ticgit:tickets:00000000-0000-0000-0000-000000000001"
        );
        assert_eq!(
            ticket_field(&id, "title"),
            "ticgit:tickets:00000000-0000-0000-0000-000000000001:title"
        );
        assert_eq!(
            ticket_field(&id, "state"),
            "ticgit:tickets:00000000-0000-0000-0000-000000000001:state"
        );
        assert_eq!(writeups_prefix(), "ticgit:writeups");
        assert_eq!(
            writeup_prefix(&id),
            "ticgit:writeups:00000000-0000-0000-0000-000000000001"
        );
        assert_eq!(
            writeup_field(&id, "title"),
            "ticgit:writeups:00000000-0000-0000-0000-000000000001:title"
        );
        assert_eq!(
            ticket_meta_field(&id, "branch"),
            "ticgit:tickets:00000000-0000-0000-0000-000000000001:meta:branch"
        );
        assert_eq!(system_key("owners"), "ticgit:owners");
        assert_eq!(system_key("schema-version"), "ticgit:schema-version");
        assert_eq!(view("not_mine"), "ticgit:views:not_mine");
        assert_eq!(views_prefix(), "ticgit:views");
    }

    #[test]
    fn schema_version_key_constant_matches_helper() {
        assert_eq!(SCHEMA_VERSION_KEY, system_key("schema-version"));
    }

    #[test]
    fn parse_ticket_field_round_trips_known_uuids() {
        let id = fixed_uuid();
        let key = ticket_field(&id, "title");
        let (got_id, field) = parse_ticket_field(&key).expect("should parse");
        assert_eq!(got_id, id);
        assert_eq!(field, "title");
    }

    #[test]
    fn parse_ticket_field_handles_random_uuids() {
        let id = Uuid::new_v4();
        let key = ticket_field(&id, "comments");
        let (got_id, field) = parse_ticket_field(&key).expect("should parse");
        assert_eq!(got_id, id);
        assert_eq!(field, "comments");
    }

    #[test]
    fn parse_ticket_field_rejects_non_ticket_keys() {
        assert!(parse_ticket_field("ticgit:owners").is_none());
        assert!(parse_ticket_field("ticgit:views:foo").is_none());
        assert!(
            parse_ticket_field("ticgit:writeups:00000000-0000-0000-0000-000000000001:title")
                .is_none()
        );
        assert!(parse_ticket_field("ticgit:tickets").is_none());
        assert!(parse_ticket_field("ticgit:tickets:not-a-uuid:title").is_none());
        assert!(parse_ticket_field("foo:bar:baz").is_none());
    }

    #[test]
    fn parse_writeup_field_round_trips_known_uuids() {
        let id = fixed_uuid();
        let key = writeup_field(&id, "versions");
        let (got_id, field) = parse_writeup_field(&key).expect("should parse");
        assert_eq!(got_id, id);
        assert_eq!(field, "versions");
    }

    #[test]
    fn parse_writeup_field_rejects_non_writeup_keys() {
        assert!(parse_writeup_field("ticgit:owners").is_none());
        assert!(parse_writeup_field("ticgit:views:foo").is_none());
        assert!(
            parse_writeup_field("ticgit:tickets:00000000-0000-0000-0000-000000000001:title")
                .is_none()
        );
        assert!(parse_writeup_field("ticgit:writeups").is_none());
        assert!(parse_writeup_field("ticgit:writeups:not-a-uuid:title").is_none());
        assert!(parse_writeup_field("foo:bar:baz").is_none());
    }

    #[test]
    fn parse_view_name_works() {
        assert_eq!(parse_view_name("ticgit:views:mine"), Some("mine"));
        assert_eq!(parse_view_name("ticgit:views:foo-bar"), Some("foo-bar"));
        assert_eq!(parse_view_name("ticgit:owners"), None);
        assert_eq!(parse_view_name("ticgit:views:"), None);
    }
}
