# ti serve

`ti serve` publishes the repository's tickets and writeups as a small read-only
website. It is the same data `ti list`, `ti show`, and `ti writeup list` print,
rendered as HTML pages you can link to and hand to people who do not have the
CLI installed.

The server is hand-rolled on `std::net`, so it pulls in no web framework and
has no runtime dependencies beyond `ti` itself.

## Quick Start

```sh
ti serve
ti serve --open
ti serve --port 9000
ti serve --port 0
```

`ti serve` prints the URL it bound to and then serves until you stop it with
`ctrl-c`:

```text
ti serve: listening on http://127.0.0.1:8177/ (ctrl-c to stop)
```

Run it from inside a TicGit repository; it fails immediately with the usual
error if there are no tickets to serve.

## Options

| Flag | Default | Meaning |
| --- | --- | --- |
| `-p`, `--port <PORT>` | `8177` | Port to listen on. Use `0` to let the OS pick a free port. |
| `--bind <ADDR>` | `127.0.0.1` | Address to bind. |
| `--open` | off | Open the served URL in your browser once it is listening. |

## Pages

| Path | Shows |
| --- | --- |
| `/` | The ticket list. |
| `/t/<id>` | One ticket: fields, description, spec, linked writeups, comments. |
| `/tickets.json` | The ticket list as JSON. |
| `/writeups` | The writeup list. |
| `/w/<id>` | One writeup: fields, linked tickets, version bodies. |
| `/writeups.json` | The writeup list as JSON. |

Ids in URLs are full UUIDs or any unique UUID prefix, the same as on the
command line, so `/t/d7f2d8` works.

The header on each list links to the other half of the site, and every ticket
detail page links to the writeups that reference it (and back again).

## Filtering The Ticket List

Query parameters on `/` mirror `ti list`'s flags. Clicking column headers,
tags, and filter chips in the UI just rewrites these.

| Param | Example | Meaning |
| --- | --- | --- |
| `status` | `?status=closed` | `open`, `closed`, or `all`. |
| `state` | `?state=blocked` | A specific lifecycle state; implies its status. |
| `tag` | `?tag=bug&tag=ui` | Repeatable. A ticket must carry **all** of the tags given. |
| `assigned` | `?assigned=you@example.com` | Assignee. |
| `q` | `?q=parser` | Search. Accepts `title:`, `description:`, and `comments:` prefixes. |
| `order` | `?order=priority.desc` | Sort key, optionally suffixed `.desc` or `.asc`. |
| `all` | `?all=1` | Include closed tickets. |
| `subissues` | `?subissues=1` | Include sub-issues. |

Sort keys are `priority`, `title`, `state`, `assigned`, and `created`.

With no parameters the list shows open tickets and hides sub-issues, matching
`ti list` and the TUI's default view. A state column appears whenever the
current view can contain closed tickets.

Boolean parameters accept `1`, `true`, `yes`, or an empty value, so `?all` and
`?all=1` are the same.

```text
/?tag=bug&tag=parser
/?assigned=you@example.com&order=priority
/?state=blocked
/?status=closed&order=created.desc
/?all=1&subissues=1
/?q=title:crash
```

## Filtering The Writeup List

Query parameters on `/writeups` follow `ti writeup list` — which only takes
`--all` — plus the same tag, author, search, and sort narrowing the ticket list
has.

| Param | Example | Meaning |
| --- | --- | --- |
| `status` | `?status=closed` | `open`, `closed`, or `all`. |
| `tag` | `?tag=perf` | Repeatable; must match all. |
| `author` | `?author=you@example.com` | Matches the creator or any listed author. |
| `q` | `?q=cache` | Search across title, tags, authors, and version bodies. |
| `order` | `?order=updated.desc` | Sort key, optionally suffixed `.desc`. |
| `all` | `?all=1` | Include closed writeups. |

Sort keys are `created`, `updated`, `title`, `status`, `versions`, and
`priority`. With no `order`, the store's own ordering is kept: priority first,
then newest.

A writeup detail page shows its latest version by default:

```text
/w/<id>          the latest version
/w/<id>?v=2      version 2 (out-of-range numbers fall back to the latest)
/w/<id>?all=1    every version, oldest first
```

## JSON

`/tickets.json` takes the same filters as `/` and emits the schema-stable
ticket array documented in [`schema/v1.json`](schema/v1.json) — the same output
as `ti list --json`.

`/writeups.json` takes the same filters as `/writeups` and emits an array of
writeup objects with `id`, `short_id`, `title`, `status`, `priority`,
`created_at`, `created_by`, `updated_at`, `authors`, `tags`, `tickets`, and
`versions` (each with `author`, `at`, and `body`). Timestamps are RFC 3339.

Both endpoints are reachable from the "JSON" link at the bottom of the
corresponding list page, which carries the current filters over.

```sh
curl -s 'http://127.0.0.1:8177/tickets.json?tag=bug' | jq '.[].title'
curl -s 'http://127.0.0.1:8177/writeups.json?all=1' | jq '.[].short_id'
```

## Notes

- **Read-only.** The server never writes: there are no forms that mutate, and
  anything other than `GET` or `HEAD` gets a `405`.
- **No authentication.** Anyone who can reach the port can read every ticket
  in the repository. The default bind is `127.0.0.1`, so it stays on your
  machine; passing `--bind 0.0.0.0` exposes the tickets to your whole network.
- **Always current.** Each request reads the store fresh and responses are
  sent with `Cache-Control: no-store`, so a reload picks up tickets changed by
  `ti` or pulled in by `ti sync` — no restart needed.
- **One request at a time.** Connections are handled sequentially, which is
  fine for a person or two browsing but is not a production web server.
