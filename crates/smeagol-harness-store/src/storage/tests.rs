use super::*;

fn record(id: &str, project: &str) -> MemoryRecord {
    MemoryRecord {
        id: id.into(),
        scope: MemoryScope::Project(project.into()),
        title: "Decisão de cache".into(),
        body: "SQLite mantém o cache local.".into(),
    }
}

fn context(workspace: &str, project: &str, identity: &str) -> ProjectContext {
    ProjectContext {
        workspace: workspace.into(),
        project: project.into(),
        repository_identity: Some(identity.into()),
        marker: None,
    }
}

#[test]
fn writes_distinct_markdown_pages_and_indexes_them() {
    let temp = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(temp.path());
    let first = store.write(&record("first", "alpha")).unwrap();
    let second = store.write(&record("second", "alpha")).unwrap();
    assert_ne!(first, second);
    assert!(first.exists() && second.exists());
    let found = store
        .search("cache, decisão!", &MemoryScope::Project("alpha".into()))
        .unwrap();
    assert_eq!(found.len(), 2);
    assert!(
        first
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("_meta.md")
            .exists()
    );
}

#[test]
fn isolates_identically_named_projects_in_different_workspaces() {
    let temp = tempfile::tempdir().unwrap();
    let work = MemoryStore::new(temp.path()).with_workspace("work");
    let personal = MemoryStore::new(temp.path()).with_workspace("personal");
    let page = work.write(&record("cache", "alpha")).unwrap();
    assert!(
        personal
            .search("cache", &MemoryScope::Project("alpha".into()))
            .unwrap()
            .is_empty()
    );
    let other = personal.write(&record("cache", "alpha")).unwrap();
    assert_ne!(page, other);
    assert_ne!(
        work.namespace("alpha").unwrap().workspace_id,
        personal.namespace("alpha").unwrap().workspace_id
    );
}

#[test]
fn search_does_not_leak_results_from_other_projects() {
    let temp = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(temp.path());
    store.write(&record("one", "alpha")).unwrap();
    store.write(&record("two", "beta")).unwrap();
    let found = store
        .search("SQLite", &MemoryScope::Project("alpha".into()))
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, "one");
}

#[test]
fn different_forge_identities_never_share_a_project_namespace() {
    let temp = tempfile::tempdir().unwrap();
    let github = MemoryStore::new(temp.path()).with_context(context(
        "default",
        "team-repo",
        "git:github.com/team/repo",
    ));
    let gitlab = MemoryStore::new(temp.path()).with_context(context(
        "default",
        "team-repo",
        "git:gitlab.com/team/repo",
    ));
    let a = github.namespace("team-repo").unwrap();
    let b = gitlab.namespace("team-repo").unwrap();
    assert_ne!(a.project_id, b.project_id);
    assert_eq!(b.project, "team-repo-2");
    assert_eq!(gitlab.namespace("team-repo").unwrap(), b);
    github.write(&record("note", "team-repo")).unwrap();
    assert!(
        gitlab
            .search("cache", &MemoryScope::Project("team-repo".into()))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn renamed_projects_keep_ids_files_and_repository_bindings() {
    let temp = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(temp.path()).with_context(context(
        "default",
        "old",
        "git:example.org/team/repo",
    ));
    let before = store.namespace("old").unwrap();
    let page = store.write(&record("note", "old")).unwrap();
    store.rename_project("old", "new").unwrap();
    let after = store.namespace("old").unwrap();
    assert_eq!(before.project_id, after.project_id);
    assert_eq!(after.project, "new");
    assert!(page.exists());
    assert_eq!(
        store
            .search("cache", &MemoryScope::Project("new".into()))
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        MemoryStore::new(temp.path())
            .namespace("new")
            .unwrap()
            .project_id,
        before.project_id
    );
}

#[test]
fn repeated_page_writes_replace_the_index_row() {
    let temp = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(temp.path());
    let mut page = record("note", "alpha");
    store.write(&page).unwrap();
    page.body = "PostgreSQL substitui o índice.".into();
    store.write(&page).unwrap();
    assert!(store.search("SQLite", &page.scope).unwrap().is_empty());
    assert_eq!(store.search("PostgreSQL", &page.scope).unwrap().len(), 1);
}

#[test]
fn handoffs_are_consumed_once_and_isolated_by_workspace() {
    let temp = tempfile::tempdir().unwrap();
    let a = MemoryStore::new(temp.path()).with_workspace("a");
    let b = MemoryStore::new(temp.path()).with_workspace("b");
    a.publish_handoff("alpha", "retomar implementação").unwrap();
    assert_eq!(b.claim_handoff("alpha").unwrap(), None);
    assert_eq!(
        a.claim_handoff("alpha").unwrap().as_deref(),
        Some("retomar implementação")
    );
    assert_eq!(a.claim_handoff("alpha").unwrap(), None);
    let path = a.publish_handoff("alpha", &"x".repeat(8_000)).unwrap();
    assert!(fs::read_to_string(path).unwrap().chars().count() < 6_100);
}

#[test]
fn malformed_handoff_bytes_are_decoded_lossily() {
    let temp = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(temp.path());
    let path = store.publish_handoff("alpha", "old").unwrap();
    fs::write(path, [b'a', 0xff, b'b']).unwrap();
    assert_eq!(
        store.claim_handoff("alpha").unwrap().as_deref(),
        Some("a�b")
    );
}

#[test]
fn session_pages_and_spools_use_stable_namespace_coordinates() {
    let temp = tempfile::tempdir().unwrap();
    let store =
        MemoryStore::new(temp.path()).with_context(context("work", "alpha", "marker:alpha"));
    let namespace = store.namespace("alpha").unwrap();
    let path = store.session_spool_path("alpha", "s1").unwrap();
    assert_eq!(
        path,
        temp.path()
            .join("spool/sessions")
            .join(&namespace.workspace_id)
            .join(&namespace.project_id)
            .join("s1.jsonl")
    );
    let mut page = record("session-summary", "alpha");
    page.scope = MemoryScope::Session("s1".into());
    let path = store.write(&page).unwrap();
    assert!(path.ends_with("sessions/s1.md"));
    assert_eq!(store.search("cache", &page.scope).unwrap().len(), 1);
    page.id = "conflicting-summary".into();
    assert!(store.write(&page).is_err());
}

#[test]
fn reindex_recovers_names_ids_pages_and_bindings_after_database_loss() {
    let temp = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(temp.path()).with_context(context(
        "work",
        "alpha",
        "git:example.org/team/repo",
    ));
    let original = store.namespace("alpha").unwrap();
    store.write(&record("first", "alpha")).unwrap();
    store.write(&record("second", "alpha")).unwrap();
    let mut session = record("summary", "alpha");
    session.scope = MemoryScope::Session("s1".into());
    store.write(&session).unwrap();
    fs::remove_file(temp.path().join("db/workspaces.sqlite")).unwrap();
    assert_eq!(store.reindex().unwrap(), 3);
    assert_eq!(store.namespace("alpha").unwrap(), original);
    assert_eq!(
        store
            .search("cache", &MemoryScope::Project("alpha".into()))
            .unwrap()
            .len(),
        2
    );
    assert_eq!(store.search("cache", &session.scope).unwrap().len(), 1);
}

#[test]
fn legacy_data_is_neither_migrated_nor_modified() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join("db")).unwrap();
    fs::create_dir_all(temp.path().join("wiki/projects")).unwrap();
    let old_db = temp.path().join("db/memory.sqlite");
    let old_page = temp.path().join("wiki/projects/alpha.md");
    fs::write(&old_db, b"legacy database").unwrap();
    fs::write(&old_page, b"legacy markdown").unwrap();
    let store = MemoryStore::new(temp.path());
    store.initialize().unwrap();
    assert_eq!(store.reindex().unwrap(), 0);
    assert!(
        store
            .search("legacy", &MemoryScope::Project("alpha".into()))
            .unwrap()
            .is_empty()
    );
    assert_eq!(fs::read(old_db).unwrap(), b"legacy database");
    assert_eq!(fs::read(old_page).unwrap(), b"legacy markdown");
}

#[test]
fn namespace_access_recovers_deleted_index_before_allocating_ids() {
    let temp = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(temp.path()).with_context(context(
        "work",
        "alpha",
        "git:example.org/team/repo",
    ));
    let original = store.namespace("alpha").unwrap();
    let first = store.write(&record("first", "alpha")).unwrap();
    fs::remove_file(temp.path().join("db/workspaces.sqlite")).unwrap();
    assert_eq!(
        store
            .search("SQLite", &MemoryScope::Project("alpha".into()))
            .unwrap()
            .len(),
        1
    );
    assert_eq!(store.namespace("alpha").unwrap(), original);
    let second = store.write(&record("second", "alpha")).unwrap();
    assert_eq!(first.parent(), second.parent());
    assert_eq!(
        uuid_directories(&temp.path().join("wiki")).unwrap().len(),
        1
    );
    assert_eq!(
        uuid_directories(&temp.path().join("wiki").join(&original.workspace_id))
            .unwrap()
            .len(),
        1
    );
    assert_eq!(store.reindex().unwrap(), 2);
}

#[test]
fn initialization_and_rename_recover_existing_manifest_ids() {
    let temp = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(temp.path());
    let original = store.namespace("alpha").unwrap();
    store.write(&record("one", "alpha")).unwrap();
    fs::remove_file(temp.path().join("db/workspaces.sqlite")).unwrap();
    store.initialize().unwrap();
    assert_eq!(store.namespace("alpha").unwrap(), original);
    fs::remove_file(temp.path().join("db/workspaces.sqlite")).unwrap();
    store.rename_project("alpha", "beta").unwrap();
    assert_eq!(
        store.namespace("beta").unwrap().project_id,
        original.project_id
    );
}

#[test]
fn invalid_sources_block_automatic_recovery_without_creating_duplicates() {
    let temp = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(temp.path());
    let original = store.namespace("alpha").unwrap();
    let source = store.project_root(&original).join("_meta.md");
    fs::write(source, "broken manifest").unwrap();
    fs::remove_file(temp.path().join("db/workspaces.sqlite")).unwrap();
    assert!(store.namespace("alpha").is_err());
    assert!(store.initialize().is_err());
    assert_eq!(
        uuid_directories(&temp.path().join("wiki")).unwrap().len(),
        1
    );
    assert_eq!(
        uuid_directories(&temp.path().join("wiki").join(&original.workspace_id))
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn concurrent_writers_share_one_namespace_without_losing_pages() {
    let temp = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(temp.path());
    let workers = (0..12)
        .map(|index| {
            let store = store.clone();
            std::thread::spawn(move || {
                store
                    .write(&record(&format!("note-{index}"), "alpha"))
                    .unwrap();
                store.namespace("alpha").unwrap()
            })
        })
        .collect::<Vec<_>>();
    let namespaces = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    assert!(
        namespaces
            .iter()
            .all(|namespace| namespace == &namespaces[0])
    );
    assert_eq!(
        store
            .search("cache", &MemoryScope::Project("alpha".into()))
            .unwrap()
            .len(),
        12
    );
}

#[test]
fn rejects_traversal_empty_queries_and_reserved_profile_projects() {
    let temp = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(temp.path());
    assert!(store.namespace("../escape").is_err());
    assert!(store.namespace("").is_err());
    assert!(store.session_spool_path("alpha", "../../outside").is_err());
    assert!(store.search("  ", &MemoryScope::Profile).is_err());
    assert!(store.write(&record("one", "_profile")).is_err());
    assert!(store.rename_project("_profile", "other").is_err());
    assert!(store.rename_project("other", "_profile").is_err());
}

#[test]
fn malformed_rebuild_rolls_back_the_existing_index() {
    let temp = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(temp.path());
    let path = store.write(&record("note", "alpha")).unwrap();
    let source = fs::read_to_string(&path).unwrap();
    fs::write(
        &path,
        source.replace("\"scope\":\"project\"", "\"scope\":\"profile\""),
    )
    .unwrap();
    assert!(store.reindex().is_err());
    assert_eq!(
        store
            .search("SQLite", &MemoryScope::Project("alpha".into()))
            .unwrap()
            .len(),
        1
    );
    fs::write(&path, "invalid frontmatter").unwrap();
    assert!(store.reindex().is_err());
    assert_eq!(
        store
            .search("SQLite", &MemoryScope::Project("alpha".into()))
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn profile_sources_rebuild_within_their_workspace() {
    let temp = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(temp.path()).with_workspace("Pessoal");
    let mut page = record("preferences", "ignored");
    page.scope = MemoryScope::Profile;
    store.write(&page).unwrap();
    assert_eq!(store.reindex().unwrap(), 1);
    assert_eq!(
        store.search("SQLite", &MemoryScope::Profile).unwrap().len(),
        1
    );
    assert!(
        MemoryStore::new(temp.path())
            .search("SQLite", &MemoryScope::Profile)
            .unwrap()
            .is_empty()
    );
}

#[cfg(unix)]
#[test]
fn refuses_symlinked_namespace_directories() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(temp.path());
    let namespace = store.namespace("alpha").unwrap();
    let root = store.project_root(&namespace);
    symlink(outside.path(), root.join("memories")).unwrap();
    assert!(store.write(&record("escape", "alpha")).is_err());
    assert!(!outside.path().join("escape.md").exists());
}
