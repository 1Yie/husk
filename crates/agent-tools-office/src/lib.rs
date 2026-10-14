//! Office document tools — the `officecli` bridge.
//!
//! Generic over [`agent_tool::WorkspaceCtx`] and [`agent_tool::SandboxCtx`], so
//! this crate names no kernel type. It shares the shell package's
//! `sandbox_cfg` on purpose: an `officecli` run is a process run, and the
//! mount/network policy must be identical to `bash`'s.

pub mod office;
