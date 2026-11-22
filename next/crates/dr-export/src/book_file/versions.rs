//! Named snapshots use the same file format and validation as project transfer.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use dr_store::versions::{BookVersion, VersionStore};

use super::{
    BookFileError, BookFileSummary, DocId, MANIFEST, Manifest, Progress, Result, Workspace,
    ZipArchive, export_book, import_book_cancellable, parse_manifest,
};

fn store(ws: &Workspace) -> VersionStore {
    VersionStore::new(ws.library().paths().root.join("versions"))
}

pub fn list(ws: &Workspace, book: &str) -> Result<Vec<BookVersion>> {
    Ok(store(ws).list(book)?)
}

pub fn save(ws: &Workspace, book: &str, name: &str) -> Result<BookVersion> {
    store(ws).create(book, name, |path| {
        export_book(ws, book, path, &|_| {}).map(|_| ())
    })
}

pub fn delete(ws: &Workspace, book: &str, id: &str) -> Result<()> {
    Ok(store(ws).delete(book, id)?)
}

pub fn restore(
    ws: &Workspace,
    book: &str,
    id: &str,
    progress: &dyn Fn(Progress),
    cancelled: &dyn Fn() -> bool,
) -> Result<BookFileSummary> {
    let path = store(ws).verified_archive(book, id)?;
    import_book_cancellable(ws, &path, progress, cancelled)
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct Changes {
    pub prose_added: usize,
    pub prose_changed: usize,
    pub prose_removed: usize,
    pub structure_changed: bool,
    pub asset_files_changed: usize,
}

/// Compare the selected version with an optional earlier snapshot.
pub fn compare(
    ws: &Workspace,
    book: &str,
    current: &str,
    earlier: Option<&str>,
) -> Result<Changes> {
    let store = store(ws);
    let current = manifest(&store.verified_archive(book, current)?)?;
    let earlier = earlier
        .map(|id| manifest(&store.verified_archive(book, id)?))
        .transpose()?;
    let before = earlier.as_ref();
    let mut prose_before: BTreeMap<&str, &str> = BTreeMap::new();
    let mut assets_before: BTreeMap<(&str, &str), &str> = BTreeMap::new();
    if let Some(before) = before {
        for doc in &before.documents {
            if is_prose(&doc.id) {
                prose_before.insert(doc.id.as_str(), doc.sha256.as_str());
            }
        }
        for asset in &before.assets {
            assets_before.insert(
                (asset.asset.as_str(), asset.variant.as_str()),
                asset.sha256.as_str(),
            );
        }
    }
    let mut changes = Changes {
        structure_changed: before.and_then(structure_hash) != structure_hash(&current),
        ..Default::default()
    };
    for doc in current.documents.iter().filter(|doc| is_prose(&doc.id)) {
        match prose_before.remove(doc.id.as_str()) {
            None => changes.prose_added += 1,
            Some(old) if old != doc.sha256 => changes.prose_changed += 1,
            Some(_) => {}
        }
    }
    changes.prose_removed = prose_before.len();
    for asset in &current.assets {
        let old = assets_before.remove(&(asset.asset.as_str(), asset.variant.as_str()));
        changes.asset_files_changed += usize::from(old != Some(asset.sha256.as_str()));
    }
    changes.asset_files_changed += assets_before.len();
    Ok(changes)
}

fn structure_hash(manifest: &Manifest) -> Option<&str> {
    manifest
        .documents
        .iter()
        .find(|doc| doc.id.starts_with("book:"))
        .map(|doc| doc.sha256.as_str())
}

fn is_prose(id: &str) -> bool {
    DocId::parse(id).is_ok_and(|doc| doc.kind().is_book_text())
}

fn manifest(path: &Path) -> Result<Manifest> {
    let file = File::open(path)?;
    let mut zip = ZipArchive::new(file).map_err(|_| BookFileError::NotABook)?;
    let mut raw = Vec::new();
    zip.by_name(MANIFEST)
        .map_err(|_| BookFileError::NotABook)?
        .take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut raw)?;
    if raw.len() > 64 * 1024 * 1024 {
        return Err(BookFileError::TooLarge);
    }
    parse_manifest(&raw)
}
