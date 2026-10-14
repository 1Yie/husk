//! Process tools: run a shell command, run a test suite, keep a service up.
//!
//! Generic over [`agent_tool::WorkspaceCtx`] and [`agent_tool::SandboxCtx`], so
//! this crate names no kernel type. `sandbox_cfg` is the shared
//! audit-verdict-to-`SandboxConfig` derivation both `bash` and `test_runner`
//! go through, so a test run is not a lesser command than a shell run.

pub mod bash;
pub mod sandbox_cfg;
pub mod services;
pub mod test_runner;
pub mod util;

/// A context for this crate's own tests.
///
/// The tools are generic over the two traits, so their tests need a context
/// and nothing else — no kernel, no registries. The kernel's `ToolCtx` is the
/// production implementation; this exists so the tests live where the tools do.
#[cfg(test)]
pub(crate) struct TestCtx {
    root: std::path::PathBuf,
    sandbox: std::sync::Arc<dyn agent_sandbox::SandboxBackend>,
}

#[cfg(test)]
impl TestCtx {
    pub(crate) fn with_sandbox(
        root: impl AsRef<std::path::Path>,
        sandbox: std::sync::Arc<dyn agent_sandbox::SandboxBackend>,
    ) -> Self {
        let root = root.as_ref();
        Self {
            root: root.canonicalize().unwrap_or_else(|_| root.to_path_buf()),
            sandbox,
        }
    }
}

#[cfg(test)]
impl agent_tool::WorkspaceCtx for TestCtx {
    fn root(&self) -> &std::path::Path {
        &self.root
    }

    fn cancel_flag(&self) -> Option<std::sync::Arc<std::sync::atomic::AtomicBool>> {
        None
    }

    fn resolve(&self, path: &str) -> Result<std::path::PathBuf, agent_tool::ToolError> {
        let joined = self.root.join(path);
        let canon = joined
            .canonicalize()
            .map_err(agent_tool::ToolError::Io)?;
        if !canon.starts_with(&self.root) {
            return Err(agent_tool::ToolError::PathEscape(path.to_string()));
        }
        Ok(canon)
    }
}

#[cfg(test)]
impl agent_tool::SandboxCtx for TestCtx {
    fn sandbox(
        &self,
    ) -> Result<std::sync::Arc<dyn agent_sandbox::SandboxBackend>, agent_tool::ToolError> {
        Ok(self.sandbox.clone())
    }

    fn sandbox_limits(&self) -> agent_sandbox::SandboxLimits {
        agent_sandbox::SandboxLimits::default()
    }
}
