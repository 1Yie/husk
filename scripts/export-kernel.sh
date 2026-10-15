#!/usr/bin/env bash
# Export the kernel closure as a standalone cargo workspace.
#   scripts/export-kernel.sh <dest-dir>
# The result builds and tests with nothing from this repository: it is the
# proof that the closure is self-contained, and the starting tree for the
# kernel's own repository. Workspace-inherited dependencies are copied over.
set -euo pipefail

cd "$(dirname "$0")/.."
dest=${1:?usage: export-kernel.sh <dest-dir>}
CRATES="agent-ipc agent-llm-core agent-tool agent-kernel agent-testkit"

rm -rf "$dest"
mkdir -p "$dest/crates"
for c in $CRATES; do
  cp -r "crates/$c" "$dest/crates/$c"
  rm -rf "$dest/crates/$c/target"
done
cp LICENSE "$dest/" 2>/dev/null || true
# The script lives in scripts/ here and at the repo root there.
sed 's#cd "$(dirname "$0")/.."#cd "$(dirname "$0")"#' scripts/check-kernel-deps.sh > "$dest/check-kernel-deps.sh"
chmod +x "$dest/check-kernel-deps.sh"

python3 - "$dest" <<'PY'
import re, sys
dest = sys.argv[1]
src = open("Cargo.toml").read()
# [workspace.dependencies] section only, up to the next top-level table.
m = re.search(r"\[workspace\.dependencies\]\n(.*?)(?=\n\[|\Z)", src, re.S)
deps = m.group(1)
needed = set()
import glob
for f in glob.glob(f"{dest}/crates/*/Cargo.toml"):
    for line in open(f):
        mm = re.match(r"^([A-Za-z0-9_-]+)\.workspace\s*=\s*true", line) or \
             re.match(r"^\[dependencies\.([A-Za-z0-9_-]+)\]", line)
        if mm: needed.add(mm.group(1))
keep = []
for line in deps.splitlines():
    mm = re.match(r"^([A-Za-z0-9_-]+)\s*=", line)
    if mm and mm.group(1) in needed:
        keep.append(line)
open(f"{dest}/Cargo.toml", "w").write(
    '[workspace]\nresolver = "2"\nmembers = ["crates/*"]\n\n[workspace.dependencies]\n'
    + "\n".join(keep) + "\n")
PY
cp .cargo -r "$dest/.cargo" 2>/dev/null || true
echo "exported to $dest"
