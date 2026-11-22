//! Opt-in downloads of multilingual Whisper weights. Only verified bytes replace
//! the installed model; cancellation and failures remove the temporary download.
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Result, bail};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug)]
pub struct Model {
    pub name: &'static str,
    pub file: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

// Hugging Face LFS OIDs at this immutable revision are SHA-256 digests.
pub const REVISION: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";
pub const MODELS: [Model; 3] = [
    Model {
        name: "Tiny · 78 MB",
        file: "ggml-tiny.bin",
        bytes: 77_691_713,
        sha256: "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
    },
    Model {
        name: "Base · 148 MB",
        file: "ggml-base.bin",
        bytes: 147_951_465,
        sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
    },
    Model {
        name: "Small · 488 MB",
        file: "ggml-small.bin",
        bytes: 487_601_967,
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
    },
];

#[cfg(feature = "network")]
pub fn download(model: Model, cancelled: &AtomicBool) -> Result<()> {
    if cancelled.load(Ordering::Relaxed) {
        bail!("Model download cancelled");
    }
    crate::install_tls_provider();
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .connect_timeout(std::time::Duration::from_secs(20))
        .timeout(std::time::Duration::from_secs(300))
        .build()?;
    let url = format!(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/{REVISION}/{}",
        model.file
    );
    let response = client.get(url).send()?.error_for_status()?;
    install_verified(response, &super::model_path()?, model, cancelled)
}

#[cfg(not(feature = "network"))]
pub fn download(_model: Model, _cancelled: &AtomicBool) -> Result<()> {
    bail!("This build requires a locally imported Whisper model")
}

fn install_verified(
    mut input: impl Read,
    destination: &Path,
    model: Model,
    cancelled: &AtomicBool,
) -> Result<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Missing model directory"))?;
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    let mut hash = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0; 64 * 1024];
    loop {
        if cancelled.load(Ordering::Relaxed) {
            bail!("Model download cancelled");
        }
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > model.bytes {
            bail!("Model exceeds its published size");
        }
        hash.update(&buffer[..count]);
        temporary.write_all(&buffer[..count])?;
    }
    if total != model.bytes || format!("{:x}", hash.finalize()) != model.sha256 {
        bail!("Model integrity check failed; the previous model was kept");
    }
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    if cancelled.load(Ordering::Relaxed) {
        bail!("Model download cancelled");
    }
    temporary
        .persist(destination)
        .map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verified_install_replaces_atomically_and_refusals_keep_the_previous_model() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.bin");
        // Synthetic bytes: no speech, manuscript or downloaded weights in fixtures.
        let model = Model {
            name: "synthetic",
            file: "test",
            bytes: 3,
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        };
        let cancel = AtomicBool::new(false);
        std::fs::write(&dest, b"previous").unwrap();
        for bytes in [b"ab".as_slice(), b"bad", b"abcd"] {
            assert!(install_verified(bytes, &dest, model, &cancel).is_err());
            assert_eq!(std::fs::read(&dest).unwrap(), b"previous");
        }
        cancel.store(true, Ordering::Relaxed);
        assert!(install_verified(b"abc".as_slice(), &dest, model, &cancel).is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), b"previous");
        cancel.store(false, Ordering::Relaxed);
        install_verified(b"abc".as_slice(), &dest, model, &cancel).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"abc");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
