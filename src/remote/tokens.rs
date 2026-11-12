//! Bearer tokens for remote clients, stored as SHA-256 digests (never in plain text) in a
//! small owner-only file. The file is re-read when it changes, so revocation is immediate.

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

const TOKEN_PREFIX: &str = "vej_";
const TOKEN_BYTES: usize = 32;
const DIGEST_PREFIX: &str = "sha256:";
const MAX_NAME_LEN: usize = 64;
const FILE_HEADER: &str =
    "# vej remote access tokens (SHA-256 digests only). Manage with `vej remote token`.\n";

/// One authorized client.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TokenEntry {
    name: String,
    digest: [u8; 32],
}

/// Read side used by the server: verifies presented tokens against the file.
pub struct TokenStore {
    path: PathBuf,
    cache: Mutex<Option<CachedTokens>>,
}

struct CachedTokens {
    stamp: Option<FileStamp>,
    entries: Vec<TokenEntry>,
}

/// Identifies a version of the file without reading it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    modified: std::time::SystemTime,
    len: u64,
}

impl TokenStore {
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            cache: Mutex::new(None),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The client name for a valid token, `None` for an unknown one. Errors (e.g. a
    /// corrupt file) must be treated as "deny".
    pub fn verify(&self, token: &str) -> Result<Option<String>> {
        let presented = digest(token);
        self.with_entries(|entries| {
            // Compare against every entry so timing doesn't depend on which one matched.
            entries.iter().fold(None, |found, entry| {
                if constant_time_eq(&entry.digest, &presented) {
                    Some(entry.name.clone())
                } else {
                    found
                }
            })
        })
    }

    pub fn is_empty(&self) -> Result<bool> {
        self.with_entries(|entries| entries.is_empty())
    }

    /// Runs `f` on the current entries, re-reading the file if it changed.
    fn with_entries<T>(&self, f: impl FnOnce(&[TokenEntry]) -> T) -> Result<T> {
        let stamp = file_stamp(&self.path)?;
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        let is_fresh = cache.as_ref().is_some_and(|c| c.stamp == stamp);
        if !is_fresh {
            let entries = read_entries(&self.path)?;
            *cache = Some(CachedTokens { stamp, entries });
        }
        let entries = cache.as_ref().map_or(&[][..], |c| c.entries.as_slice());
        Ok(f(entries))
    }
}

/// Creates a token for `name`, returning it; only its digest is stored.
pub fn create(path: &Path, name: &str) -> Result<String> {
    validate_name(name)?;
    let mut entries = read_entries(path)?;
    if entries.iter().any(|e| e.name == name) {
        bail!("a token named '{name}' already exists; revoke it first to replace it");
    }
    let mut secret = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut secret).map_err(|e| anyhow::anyhow!("no secure randomness: {e}"))?;
    let token = format!("{TOKEN_PREFIX}{}", hex::encode(secret));
    entries.push(TokenEntry {
        name: name.to_string(),
        digest: digest(&token),
    });
    write_entries(path, &entries)?;
    Ok(token)
}

/// Removes `name`'s token. Returns whether it existed.
pub fn revoke(path: &Path, name: &str) -> Result<bool> {
    let entries = read_entries(path)?;
    let remaining: Vec<TokenEntry> = entries.iter().filter(|e| e.name != name).cloned().collect();
    if remaining.len() == entries.len() {
        return Ok(false);
    }
    write_entries(path, &remaining)?;
    Ok(true)
}

pub fn list(path: &Path) -> Result<Vec<String>> {
    Ok(read_entries(path)?.into_iter().map(|e| e.name).collect())
}

fn digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

fn constant_time_eq(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn validate_name(name: &str) -> Result<()> {
    let is_valid = !name.is_empty()
        && name.len() <= MAX_NAME_LEN
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !is_valid {
        bail!(
            "token names must be 1-{MAX_NAME_LEN} characters of letters, digits, '.', '_' or '-'"
        );
    }
    Ok(())
}

fn file_stamp(path: &Path) -> Result<Option<FileStamp>> {
    match std::fs::metadata(path) {
        Ok(meta) => Ok(Some(FileStamp {
            modified: meta
                .modified()
                .context("tokens file has no modification time")?,
            len: meta.len(),
        })),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
    }
}

fn read_entries(path: &Path) -> Result<Vec<TokenEntry>> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
    };
    contents
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
        .map(|(idx, line)| {
            parse_entry(line).with_context(|| {
                format!("{} line {}: invalid token entry", path.display(), idx + 1)
            })
        })
        .collect()
}

fn parse_entry(line: &str) -> Result<TokenEntry> {
    let mut fields = line.split_whitespace();
    let (Some(name), Some(encoded), None) = (fields.next(), fields.next(), fields.next()) else {
        bail!("expected '<name> {DIGEST_PREFIX}<hex>'");
    };
    validate_name(name)?;
    let hex_digest = encoded
        .strip_prefix(DIGEST_PREFIX)
        .with_context(|| format!("digest must start with '{DIGEST_PREFIX}'"))?;
    let mut digest = [0u8; 32];
    hex::decode_to_slice(hex_digest, &mut digest).context("digest is not 64 hex characters")?;
    Ok(TokenEntry {
        name: name.to_string(),
        digest,
    })
}

/// Writes atomically (temp file + rename) with owner-only permissions.
fn write_entries(path: &Path, entries: &[TokenEntry]) -> Result<()> {
    let dir = path
        .parent()
        .context("tokens file has no parent directory")?;
    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let mut contents = String::from(FILE_HEADER);
    for entry in entries {
        contents.push_str(&format!(
            "{} {DIGEST_PREFIX}{}\n",
            entry.name,
            hex::encode(entry.digest)
        ));
    }
    let tmp = path.with_extension("tmp");
    write_private(&tmp, &contents)?;
    std::fs::rename(&tmp, path).with_context(|| format!("cannot replace {}", path.display()))
}

#[cfg(unix)]
fn write_private(path: &Path, contents: &str) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("cannot write {}", path.display()))?;
    file.write_all(contents.as_bytes())
        .with_context(|| format!("cannot write {}", path.display()))
}

#[cfg(not(unix))]
fn write_private(path: &Path, contents: &str) -> Result<()> {
    std::fs::write(path, contents).with_context(|| format!("cannot write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_tokens_file(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vej-tokens-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("nested").join("remote_tokens")
    }

    #[test]
    fn created_tokens_are_random_prefixed_and_stored_only_as_digests() {
        let path = temp_tokens_file("create");

        let first = create(&path, "laptop").unwrap();
        let second = create(&path, "phone").unwrap();

        assert!(
            first.starts_with("vej_") && first.len() == 4 + 64,
            "{first}"
        );
        assert_ne!(first, second);
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(
            !contents.contains(&first[4..]),
            "plain token must not be stored"
        );
        assert!(contents.contains("laptop sha256:"));
        assert_eq!(list(&path).unwrap(), ["laptop", "phone"]);
    }

    #[cfg(unix)]
    #[test]
    fn tokens_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_tokens_file("perms");
        create(&path, "laptop").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn verifies_known_tokens_and_rejects_others() {
        let path = temp_tokens_file("verify");
        let token = create(&path, "laptop").unwrap();
        let store = TokenStore::open(&path);

        assert_eq!(store.verify(&token).unwrap().as_deref(), Some("laptop"));
        assert_eq!(store.verify("vej_wrong").unwrap(), None);
        assert_eq!(store.verify("").unwrap(), None);
    }

    #[test]
    fn missing_file_means_no_tokens() {
        let store = TokenStore::open(temp_tokens_file("missing"));
        assert!(store.is_empty().unwrap());
        assert_eq!(store.verify("vej_anything").unwrap(), None);
    }

    #[test]
    fn picks_up_new_and_revoked_tokens_without_reopening() {
        let path = temp_tokens_file("reload");
        let store = TokenStore::open(&path);
        assert!(store.is_empty().unwrap());

        let token = create(&path, "laptop").unwrap();
        assert_eq!(store.verify(&token).unwrap().as_deref(), Some("laptop"));

        assert!(revoke(&path, "laptop").unwrap());
        assert_eq!(
            store.verify(&token).unwrap(),
            None,
            "revocation is immediate"
        );
        assert!(!revoke(&path, "laptop").unwrap());
    }

    #[test]
    fn rejects_duplicate_and_invalid_names() {
        let path = temp_tokens_file("names");
        create(&path, "laptop").unwrap();
        assert!(
            create(&path, "laptop")
                .unwrap_err()
                .to_string()
                .contains("already exists")
        );
        for bad in ["", "has space", "semi;colon", &"x".repeat(65)] {
            assert!(create(&path, bad).is_err(), "'{bad}' should be rejected");
        }
    }

    #[test]
    fn corrupt_file_fails_closed_with_line_number() {
        let path = temp_tokens_file("corrupt");
        create(&path, "laptop").unwrap();
        let mut contents = std::fs::read_to_string(&path).unwrap();
        contents.push_str("phone not-a-digest\n");
        std::fs::write(&path, contents).unwrap();

        let err = TokenStore::open(&path).verify("vej_x").unwrap_err();
        assert!(format!("{err:#}").contains("line"), "{err:#}");
    }
}
