#!/bin/sh
# Validate coordinated Husk/korei changes without rewriting Husk's Git pins or
# lockfile. The temporary copy receives Cargo's path patches, not this checkout.
set -eu

if [ "$#" -lt 1 ]; then
    printf 'Usage: %s /path/to/korei [cargo arguments...]\n' "$0" >&2
    exit 2
fi
korei=$(CDPATH= cd "$1" && pwd)
shift
root=$(CDPATH= cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/husk-local-korei.XXXXXX")
trap 'rm -rf "$work"' EXIT HUP INT TERM

python3 - "$root" "$korei" "$work" <<'PY'
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tomllib

root, korei, work = map(Path, sys.argv[1:])
files = subprocess.check_output([
    'git', '-C', str(root), 'ls-files', '--cached', '--others', '--exclude-standard', '-z',
]).split(b'\0')
for name in files:
    if not name:
        continue
    relative = Path(name.decode())
    source, destination = root / relative, work / relative
    if not source.exists() and not source.is_symlink():
        continue  # A tracked deletion is absent from the validation copy too.
    destination.parent.mkdir(parents=True, exist_ok=True)
    if source.is_symlink():
        destination.symlink_to(source.readlink())
    elif source.is_file():
        shutil.copy2(source, destination)

# Use the temporary workspace's auto-discovered config, not just a --config
# argument: tests spawn cargo metadata subprocesses, which must see it too.
config = work / '.cargo' / 'config.toml'
config.parent.mkdir(parents=True, exist_ok=True)
original = config.read_text() if config.exists() else ''
patches = ['[patch."https://github.com/1Yie/korei.git"]']
for manifest in sorted(korei.glob('crates/*/Cargo.toml')):
    package = tomllib.loads(manifest.read_text())['package']['name']
    patches.append(f'{json.dumps(package)} = {{ path = {json.dumps(str(manifest.parent))} }}')
config.write_text(original + '\n' + '\n'.join(patches) + '\n')
PY

if [ "$#" -eq 0 ]; then
    set -- test --offline -p agent-llm -p husk-host
fi
printf 'Validating temporary Husk workspace against %s\n' "$korei"
printf 'Original Cargo.toml and Cargo.lock are unchanged.\n'
cd "$work"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}"
export CARGO_BUILD_JOBS=4
export RUST_TEST_THREADS=4
nice -n 10 cargo "$@"
