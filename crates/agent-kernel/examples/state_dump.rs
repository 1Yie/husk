fn main() {
    let db = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/probe2.db".into());
    let stores =
        agent_kernel::session_store::SessionStore::all_workspaces_at(std::path::Path::new(&db));
    for s in &stores {
        for m in s.list() {
            if m.id == 112 {
                for name in [
                    "turn.json",
                    "state.json",
                    "engine.json",
                    "run.json",
                    "session.json",
                ] {
                    if let Ok(Some(b)) = s.read_state(m.id, name) {
                        println!("=== state[{name}] {} bytes", b.len());
                        let txt = String::from_utf8_lossy(&b);
                        println!("{}", txt.chars().take(500).collect::<String>());
                    }
                }
            }
        }
    }
}
