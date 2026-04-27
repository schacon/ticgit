# Git Metadata Repository

This ref stores structured metadata for the project at:

    git@github.com:schacon/ticgit.git

It is managed by [git meta](https://git-meta.com/), which associates
key-value metadata with Git objects (commits, branches, paths, change-ids,
and project-wide settings) and synchronises them across repositories using
ordinary Git transports.

## How it works

Metadata lives locally in a SQLite database (`.git/git-meta.sqlite`) and is
serialized into Git trees and commits under `refs/meta/` for transport.
This remote stores the canonical history under `refs/meta/main`; the
`main` branch you may see at the repository root is unrelated and only exists
for browsing.

Other contributors do **not** clone this repository directly. Instead they
configure it as a metadata remote on top of their existing checkout:

```
git meta remote add git@github.com:schacon/ticgit.git --name meta --namespace meta
git meta pull
```

After that, reading and writing metadata works against the project's normal
checkout:

```
git meta get commit:HEAD
git meta set commit:HEAD review:status approved
git meta push
```

## Important notes

- Metadata is exchanged on `refs/meta/main`, never on `refs/heads/main`.
- Never push directly to `refs/meta/main` -- always go through
  `git meta push`, which serializes local changes and resolves conflicts.
- This README only lives in the very first commit on `refs/meta/main`;
  later metadata commits replace the tip tree with the metadata layout.
