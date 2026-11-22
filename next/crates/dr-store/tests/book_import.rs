mod common;

use dr_store::{Library, OpenDoc};
use serde_json::json;

#[test]
fn invalid_recovery_receipt_stops_open_without_removing_existing_book() {
    let dir = tempfile::tempdir().unwrap();
    let source = "019a9d46-d000-7000-8000-000000000001";
    let lib = common::open(dir.path());
    let id = format!("book:{source}");
    let handle = lib
        .open_doc(
            &id,
            OpenDoc::CreateIfMissing {
                kind: "book",
                book_id: Some(source),
                genesis: &common::genesis,
            },
        )
        .unwrap();
    common::insert(&handle, dr_store::Origin::User, 0, "合成作品");
    handle.close();
    let original = common::text(&lib.load_copy(&id).unwrap());
    lib.local_set(
        "book-file-import-v1",
        source,
        &json!({
            "book": source,
            "final_book_vv": [0],
            "checksum": "damaged"
        })
        .to_string(),
        None,
    )
    .unwrap();
    lib.close().unwrap();
    assert!(Library::open(dir.path(), common::options()).is_err());
    let readonly = Library::open_read_only(dir.path(), common::options()).unwrap();
    assert_eq!(common::text(&readonly.load_copy(&id).unwrap()), original);
}
