use dr_store::versions::VersionStore;
use dr_store::{Result, StoreError};

#[test]
fn versions_publish_atomically_reopen_verify_and_only_delete_explicitly() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("versions");
    let store = VersionStore::new(&root);
    let book = "../../synthetic-book";
    let version = store
        .create::<StoreError>(book, " 初稿 ", |path| {
            std::fs::write(path, b"synthetic project")?;
            Ok(())
        })
        .unwrap();
    assert_eq!(version.name, "初稿");
    let again = VersionStore::new(&root);
    assert_eq!(again.list(book).unwrap(), vec![version.clone()]);
    assert!(again.list("another-book").unwrap().is_empty());
    assert!(again.verified_archive("another-book", &version.id).is_err());
    assert!(again.verified_archive(book, "../../escape").is_err());
    let archive = again.verified_archive(book, &version.id).unwrap();
    assert_eq!(std::fs::read(&archive).unwrap(), b"synthetic project");
    std::fs::write(&archive, b"damaged").unwrap();
    assert!(again.verified_archive(book, &version.id).is_err());
    // Damage never removes the named version. The author can still delete it.
    assert_eq!(again.list(book).unwrap().len(), 1);
    again.delete(book, &version.id).unwrap();
    assert!(again.list(book).unwrap().is_empty());
}

#[test]
fn interrupted_save_never_publishes_a_partial_named_version() {
    let temp = tempfile::tempdir().unwrap();
    let store = VersionStore::new(temp.path());
    let result: Result<_> = store.create("book", "incomplete", |path| {
        std::fs::write(path, b"partial")?;
        Err(StoreError::Invalid {
            what: "test",
            value: "interrupted".into(),
        })
    });
    assert!(result.is_err());
    assert!(store.list("book").unwrap().is_empty());
}
