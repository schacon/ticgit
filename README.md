# ticgit

Ticgit is a Git-native issue tracker. Tickets live in the repository as
structured [git-meta](https://crates.io/crates/git-meta-lib) metadata.

The `ti` cli can create, read, update and sync ticket data.

<img width="2362" height="1712" alt="CleanShot 2026-05-13 at 09 47 31@2x" src="https://github.com/user-attachments/assets/f5ff1a77-644d-47ba-80eb-77e7b7ee66cb" />

Also ships with `ti tui` for a cool TUI version.

<img width="2590" height="1730" alt="CleanShot 2026-05-13 at 09 45 20@2x" src="https://github.com/user-attachments/assets/8c648b6a-0c13-4234-a11b-963fff4a7a2f" />

<img width="2590" height="1730" alt="CleanShot 2026-05-13 at 09 45 43@2x" src="https://github.com/user-attachments/assets/391b0f79-c487-4146-b7b8-a39fad2cde93" />

Everything has `--json` output for scripting and `--markdown` output for agentic use. You can also train your agent to use it by asking it to run `ti agent`. 

You can also do specs and writeups and lots of fun stuff.

## Install

Download a pre-built binary:

```sh
curl -fsSL https://ticgit.dev/install | sh
```

Or install from source via Cargo:

```sh
cargo install ticgit
```

The binary is named `ti`.

## Quick Start

```sh
git init
git config user.email you@example.com
git config user.name "Your Name"

ti init
ti new --title "fix the parser" --tags bug,parser --comment "fails on empty input"
ti list
ti show <ticket-id-or-prefix>
```

Most commands accept a full UUID or any unique UUID prefix.

## Common Commands

Create tickets:

```sh
ti new --title "add docs"
ti new --title "fix crash" --tags bug,cli --assigned you@example.com
ti new --title "investigate flaky test" --comment "seen on CI twice"
```

List and filter:

```sh
ti list
ti list --status open
ti list --state blocked
ti list --tag bug
ti list --assigned you@example.com
ti list --order title.desc
ti list --json
ti list --markdown
```

Show details:

```sh
ti show <id>
ti show <id> --json
ti show <id> --markdown
```

Commands that support `--json` also support `--markdown`, which renders the same
ticket data as Markdown and includes suggested next commands for agent workflows.

## Machine Output

TicGit publishes a stable JSON schema for agent and automation workflows at
[`docs/schema/v1.json`](docs/schema/v1.json). On the website, the same schema is
available at [`https://ticgit.dev/schema/v1.json`](https://ticgit.dev/schema/v1.json).

`--json` is the stable machine interface:

- successful JSON commands write parseable JSON to stdout only
- diagnostic and error text goes to stderr
- JSON output does not include ANSI color escapes
- non-zero exit status means the command failed
- ticket ids may be full UUIDs or unique UUID prefixes
- ambiguous or missing prefixes fail with a non-zero exit status and stderr diagnostic

`ti show <id> --json` and JSON mutation commands emit a ticket object.
`ti list --json` emits an array of ticket objects. Ticket metadata appears under
`.meta` as an object whose values are strings.

`--porcelain` and `--format json` are not supported compatibility aliases today;
use `--json` for schema-stable output.

Agents can run `ti help --agent` for a Markdown guide, or read the website's
Markdown version at [`docs/index.md`](docs/index.md).

Select a current ticket:

```sh
ti checkout <id>
ti show
ti comment "follow-up note"
ti checkout --clear
```

Mutate tickets:

```sh
ti state blocked --ticket <id>
ti state closed --ticket <id>
ti state closed:wontfix --ticket <id>
ti status review --ticket <id>
ti assign you@example.com --ticket <id>
ti assign --clear --ticket <id>
ti points 3 --ticket <id>
ti milestone v1.0 --ticket <id>
ti tag --ticket <id> bug ui
ti tag --ticket <id> --remove ui
ti edit <id>
ti comment --ticket <id> "fixed in the latest patch"
```

Lifecycle values are split into a broad `status` and a specific `state`.
Open tickets use `new`, `assigned`, `in-progress`, `blocked`, or `review`.
Closed tickets use `resolved`, `wontfix`, `duplicate`, or `invalid`.
New tickets start as `open:new`; `ti state closed` defaults to
`closed:resolved`.

Recent tickets:

```sh
ti recent
ti recent --limit 20
```

Import open GitHub issues:

```sh
ti import gh
ti import gh --repo owner/repo
```

Saved views are named snapshots of ticket UUIDs:

```sh
ti save-view bugs --tag bug
ti views
ti views bugs
ti list --view bugs
```

## Sync

TicGit delegates storage and transfer to `git-meta-lib`.

```sh
ti pull
ti push
ti sync
```

`ti sync` performs a pull followed by a push. If you pass `--remote <name>`, the
named git-meta remote is used; otherwise git-meta resolves the default metadata
remote from Git config.

## GitButler

TicGit is great on its own, but it's even better with
[GitButler](https://gitbutler.com). GitButler is a wonderful Git client that
lets you work on several branches at once in a single working directory, so you
can juggle a pile of tickets without stashing, switching, or losing your place.
Its `but` CLI is a joy to use, and TicGit is built to take advantage of it.

If `but` is on your `PATH`, TicGit's review commands use it automatically:

```sh
ti review new --branch <branch-name> --ticket <id>
ti review show <branch-name>
ti review update <branch-name>
```

- Branch pickers are populated from `but branch list`, so every applied virtual
  branch and stacked head shows up as a review candidate, complete with commit
  counts, authors, and last-commit times.
- Review snapshots come from `but branch show`, which knows the real base and
  commit range of a stacked branch instead of guessing from refs.
- GitButler's own bookkeeping refs (`gitbutler/*`) are filtered out, so the list
  only ever offers branches you actually want to review.

None of this is required. When `but` is not installed, TicGit falls back to
plain `git for-each-ref` and `git rev-list` and everything keeps working. You
just get a nicer experience with GitButler installed.

## What It Stores

All TicGit data is written on the git-meta `project` target under the
`ticgit:` namespace:

```text
ticgit:schema-version                    string
ticgit:owners                            set
ticgit:views:<name>                      set of ticket UUIDs
ticgit:tickets:<uuid>:title              string
ticgit:tickets:<uuid>:description        string (optional)
ticgit:tickets:<uuid>:status             string
ticgit:tickets:<uuid>:state              string
ticgit:tickets:<uuid>:assigned           string
ticgit:tickets:<uuid>:points             string
ticgit:tickets:<uuid>:milestone          string
ticgit:tickets:<uuid>:tags               set
ticgit:tickets:<uuid>:comments           list
ticgit:tickets:<uuid>:created-at         string
ticgit:tickets:<uuid>:created-by         string
```

Ticket existence is implied by the presence of fields under
`ticgit:tickets:<uuid>:*`; there is no separate ticket index.

The local query database is git-meta's `.git/git-meta.sqlite`. Exchange with
other clones happens through `refs/meta/*` using normal Git transfer.

## Rust API

The workspace has two crates:

- `ticgit-lib`: domain model and git-meta-backed `TicketStore`.
- `ticgit`: the `ti` command-line application.

Example:

```rust
use ticgit_lib::{NewTicketOpts, TicketStore};

let store = TicketStore::discover()?;
let ticket = store.create("fix parser", NewTicketOpts::default())?;
println!("{}", ticket.id);
Ok::<(), ticgit_lib::Error>(())
```

## Development

Run the full test suite:

```sh
cargo test
```

Run just the library tests:

```sh
cargo test -p ticgit-lib
```

Run the CLI integration tests:

```sh
cargo test -p ticgit --test cli
```

Build the CLI:

```sh
cargo build -p ticgit
```

Package the crates before publishing:

```sh
cargo package -p ticgit-lib
cargo publish -p ticgit-lib

# After ticgit-lib 0.1.0 is available in the crates.io index:
cargo package -p ticgit
cargo publish -p ticgit
```

The CLI crate depends on `ticgit-lib` by both local `path` and published
`version`, so publish `ticgit-lib` first.
