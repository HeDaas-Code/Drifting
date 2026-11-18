//! Named whole-book versions. The version payload is an exported book archive.

use crate::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookVersion {
    pub id: String,
    pub book_id: String,
    pub name: String,
    pub created_at: i64,
    pub archive_sha256: String,
    pub archive_size: u64,
}

pub struct VersionStore {
    root: PathBuf,
}

impl VersionStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
    pub fn create(&self, book_id: &str, name: &str, archive: &Path) -> Result<BookVersion> {
        if book_id.trim().is_empty() || name.trim().is_empty() {
            return Err(crate::StoreError::Invalid {
                what: "book version",
                value: "id and name are required".into(),
            });
        }
        let bytes = std::fs::read(archive)?;
        let id = Uuid::now_v7().to_string();
        let dir = self.root.join(book_id).join(&id);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("book.driftingbook"), &bytes)?;
        let version = BookVersion {
            id,
            book_id: book_id.into(),
            name: name.trim().into(),
            created_at: crate::now_ms(),
            archive_sha256: format!("{:x}", Sha256::digest(&bytes)),
            archive_size: bytes.len() as u64,
        };
        let manifest =
            serde_json::to_vec_pretty(&version).map_err(|error| crate::StoreError::Invalid {
                what: "book version manifest",
                value: error.to_string(),
            })?;
        std::fs::write(dir.join("manifest.json"), manifest)?;
        Ok(version)
    }
    pub fn list(&self, book_id: &str) -> Result<Vec<BookVersion>> {
        let dir = self.root.join(book_id);
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut versions = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path().join("manifest.json");
            if path.exists() {
                let value: BookVersion =
                    serde_json::from_slice(&std::fs::read(path)?).map_err(|error| {
                        crate::StoreError::Invalid {
                            what: "book version manifest",
                            value: error.to_string(),
                        }
                    })?;
                versions.push(value);
            }
        }
        versions.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(versions)
    }
    pub fn restore_copy(&self, version: &BookVersion, destination: &Path) -> Result<()> {
        let source = self
            .root
            .join(&version.book_id)
            .join(&version.id)
            .join("book.driftingbook");
        let bytes = std::fs::read(source)?;
        if format!("{:x}", Sha256::digest(&bytes)) != version.archive_sha256 {
            return Err(crate::StoreError::Invalid {
                what: "book version archive",
                value: "checksum mismatch".into(),
            });
        }
        std::fs::write(destination, bytes)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn named_versions_list_and_restore_as_a_copy() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.driftingbook");
        std::fs::write(&source, b"synthetic book archive").unwrap();
        let store = VersionStore::new(temp.path().join("versions"));
        let version = store.create("book-1", "Before rewrite", &source).unwrap();
        assert_eq!(store.list("book-1").unwrap(), vec![version.clone()]);
        let restored = temp.path().join("restored.driftingbook");
        store.restore_copy(&version, &restored).unwrap();
        assert_eq!(std::fs::read(restored).unwrap(), b"synthetic book archive");
    }
}
