//! Import existing redb/JSONL sessions into independent session files.
//! Uses the user's real data directory; tests use fixture-specific overrides.

fn main() -> std::io::Result<()> {
    let path = agent_kernel::session_store::migrate_session_database()?;
    let stores = agent_kernel::session_store::SessionStore::all_workspaces();
    let sessions: usize = stores.iter().map(|s| s.list().len()).sum();
    println!("JSONL session storage ready: {}", path.display());
    println!(
        "workspaces={}, sessions={sessions}; original redb/JSONL sources retained",
        stores.len()
    );
    Ok(())
}
