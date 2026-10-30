pub mod logging;
pub mod notifications;

use std::path::Path;

pub fn app_paths() -> agent_paths::AppPaths {
    let paths = agent_paths::AppPaths::from_app_name("husk", ".husk")
        .unwrap_or_else(|| agent_paths::AppPaths::new("", "", ".husk"));
    if let Some(base) = dirs::config_dir() {
        migrate_directory(&base.join("agent-rs"), &paths.config_dir);
    }
    if let Some(base) = dirs::data_dir() {
        migrate_directory(&base.join("agent-rs"), &paths.data_dir);
    }
    paths
}

fn migrate_directory(old: &Path, destination: &Path) {
    if !destination.exists()
        && old.is_dir()
        && let Err(error) = std::fs::rename(old, destination)
    {
        tracing::warn!(%error, "Could not move application data");
    }
}
