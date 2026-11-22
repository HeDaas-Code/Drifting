//! Crash recovery for book-file imports. The book document is committed last.

use loro::VersionVector;
use serde::{Deserialize, Serialize};

use crate::{Library, Result, StoreError};

const SCOPE: &str = "book-file-import-v1";

#[derive(Serialize, Deserialize)]
struct Receipt {
    book: String,
    final_book_vv: Vec<u8>,
    checksum: String,
}

impl Library {
    /// Reserve a fresh target before storing any imported asset or document.
    pub fn begin_book_import(&self, book: &str, final_book_vv: &VersionVector) -> Result<()> {
        if uuid::Uuid::parse_str(book).is_err()
            || final_book_vv.is_empty()
            || !self.docs(Some(book), None)?.is_empty()
            || !self.assets_of_book(book)?.is_empty()
        {
            return Err(StoreError::invalid(
                "book file import",
                "target must be a fresh book",
            ));
        }
        let encoded_vv = final_book_vv.encode();
        let receipt = serde_json::to_string(&Receipt {
            book: book.into(),
            checksum: checksum(book, &encoded_vv),
            final_book_vv: encoded_vv,
        })
        .map_err(|e| StoreError::invalid("book import receipt", e.to_string()))?;
        self.local_write(|tx| {
            if tx.get_state(SCOPE, book)?.is_some() {
                return Err(StoreError::invalid(
                    "book import receipt",
                    "target already reserved",
                ));
            }
            // Not associated with the book: partial-book purge must retain it.
            tx.set_state(SCOPE, book, &receipt, None)
        })
    }

    /// Retire the receipt only after the book finished, or its rollback finished.
    pub fn finish_book_import(&self, book: &str) -> Result<()> {
        self.local_write(|tx| tx.remove_state(SCOPE, book).map(|_| ()))
    }

    pub(crate) fn recover_book_imports(&self) -> Result<()> {
        // Tests and migration previews may deliberately open a pre-local-state schema.
        let has_state = self.local_read(|conn| {
            Ok(conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='local_state')",
                [], |row| row.get::<_, bool>(0),
            )?)
        })?;
        if !has_state {
            return Ok(());
        }
        for entry in self.local_entries(SCOPE, None)? {
            let receipt: Receipt = serde_json::from_str(&entry.value)
                .map_err(|e| StoreError::invalid("book import receipt", e.to_string()))?;
            if receipt.book != entry.key
                || uuid::Uuid::parse_str(&receipt.book).is_err()
                || checksum(&receipt.book, &receipt.final_book_vv) != receipt.checksum
            {
                return Err(StoreError::invalid(
                    "book import receipt",
                    "target identity mismatch",
                ));
            }
            let final_vv = VersionVector::decode(&receipt.final_book_vv)
                .map_err(|e| StoreError::invalid("book import receipt", e.to_string()))?;
            let book_doc = format!("book:{}", receipt.book);
            let complete = match self.load_copy(&book_doc) {
                Ok(doc) => doc.oplog_vv() == final_vv,
                Err(StoreError::NotFound(_)) => false,
                Err(error) => return Err(error),
            };
            if !complete {
                for doc in self.docs(Some(&receipt.book), None)? {
                    self.purge_doc(&doc.doc_id)?;
                    self.forget_purged(&doc.doc_id)?;
                }
                for asset in self.assets_of_book(&receipt.book)? {
                    self.remove_asset(&asset.asset_id)?;
                }
            }
            self.finish_book_import(&receipt.book)?;
        }
        Ok(())
    }
}

fn checksum(book: &str, version: &[u8]) -> String {
    let mut bytes = book.as_bytes().to_vec();
    bytes.push(0);
    bytes.extend_from_slice(version);
    crate::digest::sha256_hex(&bytes)
}
