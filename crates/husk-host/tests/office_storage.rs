//! The office namespace is host policy, but existing sessions must keep the
//! same storage key through the extraction, including symlink spellings.

use agent_llm::ChatMessage;
use agent_store::SessionStore;
use husk_host::office::{init_store, is_office_root, office_root};

#[test]
fn office_sessions_keep_their_namespace_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    init_store(dir.path().to_path_buf());
    let office = office_root().unwrap();
    std::fs::create_dir_all(&office).unwrap();

    let store = SessionStore::open(&office).unwrap();
    assert_eq!(store.dir_name().as_deref(), Some("office"));
    let id = store.next_id();
    store
        .snapshot(id, &[ChatMessage::user("existing office conversation")])
        .unwrap();
    store.set_last_active(id).unwrap();
    drop(store);

    let reopened = SessionStore::open_existing(&office).unwrap();
    assert_eq!(reopened.dir_name().as_deref(), Some("office"));
    assert_eq!(reopened.last_active(), Some(id));
    assert_eq!(
        reopened.load_history(id).unwrap()[0].content.as_deref(),
        Some("existing office conversation")
    );

    #[cfg(unix)]
    {
        let alias = dir.path().join("office-link");
        std::os::unix::fs::symlink(&office, &alias).unwrap();
        assert!(is_office_root(&alias));
        let reopened = SessionStore::open_existing(&alias).unwrap();
        assert_eq!(reopened.dir_name().as_deref(), Some("office"));
        assert!(reopened.has_history(id).unwrap());
    }

    // A regular project named "office" is still an ordinary workspace.
    let project = dir.path().join("office");
    std::fs::create_dir(&project).unwrap();
    assert!(!is_office_root(&project));
    let ordinary = SessionStore::open(&project).unwrap();
    assert_ne!(ordinary.dir_name().as_deref(), Some("office"));
    assert!(ordinary.load_history(id).is_none());
}
