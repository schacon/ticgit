# Ticgit Rust Reimplementation Plan

A plan to rewrite the legacy Ruby `TicGit` ticketing tool as a modern Rust
CLI on top of the [`git-meta-lib`](https://crates.io/crates/git-meta-lib)
crate, replacing the `ticgit` orphan branch storage with structured
metadata stored under `refs/meta/*`.

This is a **clean reimplementation**. There is no backward-compatibility
requirement and no importer for the legacy `ticgit` branch.

---

## 1. Background

### What ticgit was

The original `TicGit` was a 2008-era ticketing system implemented in
Ruby, built around two ideas:

- Tickets live **in the same repo as the code**, so you don't depend on
  GitHub Issues or another forge.
- Tickets are stored on an **orphan branch** (`ticgit` / `ticgit-ng`) so they
  never appear in the working tree of `master`.

A ticket was a directory on that branch full of marker files. The state of
the ticket was encoded in the _names_ of those files:

```
1283813272_my-new-ticket-901/
  TICKET_ID
  TICKET_TITLE
  TITLE
  STATE_open
  ASSIGNED_jeff.welling@gmail.com
  TAG_bug
  COMMENT_1283813272_jeff.welling@gmail.com
```

State changes worked by **deleting the old marker file and committing a new
one** (e.g. removing `STATE_open` and adding `STATE_resolved`). All commands
went through a per-checkout cache at `~/.ticgit-ng/<projhash>/{working,index}`
which checked out the orphan branch into a side worktree.

### Why this is worth replacing

The old design has serious drawbacks today:

1. **Filename-as-data** — encoding `STATE`, `ASSIGNED`, `TAG`, etc. into
   filenames was clever but is brittle and not query-friendly.
2. **Orphan branch hacks** — the `in_branch` flow swaps `HEAD`, indexes, and
   working trees by hand. It is fragile and predates `git worktree`.
3. **Side cache in `~/.ticgit-ng`** — duplicate working tree + index, which
   has to be repaired when it falls out of sync.
4. **No structured merge** — two people editing the same ticket on different
   clones get text-level merge conflicts on marker files.
5. **Ruby + the unmaintained `git` gem** — installation is painful in 2026.

### Why git-meta is the natural successor

[`git-meta`](https://git-meta.com/) is exactly what TicGit was reaching
for, done properly:

- attach typed key/value metadata to **commits, branches, paths, change-ids,
  or the project**;
- store locally in **SQLite** (`.git/git-meta.sqlite`) for fast queries;
- exchange via **commits on `refs/meta/*`** that push and pull through any
  normal Git remote — no special server, no orphan branch in the user's
  repo;
- typed values (`string`, `list`, `set`) with **deterministic merge
  semantics**;
- supports **partial / blobless clones** so metadata scales to millions of
  entries.

Mapping ticgit onto git-meta is essentially:

| Old TicGit concept           | New git-meta representation                                               |
| ---------------------------- | ------------------------------------------------------------------------- |
| `ticgit` orphan branch       | `refs/meta/local/main` + `refs/meta/remotes/*`                            |
| Ticket directory             | A UUID under `ticgit:tickets:<uuid>:*` on the `project`                   |
| `TITLE` marker               | `ticgit:tickets:<uuid>:title` string                                      |
| `STATE_open`                 | `ticgit:tickets:<uuid>:state` string                                      |
| `ASSIGNED_<email>`           | `ticgit:tickets:<uuid>:assigned` string                                   |
| `POINTS`                     | `ticgit:tickets:<uuid>:points` string                                     |
| `TAG_<name>` files           | `ticgit:tickets:<uuid>:tags` set                                          |
| `COMMENT_<ts>_<email>` files | `ticgit:tickets:<uuid>:comments` list (entries are timestamped, authored) |
| Saved list views (yaml)      | `ticgit:views:<name>` sets of ids                                         |
| Per-project state cache      | git-meta SQLite, plus a small local UI cache (see §3)                     |

---

## 2. Goals and non-goals

### Goals

- A single `ti` Rust binary built with stable Rust.
- Feature parity with `ti list / show / new / state / tag / assign / comment
/ points / checkout / recent / sync` — i.e. the commands documented in the
  current README.
- Storage entirely through `git-meta-lib`. **No more orphan ticgit branch.**
- Sensible UX: quick startup, `--json` output, table output that respects
  `tput cols`, and zero hidden working trees.
- Cross-platform (macOS, Linux, Windows) without ioctl tricks for terminal
  width.

### Non-goals

- **No legacy importer.** Clean break from the Ruby orphan-branch storage.
- The Sinatra `ticgitweb` UI. The data model will support it, but a web UI
  is out of scope for v1.
- `attach` (file attachments). Defer until git-meta gains a blessed path
  for blob-valued metadata; track as a follow-up.
- Milestones beyond a simple string field.

---

## 3. Data model

Everything ticgit stores is keyed under the **`ticgit:` namespace** on the
**`project` target**. A single target keeps the data model very simple,
makes "load all tickets" a single SQLite read, and avoids spreading ticket
state across many independently-mutated targets.

```
target = project
keys:
  ticgit:<system-key>                       # ticketing-system metadata
  ticgit:tickets:<uuid>:<field>             # per-ticket fields
```

### Tickets are not change-ids

Tickets in this design are **first-class, project-scoped objects keyed by
their own UUID**. They are explicitly _not_ `change-id` targets — a ticket
is not a property of a logical change, and its lifetime is independent of
any commit or changeset.

The relationship goes the other way: a `change-id` (or `commit:`) target
can carry a `ticgit:fixes` (or similar) key that _points at_ a ticket
UUID. That is a follow-up feature, not part of v1's storage model.

### Ticket identity

Each ticket gets a v4 UUID at creation time and is addressed in the data
model by that UUID alone:

```
ticgit:tickets:<uuid>:title
ticgit:tickets:<uuid>:state
…
```

UUIDs use lowercase hex with hyphens; they pass git-meta's key-segment
validation (no `:`, `/`, null bytes, no leading `__`).

### Per-ticket keys

| Key                                | Type   | Notes                                                                 |
| ---------------------------------- | ------ | --------------------------------------------------------------------- |
| `ticgit:tickets:<uuid>:title`      | string | Required. Set on creation, editable.                                  |
| `ticgit:tickets:<uuid>:state`      | string | One of `open`, `resolved`, `invalid`, `hold`. Default `open`.         |
| `ticgit:tickets:<uuid>:assigned`   | string | Email or handle, optional.                                            |
| `ticgit:tickets:<uuid>:points`     | string | Stored as string, parsed as integer for display.                      |
| `ticgit:tickets:<uuid>:milestone`  | string | Optional.                                                             |
| `ticgit:tickets:<uuid>:tags`       | set    | Free-form tag strings.                                                |
| `ticgit:tickets:<uuid>:comments`   | list   | Each entry is a comment body; `ListEntry` carries author + timestamp. |
| `ticgit:tickets:<uuid>:created-at` | string | RFC3339 timestamp — denormalised for cheap reads.                     |
| `ticgit:tickets:<uuid>:created-by` | string | Email of the original author.                                         |

The `comments` list value relies on git-meta's built-in `ListEntry`
authorship, so we get author + timestamp per comment without inventing our
own filename convention.

### System-wide ticgit keys

Anything that is metadata _about the ticketing system itself_ — i.e. not
about one ticket — lives under bare `ticgit:` keys on the same `project`
target.

| Key                     | Type   | Notes                                                                                                        |
| ----------------------- | ------ | ------------------------------------------------------------------------------------------------------------ |
| `ticgit:owners`         | set    | Project-wide ticket owners / triagers.                                                                       |
| `ticgit:views:<name>`   | set    | A saved selection: a set of ticket UUIDs the user wants to revisit (replaces the old `list_options` yaml). |
| `ticgit:schema-version` | string | `"1"` for v1; bumped if the layout changes.                                                                  |

The bare-key form (`ticgit:owners`, `ticgit:schema-version`, …) is the
"project-level metadata about ticgit" surface. The nested form
(`ticgit:tickets:<uuid>:…`) is per-ticket data. No other shapes.

**Note on saved views.** In the original ticgit, `ti list -S name` saved
the *filter options* (state, tags, assigned, …) so they could be replayed
later. In this design a view is instead a **frozen set of ticket UUIDs**,
captured at save time. That's simpler, deterministic, and merges
naturally as a `set` value across clones. Users who want a recurring
filter just re-run `ti list -t bug -s open`.

### Reading tickets efficiently

Because everything lives on one target with a shared prefix, loading the
state of every ticket in the project is a single call:

```rust
let pairs = session
    .target(&Target::project())
    .get_all_values(Some("ticgit:tickets"))?;
// returns every `ticgit:tickets:<uuid>:<field>` value;
// distinct ticket UUIDs are derived from the keys themselves.
```

The CLI parses that into an in-memory map of `Uuid -> Ticket` once per
command invocation. `ti list` filters/sorts that map; `ti show <prefix>`
resolves `<prefix>` against the UUIDs it already has. There is **no
separate index** to keep in sync — the set of tickets in the project is
exactly the set of UUIDs that have at least one `ticgit:tickets:<uuid>:*`
field.

### Session-local UI state (NOT in git-meta)

The "currently checked out" ticket and "last list" used for index-based
references (`ti checkout 3`) are **machine-local UI ephemerals** and don't
belong in shared metadata. We keep them out of git-meta entirely:

- `~/.config/ticgit/<repo-key>.json` (XDG on Linux/macOS, AppData on
  Windows) for `current_ticket` and `last_list`.

This matches the _intent_ of the original `~/.ticgit-ng/<proj>/state` file
without mixing UI-only state into shared, mergeable metadata.

---

## 4. Crate layout

A two-crate Cargo workspace, mirroring how `git-meta-lib` / `git-meta-cli`
are split:

```
ticgit/
├── Cargo.toml                  # workspace
├── crates/
│   ├── ticgit-lib/             # library: ticket model + git-meta integration
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── ticket.rs       # Ticket struct, TicketState, parsing
│   │       ├── store.rs        # thin wrapper over git_meta_lib::Session
│   │       ├── keys.rs         # key-string helpers (one place that knows the layout)
│   │       ├── query.rs        # list filters, sorts, saved views
│   │       └── error.rs
│   └── ticgit/                 # CLI binary: argument parsing + rendering
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs
│           ├── cli.rs          # clap definitions
│           ├── commands/       # one file per subcommand
│           │   ├── new.rs
│           │   ├── list.rs
│           │   ├── show.rs
│           │   ├── state.rs
│           │   ├── tag.rs
│           │   ├── assign.rs
│           │   ├── comment.rs
│           │   ├── points.rs
│           │   ├── checkout.rs
│           │   ├── recent.rs
│           │   └── sync.rs     # wraps git_meta_lib pull/push_once
│           ├── render.rs       # table + json formatting
│           ├── editor.rs       # $EDITOR helper
│           └── session_state.rs# local checkout/last_list cache
├── README.md
├── LICENSE_MIT (kept)
└── PLAN.md
```

### Key dependencies

```toml
# ticgit-lib
git-meta-lib = "0.1"
gix          = { version = "0.81", default-features = false }
serde        = { version = "1", features = ["derive"] }
serde_json   = "1"
thiserror    = "2"
time         = { version = "0.3", features = ["formatting", "parsing", "macros"] }
uuid         = { version = "1", features = ["v4"] }

# ticgit
anyhow       = "1"
clap         = { version = "4", features = ["derive"] }
comfy-table  = "7"          # terminal table rendering
crossterm    = "0.28"       # cross-platform terminal width
dialoguer    = "0.11"       # interactive prompts (parity with git-meta-cli)
dirs         = "5"          # XDG / AppData paths
```

---

## 5. Core API sketch

```rust
// ticgit-lib/src/keys.rs — the one place that knows the layout

pub const NS: &str = "ticgit";

/// Prefix for the per-ticket field keyspace; used as the argument to
/// `get_all_values` for project-wide ticket scans.
pub fn tickets_prefix() -> String                     { format!("{NS}:tickets") }
pub fn ticket_field(id: Uuid, field: &str) -> String  { format!("{NS}:tickets:{id}:{field}") }
pub fn system_key(name: &str) -> String               { format!("{NS}:{name}") }
pub fn view(name: &str) -> String                     { format!("{NS}:views:{name}") }
```

```rust
// ticgit-lib/src/ticket.rs

use std::collections::BTreeSet;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TicketState { Open, Resolved, Invalid, Hold }

#[derive(Debug, Clone)]
pub struct Comment {
    pub author: String,
    pub at:     OffsetDateTime,
    pub body:   String,
}

#[derive(Debug, Clone)]
pub struct Ticket {
    pub id:         Uuid,
    pub title:      String,
    pub state:      TicketState,
    pub assigned:   Option<String>,
    pub points:     Option<i64>,
    pub milestone:  Option<String>,
    pub tags:       BTreeSet<String>,
    pub comments:   Vec<Comment>,
    pub created_at: OffsetDateTime,
    pub created_by: String,
}
```

```rust
// ticgit-lib/src/store.rs

use git_meta_lib::{MetaValue, Session, Target};
use crate::keys;

pub struct TicketStore { session: Session }

impl TicketStore {
    pub fn discover() -> Result<Self> { /* Session::discover() */ }

    fn project(&self) -> SessionTargetHandle<'_> {
        self.session.target(&Target::project())
    }

    pub fn create(&self, title: &str, opts: NewTicketOpts) -> Result<Ticket> {
        let id = Uuid::new_v4();
        let p  = self.project();

        // A ticket's existence is implied by its fields — no separate
        // index to maintain. Writing `title` and `state` is enough for
        // a prefix scan to discover the ticket.
        p.set(&keys::ticket_field(id, "title"),      title)?;
        p.set(&keys::ticket_field(id, "state"),      "open")?;
        p.set(&keys::ticket_field(id, "created-at"), now_rfc3339())?;
        p.set(&keys::ticket_field(id, "created-by"), self.session.email())?;

        if let Some(c) = opts.comment {
            p.list_push(&keys::ticket_field(id, "comments"), &c)?;
        }
        for t in opts.tags {
            p.set_add(&keys::ticket_field(id, "tags"), &t)?;
        }

        self.load(id)
    }

    /// One round-trip: a prefix scan returns every per-ticket field for
    /// every ticket. Distinct UUIDs are derived from the returned keys.
    pub fn list(&self) -> Result<Vec<Ticket>> {
        let pairs = self.project().get_all_values(Some(&keys::tickets_prefix()))?;
        Ok(parse_tickets(pairs))
    }

    pub fn load(&self, id: Uuid) -> Result<Ticket> {
        let prefix = format!("{}:{}", keys::tickets_prefix(), id);
        let pairs  = self.project().get_all_values(Some(&prefix))?;
        parse_one_ticket(id, pairs)
    }

    pub fn set_state(&self, id: Uuid, s: TicketState) -> Result<()> {
        self.project().set(&keys::ticket_field(id, "state"), s.as_str())
    }
    pub fn set_assigned(&self, id: Uuid, who: Option<&str>) -> Result<()>;
    pub fn add_tag(&self, id: Uuid, tag: &str) -> Result<()>;
    pub fn remove_tag(&self, id: Uuid, tag: &str) -> Result<()>;
    pub fn add_comment(&self, id: Uuid, body: &str) -> Result<()>;
    pub fn set_points(&self, id: Uuid, p: Option<i64>) -> Result<()>;

    pub fn pull(&self, remote: Option<&str>) -> Result<()>; // delegate to Session
    pub fn push(&self, remote: Option<&str>) -> Result<()>;
}
```

The crucial points:

- **One target** (`project`) so listing every ticket is a single
  `get_all_values` call.
- **All writes go through `SessionTargetHandle`**, which gives us free
  authorship and timestamps and built-in merge semantics for `list`/`set`
  values.
- **`keys.rs` is the only module that knows the key layout.** Callers
  never hand-format key strings.

---

## 6. CLI surface

The CLI keeps the original `ti` verbs (and their existing flags from the
README) so muscle memory survives.

```
ti --help

USAGE: ti <COMMAND> [OPTIONS] [ARGS]

CORE
  new                Create a new ticket
  list               List tickets (with filters / sort / saved views)
  show <id>          Show one ticket
  state <state>      Change state (open|resolved|invalid|hold)
  tag <tag>          Add tag (-d to remove)
  assign -u <user>   Assign ticket
  comment            Add a comment (-m, -f, or $EDITOR)
  points <n>         Set points
  checkout <id>      Make a ticket "current" for this checkout
  recent             Recent activity (from refs/meta log)

REMOTE
  pull [remote]      git meta pull
  push [remote]      git meta push
  sync [remote]      pull && push (TicGit-ng compatibility verb)

SETUP
  init               Run `git meta setup` and create local meta refs

GLOBAL
  --json             Machine-readable output
  --repo <path>      Override repo discovery
  -v, --version
  -h, --help
```

### Mapping to the original `ti` commands

Every command listed in the README has a 1:1 mapping; flag names are kept
where it doesn't introduce a clash with clap conventions:

| `ti` command                   | Effect on `project` target                                                       |
| ------------------------------ | -------------------------------------------------------------------------------- |
| `ti new -t TITLE`              | mint UUID; `set ticgit:tickets:<id>:title` and the other initial fields          |
| `ti list`                      | `get_all_values("ticgit:tickets")`; filter / sort in memory                      |
| `ti list -t TAG -s STATE -a U` | same flags, applied as in-memory filters                                         |
| `ti list -S name`              | snapshot the current result UUIDs into the `ticgit:views:<name>` set             |
| `ti list name`                 | read `ticgit:views:<name>` and show only those tickets                           |
| `ti list -l`                   | enumerate `ticgit:views:*` keys                                                  |
| `ti show [id]`                 | one ticket; `--full` shows untruncated comments                                  |
| `ti state STATE`               | `set ticgit:tickets:<id>:state`                                                  |
| `ti tag T`                     | `set_add ticgit:tickets:<id>:tags`; `-d` → `set_remove`                          |
| `ti assign -u U`               | `set ticgit:tickets:<id>:assigned`                                               |
| `ti points N`                  | `set ticgit:tickets:<id>:points`                                                 |
| `ti comment -m / -f / $EDITOR` | `list_push ticgit:tickets:<id>:comments`                                         |
| `ti checkout id`               | local cache only                                                                 |
| `ti recent`                    | walk `refs/meta/local/main` log via `gix`                                        |
| `ti sync --repo R`             | `Session::pull(Some(R))` + `push_once(Some(R))`                                  |

### Ticket addressing

Three ways to address a ticket, matching the original UX:

1. **By UUID prefix** — any unique prefix of a ticket UUID
   (case-insensitive, hyphens optional). Replaces "first 6 chars of the
   SHA" from legacy ticgit.
2. **By index from the last `ti list`** — local cache
   (`session_state.rs`).
3. **No argument** — falls back to `current_ticket` from the local cache.

---

## 7. Sync model

The original `ti sync` command did:

```
git fetch <remote>
git pull  <remote> <remote>/ticgit-ng    # into the orphan branch
git push  <remote> ticgit-ng:ticgit-ng
```

The replacement is much simpler because git-meta already does this:

```rust
// ti pull
session.pull(remote.as_deref())?;

// ti push  (with one fast-forward retry)
loop {
    match session.push_once(remote.as_deref())? {
        PushOutput::Ok            => break,
        PushOutput::NonFastForward => session.resolve_push_conflict(remote.as_deref())?,
    }
}

// ti sync = pull then push (kept for muscle memory)
```

A `.git-meta` file in the repo root pins the recommended metadata remote,
so `ti init` can run `git meta setup` for first-time users.

---

## 8. Output and rendering

### Table mode (default)

`comfy-table` produces the existing `ti list` columns:

```
   #   TicId  Title                       State Date  Assgn    Tags
 ---------------------------------------------------------------------
 *  1  d7f2d8 my new ticket               open  09/06 jeff.we… bug
```

We display the **first 6 chars of the UUID** in the `TicId` column, which
fits the layout the README documents and is enough for unique-prefix
resolution in any reasonably-sized project.

Terminal width comes from `crossterm::terminal::size()` — no ioctl tricks
needed and it Just Works on Windows.

### JSON mode

Every list / show command supports `--json`, emitting a stable schema:

```json
{
  "id": "5e7c1a30-…",
  "title": "my new ticket",
  "state": "open",
  "assigned": "jeff.welling@gmail.com",
  "points": null,
  "tags": ["bug"],
  "comments": [
    { "author": "scott@…", "at": "2026-04-27T12:00:00Z", "body": "lgtm" }
  ],
  "created_at": "2026-04-27T11:00:00Z",
  "created_by": "scott@…"
}
```

This is what enables editor integrations and the eventual `ticgitweb`
revival on top of the same data.

---

## 9. Editor integration

Reproduce the existing `ti new` / `ti comment` editor-buffer flow:

- prefill a temporary file under `std::env::temp_dir()` with the comment
  template,
- spawn `$EDITOR` (default `vi` on Unix, `notepad` on Windows) via
  `std::process::Command`,
- strip lines starting with `#`, parse a leading title and trailing `tags:`
  line for `ti new`.

`dialoguer` is already a dep (via the `git-meta-cli` ecosystem) and gives
us a nice fallback for the very common interactive prompts (e.g. confirm
state change).

---

## 10. Testing strategy

### Unit tests (in `ticgit-lib`)

- Round-trip: `create()` → `load()` → assert all fields match.
- Filter & sort: feed a fixed list of tickets, assert filter combinations
  match the matrix from the original specs in `spec/`.
- Key layout: golden test that locks down the exact serialized key
  strings for every ticket field (so we don't silently break the wire
  format on a refactor).

### Integration tests (in `ticgit`)

- `assert_cmd` + `tempfile` to create a fresh git repo per test.
- Walk a "happy path" script: `ti init`, `ti new`, `ti list`, `ti tag`,
  `ti state`, `ti show --json`, then assert JSON.
- Two-clone test: clone the repo, `ti push`, clone again, `ti pull`,
  assert ticket parity. This is the closest thing to the old
  `merging_spec.rb`.

### Cross-version compatibility

Pin `git-meta-lib = "0.1"`; bump together with the CLI. Tag a v0 release
of ticgit only against a stable git-meta-lib version. Encode the data
layout version in `ticgit:schema-version` so a future v2 layout can be
detected and rejected with a clear error.

---

## 11. Implementation phases

A suggested order of work:

### Phase 0 — Scaffolding (≈ ½ day)

- Initialize Cargo workspace, two crates, lints, CI (GitHub Actions
  matrix for macOS / Linux / Windows on stable Rust).
- Wire up `clap` skeleton with all subcommands as no-op stubs that print
  "not yet implemented".
- Replace top-level `Rakefile`, `ticgit-ng.gemspec`, `lib/`, `bin/` with
  the new crates. Keep `LICENSE_MIT`, retire `LICENSE_GPL` if not needed.

### Phase 1 — Core model (≈ 1–2 days)

- `ticgit-lib::keys`, `ticket`, `store`, `error`.
- `Session::discover()` integration; `project`-target reads/writes.
- Implement `create / load / list / set_state / set_assigned / add_tag /
remove_tag / add_comment / set_points`.
- Unit tests for round-trip, key-layout golden test, and merge semantics
  on `tags` and `comments`.

### Phase 2 — CLI (≈ 1–2 days)

- All read-only subcommands first: `list`, `show`, `recent`.
- Then mutating ones: `new`, `state`, `tag`, `assign`, `comment`,
  `points`, `checkout`.
- `render::table` and `--json` output.
- `session_state.rs` for local-only `current_ticket` / `last_list`
  cache.

### Phase 3 — Sync & init (≈ ½ day)

- `ti init` → wraps `git meta setup`.
- `ti pull / push / sync` → wrappers around `Session::pull/push_once`,
  with the `resolve_push_conflict` retry loop.

### Phase 4 — Polish (≈ ½ day)

- `--help` text proofread, error-message audit.
- README rewrite (current `README.mkd` documents the dead Ruby flow).
- `cargo install ticgit` + GitHub release binaries for macOS / Linux /
  Windows.

Total: roughly half a focused week of work, with each phase
independently mergeable.

---

## 12. Open questions

1. **Binary name.** `ti` is short and matches the README, but it collides
   with several existing tools on $PATH (Texas Instruments compiler shim,
   etc.). Options:
   - keep `ti` but ship a `ticgit` symlink as well;
   - rename to `tg` (clashes with `topgit`);
   - go with `ticgit` only.

2. **Linking commits / changes to tickets.** A natural follow-up is keys
   like `ticgit:fixes` or `ticgit:relates` set values on `commit:` /
   `change-id:` targets that point at ticket UUIDs, so `ti show <id>` can
   render "fixed by `<commit-sha>`" cross-references. Out of scope for
   v1 but worth designing the keys consistently when v1 ships.

3. **File attachments.** Does `git-meta-lib` gain native support for blob
   values in 0.2, or do we lean on `commit:` targets (a
   "ticket-attachment" commit on a hidden ref) for now? Tracking as a
   follow-up.

4. **Hooks.** The legacy `examples/post-commit` triggered ticgit on
   commits. Do we want a `ti hook install` that drops a post-commit hook
   shelling out to `ti push --quiet`? Cheap to add, but optional.

---

## 13. Summary

The existing TicGit-ng codebase encodes a smart but dated idea (tickets
beside the code, no extra server) on top of a hand-rolled "data lives in
filenames on an orphan branch" hack. Every mechanism it invents — the
side worktree, the marker files, the manual merge story — is now provided
properly by `git-meta`.

The Rust port is therefore a thin domain layer over `git-meta-lib`:

- **One target**: `project`.
- **One namespace**: `ticgit:`.
- **Two key shapes**:
  - `ticgit:<name>` for system-wide ticketing metadata
    (`ticgit:owners`, `ticgit:views:<name>`, `ticgit:schema-version`, …),
  - `ticgit:tickets:<uuid>:<field>` for per-ticket data.
- **No separate ticket index.** A ticket exists iff at least one
  `ticgit:tickets:<uuid>:*` field exists for that UUID; one prefix scan
  recovers the whole project.
- **Tickets are not change-ids.** The relationship, when added later, is
  the other way around: a `change-id` or `commit` target carries a key
  pointing _at_ a ticket UUID.

The remaining work is honest CLI ergonomics (clap, rendering, editor
integration, JSON output) and wiring the project-target reads/writes —
no orphan branch, no side worktrees, no migration step.
