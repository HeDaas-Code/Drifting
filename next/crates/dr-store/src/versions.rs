//! Named whole-book versions. Each published directory contains a book file
//! and an integrity receipt. A failed save leaves no visible version.

use crate::digest::{file_sha256, sha256_hex};
use crate::file_io::{atomic_write, sync_directory, sync_file};
use crate::{Result, StoreError};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const ARCHIVE: &str = "book.driftingproject";
const MANIFEST: &str = "manifest.json";

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

    fn book_dir(&self, book: &str) -> PathBuf {
        self.root.join(sha256_hex(book.as_bytes()))
    }

    pub fn create<E: From<StoreError>>(
        &self,
        book_id: &str,
        name: &str,
        write_archive: impl FnOnce(&Path) -> std::result::Result<(), E>,
    ) -> std::result::Result<BookVersion, E> {
        let name = name.trim();
        if book_id.is_empty()
            || name.is_empty()
            || name.len() > 512
            || name.chars().any(char::is_control)
        {
            return Err(StoreError::invalid("book version", "invalid book or name").into());
        }
        let dir = self.book_dir(book_id);
        fs::create_dir_all(&dir).map_err(StoreError::from)?;
        let stage = tempfile::Builder::new()
            .prefix(".pending-")
            .tempdir_in(&dir)
            .map_err(StoreError::from)?;
        let archive = stage.path().join(ARCHIVE);
        write_archive(&archive)?;
        sync_file(&archive).map_err(StoreError::from)?;
        let (archive_sha256, archive_size) = file_sha256(&archive).map_err(StoreError::from)?;
        let version = BookVersion {
            id: Uuid::now_v7().to_string(),
            book_id: book_id.into(),
            name: name.into(),
            created_at: crate::now_ms(),
            archive_sha256,
            archive_size,
        };
        let manifest = serde_json::to_vec(&version)
            .map_err(|e| StoreError::invalid("book version manifest", e.to_string()))?;
        atomic_write(&stage.path().join(MANIFEST), &manifest).map_err(StoreError::from)?;
        fs::rename(stage.path(), dir.join(&version.id)).map_err(StoreError::from)?;
        sync_directory(&dir).map_err(StoreError::from)?;
        Ok(version)
    }

    pub fn list(&self, book_id: &str) -> Result<Vec<BookVersion>> {
        let dirs = match fs::read_dir(self.book_dir(book_id)) {
            Ok(dirs) => dirs,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut versions = Vec::new();
        for entry in dirs {
            let entry = entry?;
            if let Ok(id) = entry.file_name().to_string_lossy().parse::<Uuid>()
                && entry.file_type()?.is_dir()
            {
                versions.push(self.manifest(book_id, &id.to_string())?);
            }
        }
        versions.sort_unstable_by(|a, b| (b.created_at, &b.id).cmp(&(a.created_at, &a.id)));
        Ok(versions)
    }

    fn version_dir(&self, book_id: &str, id: &str) -> Result<PathBuf> {
        let uuid =
            Uuid::parse_str(id).map_err(|_| StoreError::invalid("book version", "invalid id"))?;
        Ok(self.book_dir(book_id).join(uuid.to_string()))
    }

    fn manifest(&self, book_id: &str, id: &str) -> Result<BookVersion> {
        let path = self.version_dir(book_id, id)?.join(MANIFEST);
        let version: BookVersion = serde_json::from_slice(&fs::read(path)?)
            .map_err(|e| StoreError::invalid("book version manifest", e.to_string()))?;
        if version.id != id || version.book_id != book_id {
            return Err(StoreError::invalid(
                "book version manifest",
                "wrong version or book",
            ));
        }
        Ok(version)
    }

    pub fn verified_archive(&self, book_id: &str, id: &str) -> Result<PathBuf> {
        let manifest = self.manifest(book_id, id)?;
        let path = self.book_dir(book_id).join(id).join(ARCHIVE);
        let (hash, size) = file_sha256(&path)?;
        if hash != manifest.archive_sha256 || size != manifest.archive_size {
            return Err(StoreError::invalid(
                "book version archive",
                "checksum mismatch",
            ));
        }
        Ok(path)
    }

    pub fn delete(&self, book_id: &str, id: &str) -> Result<()> {
        let dir = self.book_dir(book_id);
        let path = self.version_dir(book_id, id)?;
        // Retire the list entry as a single operation, before deleting bytes.
        let stage = tempfile::Builder::new()
            .prefix(".deleted-")
            .tempdir_in(&dir)?;
        fs::rename(path, stage.path().join("version"))?;
        sync_directory(&dir)?;
        stage.close()?;
        Ok(())
    }
}
