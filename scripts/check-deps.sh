#!/usr/bin/env bash
# Product dependency law. The kernel (korei) and the extensions (korei-ext) live
# in sibling repositories and police themselves; this table covers only the
# crates that stay here, and allows them to name those siblings.
#
# Workspace dependency law — fails when a crate depends on an internal crate it
# is not allowed to. Only normal/build edges count; dev-dependencies are free.
#
# The table is a ratchet: it records the edges that exist today. Each refactor
# phase removes edges, never adds them. A crate missing from the table fails,
# so a new crate has to state its allowed dependencies on day one.
set -euo pipefail

cd "$(dirname "$0")/.."

declare -A ALLOW=(
  # provider implementations (rig, HTTP, config, credentials) over the contract
  [agent-llm]="agent-llm-core"
  # knowledge and storage — no kernel, no products
  [agent-context]=""
  # the conversation repository — names no product and no kernel type
  [agent-store]="agent-llm-core"
  # the coding product's semantics — depends on the kernel, never the reverse
  [agent-coding]="agent-capabilities agent-context agent-ipc agent-kernel agent-llm-core agent-skills agent-tools-fs"
  # host layer over the kernel — session assembly, commands, modes, sandbox prefs
  [husk-host]="agent-capabilities agent-coding agent-skills agent-store agent-context agent-ipc agent-kernel agent-llm agent-llm-core agent-paths agent-plugin agent-sandbox agent-tools-computer agent-tools-fs agent-tools-office agent-tools-serena agent-tools-shell agent-tools-web"
  [husk-terminal]=""
  # products / frontends — nothing may depend on these
  [app-cli]="agent-ipc agent-kernel agent-llm agent-paths agent-store husk-host"
  [app-tui]="agent-ipc agent-kernel agent-llm agent-paths agent-sandbox agent-store husk-host"
  [app-tauri]="agent-ipc agent-kernel agent-llm agent-paths agent-sandbox agent-store husk-host agent-plugin agent-skills agent-context agent-llm-core"
  [app-desktop]="agent-ipc agent-kernel agent-llm agent-paths agent-sandbox agent-store husk-host"
  [husk]="agent-context agent-ipc agent-kernel agent-llm agent-paths agent-sandbox agent-skills agent-store husk-host husk-terminal"
  [husk-gpui]="agent-context agent-ipc agent-kernel agent-llm agent-paths agent-store husk-host husk-terminal"
)

# Crates nobody may depend on (frontends).
LEAF_PRODUCTS=" app-cli app-tui husk husk-gpui "

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
         | select(.kind != "dev")
         | select(.name as $dep
             | ($dep | test("^(agent-|husk-)")) or ($ws | index($dep) != null))
         | .name ] | unique | join(" ")) ]
  | @tsv' <<<"$meta")

if [[ $fail -eq 0 ]]; then
  echo "dependency law: ok"
fi
exit $fail
