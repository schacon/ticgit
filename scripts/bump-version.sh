#!/usr/bin/env bash
set -euo pipefail

if [ $# -ne 1 ]; then
  echo "Usage: $0 <new-version>"
  echo "Example: $0 0.2.0"
  exit 1
fi

NEW="$1"
ROOT="$(cd "$(dirname "$0")" && pwd)"

# Update workspace version in Cargo.toml
sed -i '' "s/^version = \".*\"/version = \"${NEW}\"/" "$ROOT/Cargo.toml"

# Update version strings in docs/index.html
sed -i '' "s/ticgit [0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*/ticgit ${NEW}/g" "$ROOT/docs/index.html"
sed -i '' "s/>[0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*</>$NEW</g" "$ROOT/docs/index.html"

echo "Bumped version to ${NEW}"
echo "  Cargo.toml"
echo "  docs/index.html"
echo ""
echo "Run 'cargo check' to verify, then commit."
