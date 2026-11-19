use dr_export::book_file::{BookFileError, versions};
use dr_store::versions::VersionStore;
use dr_testkit::seed::{DEFAULT_SEED, Preset, digest_v3, generate_v3};
use dr_workspace::{Actor, Workspace};
use serde_json::json;

fn setup() -> (tempfile::TempDir, Workspace, String) {
    let temp = tempfile::tempdir().unwrap();
    let store =
        dr_store::Library::open(temp.path(), dr_workspace::library_options("test")).unwrap();
    let seed = generate_v3(Preset::Showcase, DEFAULT_SEED).unwrap();
    let book = seed.books[0].book_id().to_string();
    seed.write_to(&store).unwrap();
    seed.write_files_to(&store).unwrap();
    store.close().unwrap();
    let ws = Workspace::open_dir(temp.path(), "test").unwrap();
    (temp, ws, book)
}

#[test]
fn saved_version_restores_a_new_book_with_assets_after_reopen_and_keeps_source() {
    let (temp, ws, book) = setup();
    let before = digest_v3(ws.library(), &book).unwrap();
    let original = ws.observe(&format!("book:{book}")).unwrap();
    let version = versions::save(&ws, &book, "Before restructuring").unwrap();
    assert_eq!(versions::list(&ws, &book).unwrap(), vec![version.clone()]);
    ws.call(
        &Actor::User,
        "project.update",
        json!({"book":book,"name":"Edited later"}),
    )
    .unwrap();
    let prose_id = ws.library().docs(Some(&book), Some("body")).unwrap()[0]
        .doc_id
        .clone();
    let session = ws.prose_session(&prose_id).unwrap();
    let seed_text = session.reader().view().plain_text();
    session
        .edit(move |txn| txn.insert_paragraphs(0, &seed_text))
        .unwrap();
    drop(session);
    let second = versions::save(&ws, &book, "New title").unwrap();
    let difference = versions::compare(&ws, &book, &second.id, Some(&version.id)).unwrap();
    assert!(difference.structure_changed);
    assert_eq!(difference.prose_changed, 1);
    ws.shutdown().unwrap();

    let ws = Workspace::open_dir(temp.path(), "test").unwrap();
    assert_eq!(versions::list(&ws, &book).unwrap().len(), 2);
    let current = ws.observe(&format!("book:{book}")).unwrap();
    let restored = versions::restore(&ws, &book, &version.id, &|_| {}, &|| false).unwrap();
    assert_ne!(restored.book, book);
    assert_eq!(digest_v3(ws.library(), &restored.book).unwrap(), before);
    assert_eq!(
        ws.observe(&format!("book:{book}")).unwrap().version,
        current.version
    );
    assert_ne!(current.version, original.version);
    let source_assets = ws.library().assets_of_book(&book).unwrap();
    let restored_assets = ws.library().assets_of_book(&restored.book).unwrap();
    assert!(!source_assets.is_empty(), "showcase must cover assets");
    assert_eq!(source_assets.len(), restored_assets.len());
    let hashes = |assets: &[dr_store::asset::AssetRecord]| {
        let mut hashes: Vec<_> = assets
            .iter()
            .map(|asset| dr_store::digest::file_sha256(&asset.path).unwrap())
            .collect();
        hashes.sort();
        hashes
    };
    assert_eq!(hashes(&source_assets), hashes(&restored_assets));
    ws.shutdown().unwrap();
}

#[test]
fn cancellation_and_corrupt_version_leave_source_and_library_inventory_unchanged() {
    let (_temp, ws, book) = setup();
    let version = versions::save(&ws, &book, "Complete version").unwrap();
    let before = digest_v3(ws.library(), &book).unwrap();
    let docs = ws.library().docs(None, None).unwrap();
    let cancel = std::cell::Cell::new(false);
    let result = versions::restore(&ws, &book, &version.id, &|_| cancel.set(true), &|| {
        cancel.get()
    });
    assert!(
        matches!(result, Err(BookFileError::Cancelled)),
        "{result:?}"
    );
    assert_eq!(ws.library().docs(None, None).unwrap(), docs);
    assert_eq!(digest_v3(ws.library(), &book).unwrap(), before);
    let store = VersionStore::new(ws.library().paths().root.join("versions"));
    let archive = store.verified_archive(&book, &version.id).unwrap();
    std::fs::write(archive, b"broken archive").unwrap();
    assert!(versions::restore(&ws, &book, &version.id, &|_| {}, &|| false).is_err());
    assert_eq!(ws.library().docs(None, None).unwrap(), docs);
    assert_eq!(digest_v3(ws.library(), &book).unwrap(), before);
    versions::delete(&ws, &book, &version.id).unwrap();
    assert!(versions::list(&ws, &book).unwrap().is_empty());
    ws.shutdown().unwrap();
}

#[test]
fn version_restore_process_exits_at_the_selected_commit_boundary() {
    let Some(root) = std::env::var_os("DRIFTING_VERSION_CRASH_TEST_ROOT") else {
        return;
    };
    let book = std::env::var("DRIFTING_VERSION_CRASH_TEST_BOOK").unwrap();
    let version = std::env::var("DRIFTING_VERSION_CRASH_TEST_ID").unwrap();
    let boundary = std::env::var("DRIFTING_VERSION_CRASH_TEST_BOUNDARY").unwrap();
    let assets: usize = std::env::var("DRIFTING_VERSION_CRASH_TEST_ASSETS")
        .unwrap()
        .parse()
        .unwrap();
    let ws = Workspace::open_dir(root, "test").unwrap();
    let _ = versions::restore(
        &ws,
        &book,
        &version,
        &|progress| {
            let exit = match boundary.as_str() {
                "after-book" => progress.done == progress.total,
                "during-prose" => progress.done > assets && progress.done < progress.total,
                _ => panic!("unknown test boundary"),
            };
            if exit {
                std::process::exit(86);
            }
        },
        &|| false,
    );
    panic!("restore did not reach the crash boundary");
}

#[test]
fn forced_process_exit_cleans_partial_copy_and_keeps_fully_committed_copy() {
    for boundary in ["during-prose", "after-book"] {
        let (temp, ws, book) = setup();
        let version = versions::save(&ws, &book, "Crash recovery").unwrap();
        let before = digest_v3(ws.library(), &book).unwrap();
        let docs = ws.library().docs(None, None).unwrap();
        let source_assets = ws.library().assets_of_book(&book).unwrap();
        ws.shutdown().unwrap();
        let exit = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "version_restore_process_exits_at_the_selected_commit_boundary",
                "--nocapture",
            ])
            .env("DRIFTING_VERSION_CRASH_TEST_ROOT", temp.path())
            .env("DRIFTING_VERSION_CRASH_TEST_BOOK", &book)
            .env("DRIFTING_VERSION_CRASH_TEST_ID", &version.id)
            .env(
                "DRIFTING_VERSION_CRASH_TEST_ASSETS",
                source_assets.len().to_string(),
            )
            .env("DRIFTING_VERSION_CRASH_TEST_BOUNDARY", boundary)
            .status()
            .unwrap();
        assert_eq!(exit.code(), Some(86), "{boundary}");
        let ws = Workspace::open_dir(temp.path(), "test").unwrap();
        assert_eq!(digest_v3(ws.library(), &book).unwrap(), before);
        let all_docs = ws.library().docs(None, None).unwrap();
        if boundary == "during-prose" {
            assert_eq!(all_docs, docs);
        } else {
            let new_book = all_docs
                .iter()
                .find(|doc| doc.kind == "book" && !docs.contains(doc))
                .unwrap();
            let copy = new_book.book_id.as_deref().unwrap();
            assert_ne!(copy, book);
            assert_eq!(digest_v3(ws.library(), copy).unwrap(), before);
            assert_eq!(
                ws.library().assets_of_book(copy).unwrap().len(),
                source_assets.len()
            );
        }
        assert!(
            ws.library()
                .local_entries("book-file-import-v1", None)
                .unwrap()
                .is_empty()
        );
        ws.shutdown().unwrap();
    }
}
