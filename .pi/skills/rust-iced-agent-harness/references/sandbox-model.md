# Execution Sandbox Spec

Design goal: **every command the agent runs executes inside an enforced boundary, and the boundary costs milliseconds, not seconds.** Layered isolation — pick the lightest tier that contains the risk. A missing sandbox backend degrades loudly, never silently.

## Layered defense model

| Tier | Workload | Mechanism | Cold start | Overhead |
|------|----------|-----------|-----------|----------|
| **L1** memory/logic isolation | plugin scripts, WASM tools, parsers | Wasmtime (already spec'd in `plugin-system.md`) | <1 ms | KBs |
| **L2** native process sandbox — **the core tier** | `bash` tool: `cargo check`, `npm test`, builds, scripts | Linux: bubblewrap / Landlock LSM · macOS: `sandbox-exec` profile · Windows: Job Objects + restricted tokens | 5–20 ms | ≈ bare process |
| **L3** microVM — exceptional only | untrusted repo builds, kernel-adjacent work, explicitly flagged danger | Firecracker/QEMU microVM | 100–300 ms | 50–100 MB |

v1 ships **L1 + L2**. L3 is a `SandboxBackend` impl behind the same trait — add later without touching callers.

## L2 — four control dimensions

1. **Filesystem isolation**: system dirs (`/usr`, `/lib`, `/bin`, `C:\Windows`) read-only; sensitive dirs (`~/.ssh`, `~/.gnupg`, `~/.aws`, `~/.bash_history`, browser profiles) denied outright; **only `$WORKSPACE` + per-run tmp writable**.

   **Toolchain cache rule** (or every build dies with PermissionDenied): mounts are ordered — read-only toolchain caches first, then workspace. `CARGO_HOME`/`GOPATH`/`npm` cache dirs mount **read-only** (`~/.cargo/registry`, `~/.cache`); `CARGO_TARGET_DIR` is force-set to `$WORKSPACE/target` (stays writable); `TMPDIR` → the per-run dir. Package-manager writes (`cargo add`, `npm install`) need `allow_network` + the Elevated audit tier anyway.

   **Mount-order invariant**: per-run tmp goes under `/run/user/$UID/agent-run-*` (tmpfs-scoped, auto-cleaned on logout) — never under a path a later mount will shadow. If it must live in `/tmp`, bind it explicitly and never declare a blanket `--tmpfs /tmp` over it.
2. **Network control**: default per `SandboxConfig.allow_network`; offline runs unshare the net namespace (Linux) / deny `network*` (macOS). When on, document that package fetches work; future: proxy-locked egress.
3. **Resource limits**: max memory (default 2048 MB), max wall timeout (per-call `timeout_secs`, bash tool hard cap 600 s), process/thread count cap where the platform allows. Timeout kills the **process tree** (pgid kill / Job Object terminate), not just the leader — no orphaned grandchildren. Every spawn registers in the kernel's `ChildRegistry` with parent-death wiring (`prctl(PR_SET_PDEATHSIG)` Linux / `setpgid`+group-kill macOS / `KILL_ON_JOB_CLOSE` Windows) — `kill -9` on the host leaves zero survivors. See production-hardening §1.
4. **Env sanitization**: never inherit host env wholesale. **All matching is case-insensitive (uppercase-compare)** — Windows env is case-insensitive. Allowlist: `PATH LANG LC_* HOME TERM TMPDIR USER SHELL` + toolchain vars (`CARGO_HOME`, `GOPATH`, `NODE_ENV`…) explicitly configured. Denylist patterns (matched on uppercased name AND value): `*_KEY`, `*_TOKEN`, `*_SECRET`, `*_PASSWORD`, `*_URL`, `*_URI`, `*_DSN`, `*PRIVATE*`, `AWS_*`, `GITHUB_*`, `OPENAI_*`, `ANTHROPIC_*`, `DATABASE_URL`, `MONGODB_URI`, `REDIS_AUTH`, `SENTRY_DSN`, `COOKIE` — denylist wins over allowlist.

## Unified backend trait (`crates/agent-sandbox/src/traits.rs`)

```rust
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    pub workspace_dir: PathBuf,
    pub allow_network: bool,
    pub max_memory_mb: u64,      // default 2048
    pub timeout_secs: u64,       // default 60; caller may raise ≤ 600
    pub env_vars: Vec<(String, String)>,  // post-sanitization additions
    pub snapshot: SnapshotMode,  // Off | Cow | Required
}

#[derive(Debug)]
pub struct CommandOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
    pub is_timeout: bool,
    pub peak_memory_mb: Option<u64>,
    pub elapsed_ms: u64,
}

#[async_trait]
pub trait SandboxBackend: Send + Sync {
    fn id(&self) -> &'static str;                 // "bwrap" | "landlock" | "sandbox-exec" | "job-object" | "none"
    fn tier(&self) -> SandboxTier;                // L1 | L2 | L3
    async fn run_command(&self, cmd: &str, args: &[&str], cfg: &SandboxConfig)
        -> anyhow::Result<CommandOutput>;
}
```

## Platform backends

### Linux — `linux_bwrap.rs` (primary), `linux_landlock.rs` (zero-dep fallback)

Bubblewrap assembly (the proven argument set):

```text
bwrap
  --ro-bind /usr /usr --ro-bind /lib /lib --ro-bind /bin /bin
  --ro-bind /etc/resolv.conf /etc/resolv.conf   # only when allow_network
  --proc /proc --dev /dev
  --ro-bind $CARGO_HOME/registry $CARGO_HOME/registry   # toolchain cache: read-only
  --ro-bind $GOPATH $GOPATH                             #   (if present)
  --bind  $RUN_DIR $RUN_DIR                     # per-run tmp: /run/user/$UID/agent-run-*
  --bind  $EFFECTIVE_WS $WORKSPACE              # rw: real ws, or CoW snapshot bind-mounted AT $WORKSPACE
  [--unshare-net]                               # when !allow_network
  --clearenv --setenv K V ...
  --setenv CARGO_TARGET_DIR $WORKSPACE/target
  --setenv TMPDIR $RUN_DIR
  --chdir $WORKSPACE
  --die-with-parent                             # no orphans if host dies
  -- sh -c "<cmd>"
```

**Mount-order rules** (ordering is the contract — later binds shadow earlier):
- `$EFFECTIVE_WS` = real workspace when `snapshot: Off`; the CoW snapshot dir when `Cow|Required`. The snapshot always bind-mounts **at the canonical `$WORKSPACE` path inside the sandbox** — commands see a stable path regardless of which side they run against.
- Per-run tmp lives at `/run/user/$UID/agent-run-*`, bound as itself. Do NOT `--tmpfs /tmp` — a blanket tmpfs would shadow any `/tmp`-resident snapshot. If a snapshot must be under `/tmp` (portable fallback), bind it explicitly instead of declaring tmpfs.
- Toolchain caches mount **before** the workspace bind so a hostile repo can't shadow them via a nested path.

`landlock` crate (kernel ≥5.13) = no external binary: restrict fs access to workspace+tmp rw, everything else read/deny. Use when `bwrap` not on `PATH`.

### macOS — `macos_seatbelt.rs`

`sandbox-exec -p <profile>` with generated SBPL:

```scheme
(version 1) (deny default)
(allow file-read* (subpath "/usr") (subpath "/System") (subpath "/Library") (subpath "/bin"))
(allow file-read* file-write* (subpath "$WORKSPACE") (subpath "/tmp"))
(allow process-exec process-fork)
(deny file-read* (subpath "$HOME/.ssh") (subpath "$HOME/.gnupg") (subpath "$HOME/.aws"))
(allow network*)   ; only when allow_network
```

`sandbox-exec` is deprecated-but-present on every macOS; App Sandbox entitlements are the long-term path for a distributed binary.

### Windows — `windows_job.rs`

Job Object with `JOB_OBJECT_LIMIT_PROCESS_MEMORY`, `KillOnJobClose`, active-process limit; restricted token strips privileges. FS isolation is weaker than Unix — compensate: env sanitization + audit gate are stricter, and writable scope is enforced at the tool layer (paths outside workspace rejected pre-spawn).

### Fallback chain & degradation

`detect_backend()` at startup: Linux → `bwrap` → `landlock` → `none`; macOS → `sandbox-exec` → `none`; Windows → `job-object` → `none`.

Running on `none` is **allowed but loud**: config must set `sandbox.allow_unsandboxed = true`, UI shows a persistent warn chip ("UNSANDBOXED"), and every `bash` call requires confirmation regardless of permission mode.

## CoW snapshots (`crates/agent-sandbox/src/cow.rs`)

"Bold agent, instant regret medicine": clone workspace → let agent mutate → merge back or discard.

```text
real workspace ──reflink clone──▶ /tmp/agent-run-XXXX/  (<20 ms for 10k files)
     ▲                                   │
     └── merge diff on success ◀── agent edits + builds + tests here
                                     └── on failure/abort: rm -rf, workspace untouched
```

- `SnapshotMode::Cow` via `reflink-copy` (APFS/Btrfs/XFS). Fallback chain: reflink → hardlink → full copy → run-in-place (with a UI note). Never block the turn on a slow copy — cap snapshot setup at 2 s, then degrade.
- Merge-back: `similar`-diff snapshot vs. real → funnel through the **same** `AwaitingToolConfirmation` diff review → apply → record hunks in HunkTracker (`origin: "sandbox-merge"`). **v1 keeps merge-back file-level**: whole-file replace + unified-diff display. Line-level hunk conflict resolution is a later iteration — don't build the merge algorithm on day one.
- `SnapshotMode::Required` (config per project): every `bash`/patch turn runs against a snapshot; `Cow` = opt-in per command or per turn; `Off` = direct writes.

## Implementation phasing (for AI-assisted builds)

Ship backends incrementally — cross-platform `#[cfg]` branches written in one pass produce compile errors, not working sandboxes:

1. **Phase 1**: `traits.rs` + `linux_bwrap.rs` + `none.rs` only. Prove the trait contract, mount-order invariants, env sanitization, and unit tests on one backend.
2. **Phase 2**: `linux_landlock.rs` + `macos_sbpl.rs` (the two next-cheapest).
3. **Phase 3**: `win_job.rs` (different process model — deserves its own pass).
4. **Merge-back**: file-level replace v1 → hunk-level conflict merge v2.

## Pre-execution audit (`crates/agent-sandbox/src/audit.rs`)

Before any `bash` dispatch, pattern-match the command. Result feeds `AwaitingToolConfirmation.risk` and the sandbox config.

| Match (regex/pattern) | Level | Effect |
|----------------------|-------|--------|
| `rm -rf`, `rm -fr`, `mkfs`, `dd of=/dev`, `> /dev/sd`, fork-bomb `:(){…}` | **Critical** | red card; runs only after explicit approve; forced `snapshot: Required` |
| `git push --force`, `git reset --hard`, `sudo`, `chmod -R 777`, `curl/wget … \| sh`/`bash` | **Elevated** | confirmation required in every mode incl. `acceptEdits` |
| `npm install`, `cargo add`, `brew install`, `pip install` | **Network-mutating** | confirmation + forces `allow_network` prompt |
| paths outside `$WORKSPACE`, `$HOME` writes | **Scope-violation** | denied unless user approves sandbox widening for that call |
| everything else | **Normal** | sandbox defaults; confirm per permission mode |

Audit is **pre-spawn string analysis** — it complements, never replaces, the OS sandbox. A critical command the user approves still runs sandboxed (with CoW), never on the raw host.

## Who runs sandboxed

- `bash` tool: **always** L2 (or loud `none`).
- MCP plugin servers: optional per manifest (`sandboxed: true`) — they are child processes too; run under the same backend when feasible.
- WASM plugins: already L1 by construction.
- `fuzzy_patch`/`apply_patch`/`smart_read`: in-process, governed by tool-layer path checks, not process sandbox. **Path check = `std::fs::canonicalize` first, then workspace-prefix compare** — a `ln -s ~/.ssh/id_rsa $WORKSPACE/x` inside the workspace must resolve to its real path and be rejected; string-prefix checks alone are a symlink-escape hole.

## UI contract hooks (see iced-ui-contract.md)

- `PendingApprovalData.risk`: `"normal" | "elevated" | "critical" | "network"` + `audit_reason` shows the matched pattern. Critical renders the red banner variant.
- `SandboxTelemetry` per running step: `elapsed_ms`, `peak_memory_mb`, `sandboxed` (backend id or "none"), `snapshot` mode — live-updating card while `ExecutingTool`.
- Persistent warn chip when backend is `none` or a snapshot merge is pending.

## Non-negotiables

- Env sanitization applies to **every** spawn, including sandboxed ones — sandbox contains damage, sanitization prevents leakage.
- Timeout always kills the whole process group/tree.
- Audit denial and sandbox widening decisions are logged per-session (audit trail viewable in workspace panel).
- `none` backend never runs silently; `sandbox.allow_unsandboxed` is opt-in only.
- Mount ordering is load-bearing: ro toolchain caches → per-run tmp → workspace/snapshot bind last; a blanket `--tmpfs /tmp` that could shadow a snapshot is forbidden.
- In-process tools canonicalize every path before prefix checks — symlinks are not a sandbox boundary.

## Acceptance checklist

- [ ] Sandboxed `cat ~/.ssh/id_rsa` fails on Linux and macOS
- [ ] `env` inside sandbox contains no `*_KEY`/`_TOKEN`/`_SECRET`
- [ ] `--unshare-net` / `(deny network*)` verified: `curl` to localhost fails when `allow_network: false`
- [ ] 60 s timeout on `while true; do :; done` kills entire tree, no orphan procs (`pgrep` clean)
- [ ] `kill -9` on host while sandboxed cmd + MCP server run → zero survivors in 1 s
- [ ] Reflink snapshot of 5k-file workspace < 50 ms; merge-back diff appears in approval UI
- [ ] `rm -rf /` audit → critical card; even after approve it runs inside CoW snapshot
- [ ] Backend `none` → warn chip visible + every bash call confirms
- [ ] Sandbox spawn overhead ≤ 20 ms measured (bwrap/sandbox-exec warm)
- [ ] `cargo check` inside sandbox succeeds (toolchain cache ro-mounted, `CARGO_TARGET_DIR` writable)
- [ ] CoW run: snapshot at `/run/user/$UID/...` survives mount ordering — command sees it at `$WORKSPACE`
- [ ] `ln -s ~/.ssh/id_rsa ws/x` → `smart_read ws/x` rejected after canonicalize
- [ ] `export database_url=postgres://u:p@h/db` → stripped (case-insensitive denylist)
