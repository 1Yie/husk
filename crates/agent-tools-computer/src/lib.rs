//! Computer-use tools: drive the screen, and capture it.
//!
//! Generic over [`agent_tool::WorkspaceCtx`] and [`agent_tool::DesktopCtx`], so
//! this crate names no kernel type. The backend is a host-installed capability:
//! a headless host installs none and both tools refuse with a reason.

pub mod computer;
pub mod screenshot;

/// A context for this crate's own tests — the tools need a desktop and a root,
/// nothing else. The kernel's `ToolCtx` is the production implementation.
#[cfg(test)]
pub(crate) struct TestCtx {
    root: std::path::PathBuf,
    desktop: std::sync::Arc<dyn agent_computer::DesktopBackend>,
    /// Set by the cancellation test, which needs a flag it can flip mid-call.
    pub cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

#[cfg(test)]
impl TestCtx {
    pub(crate) fn with_desktop(
        root: impl AsRef<std::path::Path>,
        desktop: std::sync::Arc<dyn agent_computer::DesktopBackend>,
    ) -> Self {
        let root = root.as_ref();
        Self {
            root: root.canonicalize().unwrap_or_else(|_| root.to_path_buf()),
            desktop,
            cancel: None,
        }
    }
}

#[cfg(test)]
impl agent_tool::WorkspaceCtx for TestCtx {
    fn root(&self) -> &std::path::Path {
        &self.root
    }

    fn cancel_flag(&self) -> Option<std::sync::Arc<std::sync::atomic::AtomicBool>> {
        self.cancel.clone()
    }

    fn resolve(&self, path: &str) -> Result<std::path::PathBuf, agent_tool::ToolError> {
        let joined = self.root.join(path);
        let canon = joined.canonicalize().map_err(agent_tool::ToolError::Io)?;
        if !canon.starts_with(&self.root) {
            return Err(agent_tool::ToolError::PathEscape(path.to_string()));
        }
        Ok(canon)
    }
}

#[cfg(test)]
impl agent_tool::DesktopCtx for TestCtx {
    fn desktop(
        &self,
    ) -> Result<std::sync::Arc<dyn agent_computer::DesktopBackend>, agent_tool::ToolError> {
        Ok(self.desktop.clone())
    }
}
