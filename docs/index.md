# TicGit

TicGit is a Git-native issue tracker. Tickets live in the repository as structured `git-meta` metadata instead of files on an orphan branch. The command-line tool is named `ti`.

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
ti show <id> --filter .title
ti show <id> --filter .comments[0].body
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
ti state blocked --ticket <id>
ti state closed --ticket <id>
ti state closed:wontfix --ticket <id>
ti status review --ticket <id>
ti assign you@example.com --ticket <id>
ti points 3 --ticket <id>
ti milestone v1.0 --ticket <id>
ti tag --ticket <id> bug ui
ti edit <id>
ti comment --ticket <id> "fixed in the latest patch"
ti close <id>
```

Lifecycle values are split into a broad `status` and a specific `state`. Open tickets use `new`, `assigned`, `in-progress`, `blocked`, or `review`. Closed tickets use `resolved`, `wontfix`, `duplicate`, or `invalid`. New tickets start as `open:new`; `ti state closed` defaults to `closed:resolved`.

## Machine Output

Use `--markdown` for agent workflows and `--json` for scripts that need stable schema output:

```sh
ti list --markdown
ti show <id> --markdown
ti new --title "agent task" --markdown
ti state blocked --ticket <id> --markdown
ti comment --ticket <id> "blocked on parser fixture" --markdown
```

The stable machine-output schema is published at [`schema/v1.json`](schema/v1.json). `ti show --json` and JSON mutation commands emit a ticket object. `ti list --json` emits an array of ticket objects. Commands that support `--json` also support `--markdown`, which renders the same ticket data as Markdown and includes suggested next commands. Ticket metadata appears under `.meta` as an object whose values are strings.

`--json` is the stable machine interface:

- successful JSON commands write parseable JSON to stdout only
- diagnostic and error text goes to stderr
- JSON output does not include ANSI color escapes
- non-zero exit status means the command failed
- ticket ids may be full UUIDs or unique UUID prefixes
- ambiguous or missing prefixes fail with a non-zero exit status and stderr diagnostic

`--porcelain` and `--format json` are not supported compatibility aliases today; use `--json` for schema-stable output.

Agents can also run:

```sh
ti help --agent
```

That prints an agent-focused Markdown guide directly from the installed CLI.

## Sync

Ticket metadata is separate from normal Git commits. Sync it explicitly when collaborating:

```sh
ti sync
```

`ti sync` performs a pull followed by a push through `git-meta`.

## GitButler

TicGit is great on its own, but it's even better with [GitButler](https://gitbutler.com). GitButler is a wonderful Git client that lets you work on several branches at once in a single working directory, so you can juggle a pile of tickets without stashing, switching, or losing your place. Its `but` CLI is a joy to use, and TicGit is built to take advantage of it.

If `but` is on your `PATH`, TicGit's review commands use it automatically:

```sh
ti review new --branch <branch-name> --ticket <id>
ti review show <branch-name>
ti review update <branch-name>
```

- Branch pickers are populated from `but branch list`, so every applied virtual branch and stacked head shows up as a review candidate, complete with commit counts, authors, and last-commit times.
- Review snapshots come from `but branch show`, which knows the real base and commit range of a stacked branch instead of guessing from refs.
- GitButler's own bookkeeping refs (`gitbutler/*`) are filtered out, so the list only ever offers branches you actually want to review.

None of this is required. When `but` is not installed, TicGit falls back to plain `git for-each-ref` and `git rev-list` and everything keeps working. You just get a nicer experience with GitButler installed.

## What TicGit Stores

All TicGit data is written on the git-meta `project` target under the `ticgit:` namespace:

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

Ticket existence is implied by the presence of fields under `ticgit:tickets:<uuid>:*`; there is no separate ticket index. Exchange with other clones happens through `refs/meta/*` using normal Git transfer.
