# TicGit

TicGit is a Git-native issue tracker. Tickets live in the repository as
structured [git-meta](https://crates.io/crates/git-meta-lib) metadata instead of
files on an orphan branch.

This is a clean Rust reimplementation of the old `ticgit-ng` idea. It does not
read or migrate legacy `ticgit-ng` branches.

## What It Stores

All TicGit data is written on the git-meta `project` target under the
`ticgit:` namespace:

```text
ticgit:schema-version                    string
ticgit:owners                            set
ticgit:views:<name>                      set of ticket UUIDs
ticgit:tickets:<uuid>:title              string
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

## Install

From a checkout:

```sh
cargo install --path crates/ticgit
```

After the crates are published:

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
ti list --state open
ti list --tag bug
ti list --assigned you@example.com
ti list --order title.desc
ti list --json
```

Show details:

```sh
ti show <id>
ti show <id> --json
```

Select a current ticket:

```sh
ti checkout <id>
ti show
ti comment "follow-up note"
ti checkout --clear
```

Mutate tickets:

```sh
ti state resolved --ticket <id>
ti assign you@example.com --ticket <id>
ti assign --clear --ticket <id>
ti points 3 --ticket <id>
ti milestone v1.0 --ticket <id>
ti tag --ticket <id> bug ui
ti tag --ticket <id> --remove ui
ti comment --ticket <id> "fixed in the latest patch"
```

Recent tickets:

```sh
ti recent
ti recent --limit 20
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

## Notes

This project intentionally avoids the old `ticgit-ng` branch format. The new
format uses structured string, set, and list values with deterministic git-meta
merge behavior, which keeps ticket metadata queryable locally and shareable via
Git refs.
