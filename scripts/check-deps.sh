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
  # optional host adapters — never a kernel dependency
  [agent-capabilities]="agent-computer agent-paths agent-sandbox agent-tool"
  # skills are knowledge, not capability — no kernel, no products
  [agent-skills]=""
  # where the app keeps its files — pure path resolution, names no product
  [agent-paths]=""
  # the conversation repository — names no product and no kernel type
  [agent-store]="agent-llm"
  # workspace file tools — generic over the context, so nothing of the kernel
  [agent-tools-fs]="agent-tool"
  # networking tools — same pattern
  [agent-tools-web]="agent-capabilities agent-llm agent-tool"
  # process tools — these also need the sandbox seam
  [agent-tools-shell]="agent-capabilities agent-sandbox agent-tool"
  # screen tools — the desktop seam is optional by design
  [agent-tools-computer]="agent-capabilities agent-computer agent-llm agent-paths agent-tool"
  # document tools — share the shell package's sandbox derivation
  [agent-tools-office]="agent-capabilities agent-sandbox agent-tool agent-tools-shell"
  # serena MCP bridge — spawns a stdio child, generic over the context
  [agent-tools-serena]="agent-tool"
  # the coding product's semantics — depends on the kernel, never the reverse
  [agent-coding]="agent-context agent-ipc agent-kernel agent-llm agent-skills agent-tools-fs"
  # host layer over the kernel — session assembly, commands, modes, sandbox prefs
  [husk-host]="agent-capabilities agent-coding agent-skills agent-store agent-context agent-ipc agent-kernel agent-llm agent-paths agent-plugin agent-sandbox agent-tools-computer agent-tools-fs agent-tools-office agent-tools-serena agent-tools-shell agent-tools-web"
  # runtime
  [agent-kernel]="agent-ipc agent-llm agent-plugin agent-tool"
  # products / frontends — nothing may depend on these
  [app-cli]="agent-ipc agent-kernel agent-llm agent-paths agent-store husk-host"
  [app-tui]="agent-ipc agent-kernel agent-llm agent-paths agent-sandbox agent-store husk-host"
  [husk]="agent-context agent-ipc agent-kernel agent-llm agent-paths agent-sandbox agent-skills agent-store husk-host"
  # the second-product acceptance test (T11): kernel + llm only, by design
  [minimal-agent]="agent-kernel agent-llm"
  # dev-only test support — reached only through [dev-dependencies]
  [agent-testkit]="agent-llm"
)

# Crates nobody may depend on (frontends).
LEAF_PRODUCTS=" app-cli app-tui husk minimal-agent agent-testkit "

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
