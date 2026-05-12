---
name: ticgit
description: Use TicGit (`ti`) to track Git-native tickets in this repository.
---

# TicGit Agent Guide

TicGit stores tickets as Git metadata. Use `ti` for planning, progress notes,
triage, and resolving work. Prefer commands with `--markdown` when reading
ticket data because Markdown output includes useful context and next commands.

## Basic Workflow

Find work:

```sh
ti list --markdown
ti next --markdown
ti show <id> --markdown
```

Select a current ticket when you will run several commands against it:

```sh
ti checkout <id>
ti show --markdown
ti comment "progress update"
```

Claim work before starting:

```sh
ti claim
```

## Create And Edit Tickets

Create a ticket from a file. The first line is the title; the rest is the
description:

```sh
ti new -F /tmp/ticket.md --tags bug,parser --markdown
```

Edit title and description:

```sh
ti edit <id>
ti edit <id> -F /tmp/ticket.md
```

## Progress Notes

Add comments for useful observations, plans, blockers, and verification:

```sh
ti comment -t <id> "found the failing case"
ti comment "implemented fix; running cargo test -p ticgit"
ti comment -t <id> --edit
```

## State And Triage

Tickets have a broad status and a specific state:

```text
open: new, assigned, in-progress, blocked, review
closed: resolved, wontfix, duplicate, invalid
```

Useful updates:

```sh
ti state blocked -t <id>
ti state review -t <id>
ti close -t <id>
```

`ti close` resolves the ticket and records the current user as `closed_by`.

## Planning Fields

Use priority, tags, estimates, and milestones to keep work easy to sort:

```sh
ti priority -t <id> 2
ti tag -t <id> bug parser
ti points -t <id> 3
ti milestone -t <id> v1.0
```

Use `spec` for implementation notes before coding:

```sh
ti spec -t <id> -F /tmp/spec.md
ti show <id> --filter .spec
```

## Dependencies

Track ordering constraints explicitly:

```sh
ti dep <blocker-id> -t <id>
ti dep <blocker-id> -t <id> --remove
```

`ti next` skips tickets with unresolved dependencies.

## Saved Views And Sync

Save common filters:

```sh
ti list --tag bug
ti views save bugs
ti list bugs --markdown
```

Sync ticket metadata when collaborating:

```sh
ti sync
```

## Agent Practices

- Use ticket IDs or unique prefixes.
- Prefer `--markdown` for reading tickets.
- Check for a spec before implementing; add one if the path is unclear.
- Comment when you learn something important or finish a meaningful step.
- Mark blockers with `ti state blocked` and dependencies with `ti dep`.
- Resolve tickets only after implementation and verification are complete.
