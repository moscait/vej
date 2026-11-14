//! Verified model downloads: the bytes are hashed while streaming and the file is only
//! committed to the cache when the digest matches the catalog's pinned `sha256`.

use anyhow::{Context, Result, anyhow};
use indicatif::{ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::{ModelSpec, RECOMMENDED_MODELS, find_model_spec, get_cache_dir};

/// A download in progress. Bytes go to a temporary file and into a SHA-256 hasher at the
/// same time. [`VerifiedDownload::commit`] checks the digest and renames the file into place;
/// dropping the value before that (an error, a mismatch, a cancelled future) deletes the
/// temporary file so no unverified bytes are left in the cache.
pub(crate) struct VerifiedDownload {
    file: Option<File>,
    hasher: Sha256,
    temp_path: PathBuf,
    committed: bool,
}

impl VerifiedDownload {
    pub(crate) fn create(temp_path: &Path) -> Result<Self> {
        let file = File::create(temp_path)
            .with_context(|| format!("Failed to create temporary file {:?}", temp_path))?;
        Ok(Self {
            file: Some(file),
            hasher: Sha256::new(),
            temp_path: temp_path.to_path_buf(),
            committed: false,
        })
    }

    pub(crate) fn write(&mut self, chunk: &[u8]) -> Result<()> {
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| anyhow!("download already finished"))?;
        file.write_all(chunk)
            .with_context(|| format!("Error writing to temporary file {:?}", self.temp_path))?;
        self.hasher.update(chunk);
        Ok(())
    }

    /// Flushes the file to disk, compares the digest with `expected_sha256` (case-insensitive)
    /// and moves the file to `dest`. On a mismatch the temporary file is deleted and the error
    /// names both digests.
    pub(crate) fn commit(mut self, expected_sha256: &str, dest: &Path) -> Result<()> {
        let file = self
            .file
            .take()
            .ok_or_else(|| anyhow!("download already finished"))?;
        file.sync_all()
            .with_context(|| format!("Failed to flush {:?} to disk", self.temp_path))?;
        drop(file);

        let actual = hex::encode(std::mem::take(&mut self.hasher).finalize());
        let expected = expected_sha256.trim();
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(anyhow!(
                "SHA-256 mismatch for {:?}: expected {}, got {}. \
                 The downloaded file was discarded; the upstream file may have changed.",
                dest.file_name().unwrap_or(dest.as_os_str()),
                expected.to_lowercase(),
                actual
            ));
        }

        std::fs::rename(&self.temp_path, dest)
            .with_context(|| format!("Failed to move {:?} to {:?}", self.temp_path, dest))?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for VerifiedDownload {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        // Close the handle before unlinking so this also works on Windows.
        self.file.take();
        if let Err(e) = std::fs::remove_file(&self.temp_path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = ?self.temp_path, error = %e, "could not remove partial download");
        }
    }
}

fn catalog_ids() -> String {
    RECOMMENDED_MODELS
        .iter()
        .map(|m| m.id)
        .collect::<Vec<_>>()
        .join(", ")
}

fn progress_bar(total_size: u64) -> Result<ProgressBar> {
    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template(
                "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})",
            )?
            .progress_chars("#>-"),
    );
    Ok(pb)
}

async fn stream_to_file(spec: &ModelSpec, temp_file: &Path, dest_file: &Path) -> Result<()> {
    let url = spec.download_url();
    let res = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .with_context(|| format!("Failed to connect to {}", url))?;
    if !res.status().is_success() {
        return Err(anyhow!(
            "Download failed with HTTP status {}: {}",
            res.status(),
            url
        ));
    }

    let pb = progress_bar(res.content_length().unwrap_or(0))?;
    let mut download = VerifiedDownload::create(temp_file)?;
    let mut res = res;
    while let Some(chunk) = res
        .chunk()
        .await
        .with_context(|| "Error reading chunk from stream")?
    {
        download.write(&chunk)?;
        pb.inc(chunk.len() as u64);
    }
    pb.finish_with_message("Download complete");

    download.commit(spec.sha256, dest_file)
}

pub async fn pull_model(query: &str) -> Result<PathBuf> {
    let spec = find_model_spec(query).ok_or_else(|| {
        anyhow!(
            "Unknown model '{}'. Supported presets are: {}",
            query,
            catalog_ids()
        )
    })?;

    let dest_dir = get_cache_dir();
    std::fs::create_dir_all(&dest_dir)
        .with_context(|| format!("Failed to create model cache directory: {:?}", dest_dir))?;

    let dest_file = dest_dir.join(spec.filename);
    if dest_file.exists() {
        println!(
            "Model '{}' is already downloaded at {:?}",
            spec.id, dest_file
        );
        return Ok(dest_file);
    }

    println!(
        "Downloading {} ({} - ~{:.2} GB) from Hugging Face...",
        spec.name, spec.country, spec.size_gb
    );
    println!("Source: {}", spec.download_url());
    println!("Expected SHA-256: {}", spec.sha256);

    let temp_file = dest_dir.join(format!("{}.tmp", spec.filename));
    stream_to_file(spec, &temp_file, &dest_file).await?;

    println!("Verified SHA-256 and saved model to {:?}", dest_file);
    Ok(dest_file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const HELLO_WORLD_SHA256: &str =
        "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn scratch_dir(test: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("vej-download-{test}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn write_hello_world(download: &mut VerifiedDownload) {
        download.write(b"hello ").expect("write first chunk");
        download.write(b"world").expect("write second chunk");
    }

    #[test]
    fn commit_moves_the_file_into_place_when_the_hash_matches() {
        let dir = scratch_dir("ok");
        let temp = dir.join("model.gguf.tmp");
        let dest = dir.join("model.gguf");

        let mut download = VerifiedDownload::create(&temp).expect("create temp file");
        write_hello_world(&mut download);
        download
            .commit(HELLO_WORLD_SHA256, &dest)
            .expect("hash matches");

        assert_eq!(std::fs::read(&dest).expect("read dest"), b"hello world");
        assert!(!temp.exists(), "temp file must be renamed away");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn commit_rejects_a_mismatched_hash_and_removes_the_temp_file() {
        let dir = scratch_dir("mismatch");
        let temp = dir.join("model.gguf.tmp");
        let dest = dir.join("model.gguf");
        let expected = "0".repeat(64);

        let mut download = VerifiedDownload::create(&temp).expect("create temp file");
        write_hello_world(&mut download);
        let err = download
            .commit(&expected, &dest)
            .expect_err("hash must not match");

        let msg = err.to_string();
        assert!(
            msg.contains(&expected),
            "message names expected hash: {msg}"
        );
        assert!(
            msg.contains(HELLO_WORLD_SHA256),
            "message names actual hash: {msg}"
        );
        assert!(!dest.exists(), "nothing may be committed on mismatch");
        assert!(!temp.exists(), "temp file must be deleted on mismatch");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn hash_comparison_ignores_case_of_the_expected_digest() {
        let dir = scratch_dir("case");
        let temp = dir.join("model.gguf.tmp");
        let dest = dir.join("model.gguf");

        let mut download = VerifiedDownload::create(&temp).expect("create temp file");
        write_hello_world(&mut download);
        download
            .commit(&HELLO_WORLD_SHA256.to_uppercase(), &dest)
            .expect("case-insensitive match");

        assert!(dest.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dropping_an_uncommitted_download_removes_the_temp_file() {
        let dir = scratch_dir("drop");
        let temp = dir.join("model.gguf.tmp");

        {
            let mut download = VerifiedDownload::create(&temp).expect("create temp file");
            download.write(b"partial").expect("write");
            assert!(temp.exists());
        }

        assert!(!temp.exists(), "drop guard must remove the partial file");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn create_fails_when_the_directory_is_missing() {
        let dir = scratch_dir("nodir");
        let temp = dir.join("missing").join("model.gguf.tmp");
        assert!(VerifiedDownload::create(&temp).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
