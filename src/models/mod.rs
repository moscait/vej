mod download;

pub use download::pull_model;

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A catalog model. Downloads are pinned to one Hugging Face commit (`revision`) and verified
/// against `sha256`, so a change to the upstream repository can never swap the bytes vej loads.
#[derive(Debug, Clone, Serialize)]
pub struct ModelSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub vendor: &'static str,
    pub country: &'static str,
    pub size_gb: f32,
    pub aliases: &'static [&'static str],
    /// Hugging Face repository, `owner/name`.
    pub repo: &'static str,
    /// Full commit SHA of the repository revision the file is fetched from.
    pub revision: &'static str,
    pub filename: &'static str,
    /// Lowercase hex SHA-256 of the GGUF file (the Git LFS object id).
    pub sha256: &'static str,
    pub template: TemplateStyle,
}

impl ModelSpec {
    /// Immutable download URL for this exact file at this exact revision.
    pub fn download_url(&self) -> String {
        format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            self.repo, self.revision, self.filename
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TemplateStyle {
    Llama3,
    ChatML,
    Gemma,
    Gemma4,
}

impl TemplateStyle {
    /// Names accepted by [`std::str::FromStr`] (case-insensitive) and used in config files.
    pub const NAMES: [&'static str; 4] = ["Llama3", "ChatML", "Gemma", "Gemma4"];
}

impl std::str::FromStr for TemplateStyle {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "llama3" => Ok(Self::Llama3),
            "chatml" => Ok(Self::ChatML),
            "gemma" => Ok(Self::Gemma),
            "gemma4" => Ok(Self::Gemma4),
            other => Err(anyhow!(
                "unknown template '{other}'; expected one of {}",
                Self::NAMES.join(", ")
            )),
        }
    }
}

// Revisions and hashes come from the Hugging Face API:
//   https://huggingface.co/api/models/<repo>              -> .sha
//   https://huggingface.co/api/models/<repo>/tree/<sha>   -> entry for <filename> -> .lfs.oid
// Sizes are decimal gigabytes of the pinned file.
pub static RECOMMENDED_MODELS: &[ModelSpec] = &[
    ModelSpec {
        id: "llama-3.2-1b",
        name: "Llama-3.2-1B-Instruct",
        vendor: "Meta",
        country: "USA",
        size_gb: 0.81,
        aliases: &[
            "llama-3.2-1b",
            "llama3.2-1b",
            "llama-1b",
            "llama3.2",
            "meta-1b",
        ],
        repo: "bartowski/Llama-3.2-1B-Instruct-GGUF",
        revision: "067b946cf014b7c697f3654f621d577a3e3afd1c",
        filename: "Llama-3.2-1B-Instruct-Q4_K_M.gguf",
        sha256: "6f85a640a97cf2bf5b8e764087b1e83da0fdb51d7c9fab7d0fece9385611df83",
        template: TemplateStyle::Llama3,
    },
    ModelSpec {
        id: "smollm2-1.7b",
        name: "SmolLM2-1.7B-Instruct",
        vendor: "Hugging Face",
        country: "USA / France",
        size_gb: 1.06,
        aliases: &["smollm2-1.7b", "smollm2", "smollm", "hf-1.7b"],
        repo: "bartowski/SmolLM2-1.7B-Instruct-GGUF",
        revision: "1f03464768bfcc0319fc50da8ff5fb20b6417ba2",
        filename: "SmolLM2-1.7B-Instruct-Q4_K_M.gguf",
        sha256: "77665ea4815999596525c636fbeb56ba8b080b46ae85efef4f0d986a139834d7",
        template: TemplateStyle::ChatML,
    },
    ModelSpec {
        id: "llama-3.2-3b",
        name: "Llama-3.2-3B-Instruct",
        vendor: "Meta",
        country: "USA",
        size_gb: 2.02,
        aliases: &["llama-3.2-3b", "llama3.2-3b", "llama-3b", "meta-3b"],
        repo: "bartowski/Llama-3.2-3B-Instruct-GGUF",
        revision: "5ab33fa94d1d04e903623ae72c95d1696f09f9e8",
        filename: "Llama-3.2-3B-Instruct-Q4_K_M.gguf",
        sha256: "6c1a2b41161032677be168d354123594c0e6e67d2b9227c84f296ad037c728ff",
        template: TemplateStyle::Llama3,
    },
    ModelSpec {
        id: "llama-3.1-8b",
        name: "Llama-3.1-8B-Instruct",
        vendor: "Meta",
        country: "USA",
        size_gb: 4.92,
        aliases: &["llama-3.1-8b", "llama3.1-8b", "llama-8b", "meta-8b"],
        repo: "bartowski/Meta-Llama-3.1-8B-Instruct-GGUF",
        revision: "bf5b95e96dac0462e2a09145ec66cae9a3f12067",
        filename: "Meta-Llama-3.1-8B-Instruct-Q4_K_M.gguf",
        sha256: "7b064f5842bf9532c91456deda288a1b672397a54fa729aa665952863033557c",
        template: TemplateStyle::Llama3,
    },
    ModelSpec {
        id: "granite-4.2-3b",
        name: "Granite-4.2-3B-Instruct",
        vendor: "IBM",
        country: "USA",
        size_gb: 2.24,
        aliases: &[
            "granite-4.2-3b",
            "granite-4.2",
            "granite4.2",
            "granite4",
            "granite",
            "ibm-granite",
            "ibm-4.2",
        ],
        repo: "ibm-granite/granite-4.2-3b-GGUF",
        revision: "c40945d71cd90f249a56985e8155551a9188dc30",
        filename: "granite-4.2-3b-Q4_K_M.gguf",
        sha256: "e0406663965846ae22a403456eb826ccce5f450840491f71952f18a7cb78e7d5",
        template: TemplateStyle::ChatML,
    },
    ModelSpec {
        id: "gemma-4-e2b",
        name: "Gemma-4-E2B-IT",
        vendor: "Google",
        country: "USA",
        size_gb: 3.11,
        aliases: &[
            "gemma-4-e2b",
            "gemma4-e2b",
            "gemma-4",
            "gemma4",
            "gemma-4-2b",
            "google-4-e2b",
        ],
        repo: "unsloth/gemma-4-E2B-it-GGUF",
        revision: "0314792d7f1f7e229411f620751375812bb9faf2",
        filename: "gemma-4-E2B-it-Q4_K_M.gguf",
        sha256: "740185b21d22ceb83a11c3aa62ad5842ef32c70f6096d756bbee85a1e4ec34b8",
        template: TemplateStyle::Gemma4,
    },
];

pub fn get_cache_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("VEJ_CACHE_DIR") {
        PathBuf::from(dir)
    } else if let Some(cache_dir) = dirs::cache_dir() {
        cache_dir.join("vej").join("models")
    } else {
        PathBuf::from(".vej").join("models")
    }
}

pub fn find_model_spec(query: &str) -> Option<&'static ModelSpec> {
    let q = query.to_lowercase().trim().to_string();
    RECOMMENDED_MODELS.iter().find(|m| {
        m.id.eq_ignore_ascii_case(&q)
            || m.name.eq_ignore_ascii_case(&q)
            || m.aliases.iter().any(|a| a.eq_ignore_ascii_case(&q))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_lower_hex(s: &str) -> bool {
        s.chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    }

    #[test]
    fn every_catalog_entry_pins_a_commit_and_a_sha256() {
        for m in RECOMMENDED_MODELS {
            assert_eq!(
                m.revision.len(),
                40,
                "{}: revision must be a full commit SHA",
                m.id
            );
            assert!(
                is_lower_hex(m.revision),
                "{}: revision must be lowercase hex",
                m.id
            );
            assert_eq!(m.sha256.len(), 64, "{}: sha256 must be 64 hex chars", m.id);
            assert!(
                is_lower_hex(m.sha256),
                "{}: sha256 must be lowercase hex",
                m.id
            );
            assert_eq!(
                m.repo.split('/').count(),
                2,
                "{}: repo must be owner/name",
                m.id
            );
            assert!(
                m.filename.ends_with(".gguf"),
                "{}: filename must be a GGUF",
                m.id
            );
            assert!(
                !m.filename.contains('/'),
                "{}: filename must be a bare name",
                m.id
            );
        }
    }

    #[test]
    fn download_url_is_pinned_to_the_revision_not_main() {
        let spec = find_model_spec("llama-3.2-1b").expect("catalog model");
        let url = spec.download_url();
        assert_eq!(
            url,
            format!(
                "https://huggingface.co/{}/resolve/{}/{}",
                spec.repo, spec.revision, spec.filename
            )
        );
        assert!(
            !url.contains("/resolve/main/"),
            "url must not float on main"
        );
    }

    #[test]
    fn catalog_ids_and_filenames_are_unique() {
        let mut ids: Vec<&str> = RECOMMENDED_MODELS.iter().map(|m| m.id).collect();
        let mut files: Vec<&str> = RECOMMENDED_MODELS.iter().map(|m| m.filename).collect();
        ids.sort_unstable();
        files.sort_unstable();
        ids.dedup();
        files.dedup();
        assert_eq!(ids.len(), RECOMMENDED_MODELS.len());
        assert_eq!(files.len(), RECOMMENDED_MODELS.len());
    }
}
