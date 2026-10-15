//! Product configuration protection follows the host's actual paths and is
//! applied to both command execution and persistent process wrapping.

use std::sync::{Arc, Mutex};

use agent_paths::AppPaths;
use agent_sandbox::{CommandOutput, SandboxBackend, SandboxConfig, SandboxLimits, SandboxTier};
use husk_host::sandbox_prefs::{protected_paths, SwitchableSandbox};

#[derive(Default)]
struct RecordingBackend {
    configs: Mutex<Vec<SandboxConfig>>,
}

#[async_trait::async_trait]
impl SandboxBackend for RecordingBackend {
    fn id(&self) -> &'static str {
        "recording"
    }

    fn tier(&self) -> SandboxTier {
        SandboxTier::L2
    }

    async fn run_command(
        &self,
        _cmd: &str,
        _args: &[&str],
        cfg: &SandboxConfig,
    ) -> anyhow::Result<CommandOutput> {
        self.configs.lock().unwrap().push(cfg.clone());
        Ok(CommandOutput {
            status: 0,
            stdout: String::new(),
            stderr: String::new(),
            is_timeout: false,
            sandbox_denials: Vec::new(),
            peak_memory_mb: None,
            elapsed_ms: 0,
        })
    }

    fn wrap_spawn(
        &self,
        cfg: &SandboxConfig,
        _cmd: &str,
        _args: &[&str],
    ) -> Option<tokio::process::Command> {
        self.configs.lock().unwrap().push(cfg.clone());
        None
    }
}

#[tokio::test]
async fn host_paths_are_protected_for_commands_and_persistent_spawns() {
    let dir = tempfile::tempdir().unwrap();
    let paths = AppPaths::new(dir.path().join("private-config"), dir.path(), ".host");
    let protected = protected_paths(&paths);
    assert!(protected.contains(&paths.config_dir));
    if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
        assert!(protected.contains(&home.join(".config/husk")));
        assert!(protected.contains(&home.join(".config/agent-rs")));
    }

    let backend = Arc::new(RecordingBackend::default());
    let sandbox = SwitchableSandbox::with_limits(backend.clone(), || SandboxLimits {
        enabled: Some(true),
        ..Default::default()
    })
    .with_deny_read(protected.clone());
    let user_private = dir.path().join("user-private");
    let cfg = SandboxConfig {
        deny_read: vec![user_private.clone()],
        ..Default::default()
    };
    sandbox.run_command("check", &[], &cfg).await.unwrap();
    sandbox.wrap_spawn(&cfg, "server", &[]);

    let configs = backend.configs.lock().unwrap();
    assert_eq!(configs.len(), 2);
    for config in configs.iter() {
        assert!(config.deny_read.contains(&user_private));
        for path in &protected {
            assert!(config.deny_read.contains(path), "missing {}", path.display());
        }
    }
    assert_eq!(cfg.deny_read, vec![user_private]);
}
