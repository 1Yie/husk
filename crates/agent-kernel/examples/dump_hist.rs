fn main() {
    let db = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/probe_sessions.db".into());
    let dbp = std::path::Path::new(&db);
    let stores = agent_kernel::session_store::SessionStore::all_workspaces_at(dbp);
    // Newest session across every workspace, then dump its tail.
    let mut metas: Vec<_> = stores
        .iter()
        .flat_map(|s| s.list().into_iter().map(move |m| (s, m)))
        .collect();
    metas.sort_by_key(|(_, m)| std::cmp::Reverse(m.updated_at));
    for (store, meta) in metas.iter().take(4) {
        println!(
            "=== ws={} session={} title={:?} updated={}",
            store.dir_name().unwrap_or_default(),
            meta.id,
            meta.title,
            meta.updated_at
        );
        let Some(hist) = store.load_history(meta.id) else {
            println!("  (no history)");
            continue;
        };
        let start = hist.len().saturating_sub(10);
        for (i, m) in hist.iter().enumerate().skip(start) {
            let content = m.content.as_deref().unwrap_or("∅");
            let short: String = content.chars().take(200).collect();
            let tools = m.tool_calls.as_ref().map(|tc| {
                tc.iter()
                    .map(|t| format!("{}({})", t.name, t.arguments))
                    .collect::<Vec<_>>()
                    .join(",")
            });
            let notice = format!("{:?}", m.notice);
            println!(
                "  [{i}] {:?} tools={:?} {} {:?}",
                m.role,
                tools,
                if notice == "None" {
                    "".to_string()
                } else {
                    notice
                },
                short
            );
        }
    }
}
