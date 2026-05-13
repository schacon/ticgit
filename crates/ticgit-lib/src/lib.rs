//! # ticgit-lib
//!
//! Ticket-tracking on top of [git-meta](https://crates.io/crates/git-meta-lib).
//!
//! Tickets live as project-target metadata under the `ticgit:` namespace:
//!
//! ```text
//! ticgit:tickets:<uuid>:title          # string
//! ticgit:tickets:<uuid>:description    # string (optional)
//! ticgit:tickets:<uuid>:status         # string ("open" | "closed")
//! ticgit:tickets:<uuid>:state          # string ("new" | "blocked" | ...)
//! ticgit:tickets:<uuid>:assigned       # string (optional)
//! ticgit:tickets:<uuid>:points         # string (optional integer)
//! ticgit:tickets:<uuid>:milestone      # string (optional)
//! ticgit:tickets:<uuid>:tags           # set
//! ticgit:tickets:<uuid>:meta:<key>     # string
//! ticgit:tickets:<uuid>:comments       # list of JSON-encoded {author, body}
//! ticgit:tickets:<uuid>:created-at     # RFC3339 string
//! ticgit:tickets:<uuid>:created-by     # string (email)
//! ticgit:writeups:<uuid>:title         # string
//! ticgit:writeups:<uuid>:status        # string ("open" | "closed")
//! ticgit:writeups:<uuid>:tags          # set
//! ticgit:writeups:<uuid>:authors       # set of emails
//! ticgit:writeups:<uuid>:versions      # list of markdown documents
//! ticgit:writeups:<uuid>:tickets       # set of linked ticket UUIDs
//! ticgit:views:<name>                  # set of UUIDs (saved selection)
//! ticgit:owners                        # set of emails
//! ticgit:schema-version                # string ("1")
//! ```
//!
//! See the top-level `README.md` and `docs/schema/v1.json` for higher-level
//! docs and the stable JSON machine-output schema.

pub mod error;
pub mod keys;
pub mod query;
pub mod store;
pub mod ticket;
pub mod writeup;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use error::{Error, Result};
pub use query::{Filter, SearchFilter, SearchScope, SortKey, SortOrder};
pub use store::TicketStore;
pub use ticket::{
    validate_code_uri, Comment, NewTicketOpts, Ticket, TicketLifecycle, TicketState, TicketStatus,
};
pub use writeup::{NewWriteupOpts, Writeup, WriteupStatus, WriteupVersion};

/// Re-exported for callers who want to talk to git-meta directly.
pub use git_meta_lib::{MetaValue, Session, Target};
