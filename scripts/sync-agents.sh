#!/usr/bin/env bash
# Sync the agent table from the upstream source of truth.
#
# Upstream: https://github.com/skill-one/agents-info
#
# The upstream agents.jsonl names the skills-dir key `skills_dir` and carries
# extra presentation fields (website / icon / repo / stars). This project keeps
# only the fields it consumes -- name / display / skills_dir / detect, plus the
# local overrides below -- and mirrors the upstream line order, which defines
# the listing order.
#
# Run this manually, review the diff, and commit the regenerated table. It is
# intentionally NOT wired into CI or the build: the committed table is the
# offline artifact the binary embeds at compile time (`include_str!`).
set -euo pipefail

UPSTREAM_URL="https://raw.githubusercontent.com/skill-one/agents-info/main/agents.jsonl"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$REPO_ROOT/src/core/agents.jsonl"

# Local fields the upstream table does not carry, merged per agent name.
# Currently: claude-code links even when its root dir does not exist yet.
LOCAL_OVERRIDES='{"claude-code":{"link_without_root":true}}'

command -v jq >/dev/null 2>&1 || {
  echo "error: jq is required (https://jqlang.github.io/jq/)" >&2
  exit 1
}

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT

echo "Fetching $UPSTREAM_URL"
curl -fsSL "$UPSTREAM_URL" -o "$tmp"

# Keep only the consumed fields (plus local overrides), one compact JSON
# object per line.
jq -c --argjson ov "$LOCAL_OVERRIDES" \
  '{name, display, skills_dir, detect} + ($ov[.name] // {})' "$tmp" > "$OUT"

printf 'Wrote %s agents to %s\n' "$(wc -l < "$OUT" | tr -d ' ')" "src/core/agents.jsonl"
