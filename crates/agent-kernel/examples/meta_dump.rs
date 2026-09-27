fn main() {
    let db = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/probe2.db".into());
    let stores =
        agent_kernel::session_store::SessionStore::all_workspaces_at(std::path::Path::new(&db));
    for s in &stores {
        for m in s.list() {
            if m.id == 112 {
                println!("session=112 updated={} queued={:?} model={:?} provider={:?} perm={:?} agent={:?} think={:?}",
                    m.updated_at, m.queued_prompts, m.model, m.provider, m.permission_mode, m.agent_mode, m.thinking_level);
            }
        }
    }
}
