use anyhow::{Context, Result, anyhow};
use indicatif::{ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct ModelSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub vendor: &'static str,
    pub country: &'static str,
    pub size_gb: f32,
    pub aliases: &'static [&'static str],
    pub hf_url: &'static str,
    pub filename: &'static str,
    pub template: TemplateStyle,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TemplateStyle {
    Llama3,
    ChatML,
    Gemma,
    Gemma4,
}

pub static RECOMMENDED_MODELS: &[ModelSpec] = &[
    ModelSpec {
        id: "llama-3.2-1b",
        name: "Llama-3.2-1B-Instruct",
        vendor: "Meta",
        country: "USA",
        size_gb: 0.75,
        aliases: &[
            "llama-3.2-1b",
            "llama3.2-1b",
            "llama-1b",
            "llama3.2",
            "meta-1b",
        ],
        hf_url: "https://huggingface.co/bartowski/Llama-3.2-1B-Instruct-GGUF/resolve/main/Llama-3.2-1B-Instruct-Q4_K_M.gguf",
        filename: "Llama-3.2-1B-Instruct-Q4_K_M.gguf",
        template: TemplateStyle::Llama3,
    },
    ModelSpec {
        id: "smollm2-1.7b",
        name: "SmolLM2-1.7B-Instruct",
        vendor: "Hugging Face",
        country: "USA / France",
        size_gb: 1.05,
        aliases: &["smollm2-1.7b", "smollm2", "smollm", "hf-1.7b"],
        hf_url: "https://huggingface.co/bartowski/SmolLM2-1.7B-Instruct-GGUF/resolve/main/SmolLM2-1.7B-Instruct-Q4_K_M.gguf",
        filename: "SmolLM2-1.7B-Instruct-Q4_K_M.gguf",
        template: TemplateStyle::ChatML,
    },
    ModelSpec {
        id: "llama-3.2-3b",
        name: "Llama-3.2-3B-Instruct",
        vendor: "Meta",
        country: "USA",
        size_gb: 2.02,
        aliases: &["llama-3.2-3b", "llama3.2-3b", "llama-3b", "meta-3b"],
        hf_url: "https://huggingface.co/bartowski/Llama-3.2-3B-Instruct-GGUF/resolve/main/Llama-3.2-3B-Instruct-Q4_K_M.gguf",
        filename: "Llama-3.2-3B-Instruct-Q4_K_M.gguf",
        template: TemplateStyle::Llama3,
    },
    ModelSpec {
        id: "llama-3.1-8b",
        name: "Llama-3.1-8B-Instruct",
        vendor: "Meta",
        country: "USA",
        size_gb: 4.92,
        aliases: &["llama-3.1-8b", "llama3.1-8b", "llama-8b", "meta-8b"],
        hf_url: "https://huggingface.co/bartowski/Meta-Llama-3.1-8B-Instruct-GGUF/resolve/main/Meta-Llama-3.1-8B-Instruct-Q4_K_M.gguf",
        filename: "Meta-Llama-3.1-8B-Instruct-Q4_K_M.gguf",
        template: TemplateStyle::Llama3,
    },
    ModelSpec {
        id: "granite-4.2-3b",
        name: "Granite-4.2-3B-Instruct",
        vendor: "IBM",
        country: "USA",
        size_gb: 2.09,
        aliases: &[
            "granite-4.2-3b",
            "granite-4.2",
            "granite4.2",
            "granite4",
            "granite",
            "ibm-granite",
            "ibm-4.2",
        ],
        hf_url: "https://huggingface.co/ibm-granite/granite-4.2-3b-GGUF/resolve/main/granite-4.2-3b-Q4_K_M.gguf",
        filename: "granite-4.2-3b-Q4_K_M.gguf",
        template: TemplateStyle::ChatML,
    },
    ModelSpec {
        id: "gemma-4-e2b",
        name: "Gemma-4-E2B-IT",
        vendor: "Google",
        country: "USA",
        size_gb: 1.85,
        aliases: &[
            "gemma-4-e2b",
            "gemma4-e2b",
            "gemma-4",
            "gemma4",
            "gemma-4-2b",
            "google-4-e2b",
        ],
        hf_url: "https://huggingface.co/unsloth/gemma-4-E2B-it-GGUF/resolve/main/gemma-4-E2B-it-Q4_K_M.gguf",
        filename: "gemma-4-E2B-it-Q4_K_M.gguf",
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

pub fn resolve_model_path(query: &str) -> Result<PathBuf> {
    let p = Path::new(query);
    if p.exists() {
        return Ok(p.to_path_buf());
    }

    if let Some(spec) = find_model_spec(query) {
        let cached = get_cache_dir().join(spec.filename);
        if cached.exists() {
            return Ok(cached);
        }
        return Err(anyhow!(
            "Model '{}' ({}) is recognized but not yet downloaded.\nRun `vej models pull {}` to download it to {:?}",
            spec.id,
            spec.name,
            spec.id,
            cached
        ));
    }

    Err(anyhow!(
        "Model '{}' was not found as a local file or in known presets.\nAvailable presets: {}",
        query,
        RECOMMENDED_MODELS
            .iter()
            .map(|m| m.id)
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

pub async fn pull_model(query: &str) -> Result<PathBuf> {
    let spec = find_model_spec(query).ok_or_else(|| {
        anyhow!(
            "Unknown model '{}'. Supported presets are: {}",
            query,
            RECOMMENDED_MODELS
                .iter()
                .map(|m| m.id)
                .collect::<Vec<_>>()
                .join(", ")
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

    let temp_file = dest_dir.join(format!("{}.tmp", spec.filename));

    println!(
        "Downloading {} ({} - ~{:.2} GB) from Hugging Face...",
        spec.name, spec.country, spec.size_gb
    );
    println!("Source: {}", spec.hf_url);

    let client = reqwest::Client::new();
    let res = client
        .get(spec.hf_url)
        .send()
        .await
        .with_context(|| format!("Failed to connect to {}", spec.hf_url))?;

    if !res.status().is_success() {
        return Err(anyhow!(
            "Download failed with HTTP status {}: {}",
            res.status(),
            spec.hf_url
        ));
    }

    let total_size = res.content_length().unwrap_or(0);
    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")?
            .progress_chars("#>-"),
    );

    let mut file = File::create(&temp_file)
        .with_context(|| format!("Failed to create temporary file {:?}", temp_file))?;

    let mut res = res;
    while let Some(chunk) = res
        .chunk()
        .await
        .with_context(|| "Error reading chunk from stream")?
    {
        file.write_all(&chunk)
            .with_context(|| "Error writing to temporary file")?;
        pb.inc(chunk.len() as u64);
    }

    pb.finish_with_message("Download complete");
    std::fs::rename(&temp_file, &dest_file)
        .with_context(|| format!("Failed to move {:?} to {:?}", temp_file, dest_file))?;

    println!("Model successfully saved to {:?}", dest_file);
    Ok(dest_file)
}
