#!/usr/bin/env bash
# Fails when a user-visible "Zed" string reaches a crate that a running
# KnightCode can compile in.
#
# The point is not the strings that are here today — Task 6 of WP06 swept
# those — but the ones an upstream merge adds tomorrow. Every hit has to be
# either fixed or written into script/branding-allowlist.txt with a reason, so a
# merge that introduces a new user-visible Zed string becomes visible instead of
# silent.
#
#   script/check-branding.sh            # fail on anything not allowlisted
#   script/check-branding.sh --list     # print every current hit, allowlisted or not
#
# "Prose" is a string literal containing the word Zed followed by a space. That
# is a deliberately blunt rule: it catches sentences and misses identifiers,
# paths and settings values, which is the right trade for a gate that has to
# survive merges without anyone tuning it.

set -euo pipefail

cd "$(dirname "$0")/.."

ALLOWLIST="script/branding-allowlist.txt"

# Crates a running IDE cannot reach, plus the test and preview code inside the
# ones it can. Collab, the cloud API clients and the eval/benchmark tools are
# excluded wholesale: none of them is compiled into a surface KnightCode shows.
UNREACHABLE_CRATES='^crates/(collab|collab_ui|call|channel|livekit_[a-z]+|zeta_prompt|cloud_llm_client|cloud_api_types|cloud_api_client|git_hosting_providers|docs_preprocessor|edit_prediction_cli|eval_cli|eval_utils|theme_importer|schema_generator|remote_server|benchmarks|extension_cli)/'
NOT_A_SURFACE='/(tests|fixtures|benchmarks|benches|examples)/|_tests?\.rs:|/stories/|component_preview|/eval|test_support'

hits() {
  grep -rn --include=*.rs -oE '"[^"]*Zed [^"]*"' crates/ \
    | grep -vE "$NOT_A_SURFACE" \
    | grep -vE "$UNREACHABLE_CRATES" \
    | sed -E 's/^([^:]+):[0-9]+:/\1|/' \
    | sort -u
}

if [[ "${1:-}" == "--list" ]]; then
  hits
  exit 0
fi

# Line numbers move on every merge, so an entry is keyed on the file and the
# string itself. Comments and blank lines are ignored.
allowed=$(grep -vE '^\s*(#|$)' "$ALLOWLIST" || true)

unexpected=$(comm -23 <(hits) <(printf '%s\n' "$allowed" | sort -u))

if [[ -n "$unexpected" ]]; then
  echo "New user-visible Zed strings reached a live crate:"
  echo
  printf '%s\n' "$unexpected" | sed 's/^/  /'
  echo
  echo "Rewrite each one, or add it to $ALLOWLIST with a one-line reason above it."
  exit 1
fi

# An allowlist entry that no longer matches anything is dead weight, and a
# reviewer reading it would be reading a reason for a string that is gone.
stale=$(comm -13 <(hits) <(printf '%s\n' "$allowed" | sort -u))
if [[ -n "$stale" ]]; then
  echo "Stale entries in $ALLOWLIST — the strings are gone, remove the lines:"
  echo
  printf '%s\n' "$stale" | sed 's/^/  /'
  exit 1
fi

echo "check-branding: $(hits | wc -l) hits, all allowlisted"
