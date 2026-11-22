//! Linux Secret Service store (freedesktop.org Secret Service API).

use keyring::{Entry, Error as KeyringError};

use super::{RawStore, Result, SecretError};

pub(crate) struct SecretService {
    service: String,
}

impl SecretService {
    pub(crate) fn new(service: &str) -> Self {
        Self {
            service: service.into(),
        }
    }
    fn entry(&self, key: &str) -> Result<Entry> {
        Entry::new(&self.service, key).map_err(|error| SecretError::Platform {
            operation: "open",
            code: error_code(error),
        })
    }
}

fn error_code(error: KeyringError) -> i64 {
    match error {
        KeyringError::NoEntry => -1,
        _ => -2,
    }
}

impl RawStore for SecretService {
    fn get(&self, key: &str) -> Result<Option<String>> {
        let entry = self.entry(key)?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(error) => Err(SecretError::Platform {
                operation: "read",
                code: error_code(error),
            }),
        }
    }
    fn exists(&self, key: &str) -> Result<bool> {
        match self.entry(key)?.get_password() {
            Ok(_) => Ok(true),
            Err(KeyringError::NoEntry) => Ok(false),
            Err(error) => Err(SecretError::Platform {
                operation: "check",
                code: error_code(error),
            }),
        }
    }
    fn set(&self, key: &str, value: &str) -> Result<()> {
        self.entry(key)?
            .set_password(value)
            .map_err(|error| SecretError::Platform {
                operation: "write",
                code: error_code(error),
            })
    }
    fn delete(&self, key: &str) -> Result<()> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(error) => Err(SecretError::Platform {
                operation: "delete",
                code: error_code(error),
            }),
        }
    }
}
