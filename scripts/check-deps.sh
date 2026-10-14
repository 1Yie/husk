#!/usr/bin/env bash
# Workspace dependency law — fails when a crate depends on an internal crate it
# is not allowed to. Only normal/build edges count; dev-dependencies are free.
#
# The table is a ratchet: it records the edges that exist today. Each refactor
# phase removes edges, never adds them. A crate missing from the table fails,
# so a new crate has to state its allowed dependencies on day one.
set -euo pipefail

cd "$(dirname "$0")/.."

declare -A ALLOW=(
  # leaves — know nothing about the rest of the workspace
  [agent-ipc]=""
  [agent-llm]=""
  [agent-sandbox]=""
  [agent-context]=""
  [agent-computer]=""
  # extension ecosystem
  [agent-plugin]="agent-llm agent-paths agent-sandbox"
  # the tool contract — a leaf that knows no product, only the wire types
  [agent-tool]="agent-llm"
  # skills are knowledge, not capability — no kernel, no products
  [agent-skills]=""
  # where the app keeps its files — pure path resolution, names no product
  [agent-paths]=""
  # runtime
  [agent-kernel]="agent-computer agent-context agent-ipc agent-llm agent-plugin agent-paths agent-sandbox agent-skills agent-tool"
  # products / frontends — nothing may depend on these
  [app-cli]="agent-ipc agent-kernel agent-llm"
  [app-tui]="agent-ipc agent-kernel agent-llm agent-sandbox"
  [husk]="agent-context agent-ipc agent-kernel agent-llm agent-paths agent-sandbox"
)

# Crates nobody may depend on (frontends).
LEAF_PRODUCTS=" app-cli app-tui husk "

meta=$(cargo metadata --no-deps --format-version 1)
fail=0

while IFS=$'\t' read -r name deps; do
  if [[ ! -v "ALLOW[$name]" ]]; then
    echo "FAIL  $name: not in the allowlist in scripts/check-deps.sh — declare its allowed workspace deps"
    fail=1
    continue
  fi
  for dep in $deps; do
    if [[ " ${ALLOW[$name]} " != *" $dep "* ]]; then
      echo "FAIL  $name -> $dep is not allowed (allowed: ${ALLOW[$name]:-none})"
      fail=1
    fi
    if [[ "$LEAF_PRODUCTS" == *" $dep "* ]]; then
      echo "FAIL  $name -> $dep: a frontend/product crate cannot be depended on"
      fail=1
    fi
  done
done < <(jq -r '
  ([.packages[].name] | unique) as $ws
  | .packages[]
  | [ .name,
      ([ .dependencies[]
         | select(.kind != "dev" and (.name as $n | $ws | index($n)))
         | .name ] | unique | join(" ")) ]
  | @tsv' <<<"$meta")

if [[ $fail -eq 0 ]]; then
  echo "dependency law: ok"
fi
exit $fail
