# AGENTS.md

Documentation follows the `docs-engineering` skill; `docs/README.md` maps where each kind of document lives.

## Build resources

- Rust builds and tests may use at most 20% of the system. `.cargo/config.toml`
  pins `jobs = 4` and `RUST_TEST_THREADS = 4` (20 cores). Do not override them
  with `-j`, `--jobs`, `CARGO_BUILD_JOBS` or `--test-threads` above 4.
- Run long Rust builds niced: `nice -n 10 cargo ...`.
