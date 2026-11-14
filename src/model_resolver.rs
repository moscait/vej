//! Maps a model query (catalog id, alias, `[models.<name>]` override, or file path) to the
//! GGUF file to load and the chat template to use.

use anyhow::{Result, anyhow};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::models::{RECOMMENDED_MODELS, TemplateStyle, find_model_spec, get_cache_dir};

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedModel {
    /// Canonical name: the catalog id, the override name as configured, or the path as given.
    pub id: String,
    pub path: PathBuf,
    /// `None` leaves the engine to guess the template from the name.
    pub template: Option<TemplateStyle>,
}

#[derive(Debug, Clone)]
struct Override {
    /// The `[models.<key>]` header as written, for messages.
    key: String,
    /// Canonical name: the catalog id, or the key itself for a custom model.
    name: String,
    /// Absolute, or resolved against the config directory.
    path: Option<PathBuf>,
    template: Option<TemplateStyle>,
}

/// Built once from the config; cheap to clone.
#[derive(Debug, Clone)]
pub struct ModelResolver {
    /// Keyed by the lowercase canonical name.
    overrides: HashMap<String, Override>,
    cache_dir: PathBuf,
}

impl ModelResolver {
    /// Catalog models and file paths only, with no config overrides.
    pub fn catalog_only() -> Self {
        Self {
            overrides: HashMap::new(),
            cache_dir: get_cache_dir(),
        }
    }

    pub(crate) fn from_config(config: &Config) -> Result<Self> {
        Self::new(config, get_cache_dir())
    }

    pub(crate) fn new(config: &Config, cache_dir: PathBuf) -> Result<Self> {
        config.validate_model_overrides()?;
        let overrides = config
            .models
            .iter()
            .map(|(key, entry)| {
                let name = find_model_spec(key)
                    .map(|spec| spec.id.to_string())
                    .unwrap_or_else(|| key.trim().to_string());
                let path = entry
                    .path
                    .as_deref()
                    .map(|p| against_config_dir(p, config.config_dir.as_deref()));
                let over = Override {
                    key: key.clone(),
                    name: name.clone(),
                    path,
                    template: entry.template,
                };
                (name.to_lowercase(), over)
            })
            .collect();
        Ok(Self {
            overrides,
            cache_dir,
        })
    }

    /// The name a model is tracked under: catalog aliases collapse to the catalog id and
    /// custom names to their configured spelling; anything else passes through.
    pub fn canonical_id(&self, query: &str) -> String {
        if let Some(spec) = find_model_spec(query) {
            return spec.id.to_string();
        }
        match self.overrides.get(&query.trim().to_lowercase()) {
            Some(over) => over.name.clone(),
            None => query.to_string(),
        }
    }

    pub fn resolve(&self, query: &str) -> Result<ResolvedModel> {
        let as_path = Path::new(query);
        if as_path.is_file() {
            return Ok(ResolvedModel {
                id: query.to_string(),
                path: as_path.to_path_buf(),
                template: None,
            });
        }

        let id = self.canonical_id(query);
        let spec = find_model_spec(&id);
        let over = self.overrides.get(&id.to_lowercase());
        let template = over
            .and_then(|o| o.template)
            .or_else(|| spec.map(|s| s.template));

        if let Some(path) = over.and_then(|o| o.path.as_deref()) {
            if !path.is_file() {
                let key = over.map(|o| o.key.as_str()).unwrap_or(&id);
                return Err(anyhow!(
                    "Model '{id}' is configured in [models.{key}] with path {:?}, but that \
                     file does not exist",
                    path
                ));
            }
            return Ok(ResolvedModel {
                id,
                path: path.to_path_buf(),
                template,
            });
        }

        if let Some(spec) = spec {
            let cached = self.cache_dir.join(spec.filename);
            if cached.is_file() {
                return Ok(ResolvedModel {
                    id,
                    path: cached,
                    template,
                });
            }
            return Err(anyhow!(
                "Model '{}' ({}) is recognized but not yet downloaded.\nRun `vej models pull {}` \
                 to download it to {:?}",
                spec.id,
                spec.name,
                spec.id,
                cached
            ));
        }

        Err(anyhow!(
            "Model '{query}' was not found as a local file, a catalog preset, or a \
             [models.<name>] entry.\nAvailable: {}",
            self.known_names().join(", ")
        ))
    }

    /// Catalog ids followed by custom override names.
    fn known_names(&self) -> Vec<String> {
        let mut custom: Vec<String> = self
            .overrides
            .values()
            .filter(|o| find_model_spec(&o.name).is_none())
            .map(|o| o.name.clone())
            .collect();
        custom.sort_unstable();
        RECOMMENDED_MODELS
            .iter()
            .map(|m| m.id.to_string())
            .chain(custom)
            .collect()
    }
}

fn against_config_dir(path: &Path, config_dir: Option<&Path>) -> PathBuf {
    match config_dir {
        Some(dir) if path.is_relative() => dir.join(path),
        _ => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, ModelConfig};
    use crate::models::{TemplateStyle, find_model_spec};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    fn scratch(test: &str) -> PathBuf {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("vej-resolver-{test}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    fn touch(path: &Path) {
        std::fs::write(path, b"GGUF").expect("write placeholder");
    }

    type Entry<'a> = (&'a str, Option<&'a str>, Option<TemplateStyle>);

    fn config_with(dir: &Path, entries: &[Entry]) -> Config {
        let mut config = Config {
            config_dir: Some(dir.to_path_buf()),
            ..Config::default()
        };
        for (key, path, template) in entries {
            config.models.insert(
                key.to_string(),
                ModelConfig {
                    path: path.map(PathBuf::from),
                    template: *template,
                    ..ModelConfig::default()
                },
            );
        }
        config
    }

    fn resolver(config: &Config, cache_dir: &Path) -> ModelResolver {
        ModelResolver::new(config, cache_dir.to_path_buf()).expect("valid overrides")
    }

    #[test]
    fn catalog_model_resolves_to_the_cache_file_with_its_template() {
        let dir = scratch("catalog");
        let spec = find_model_spec("llama-3.2-1b").unwrap();
        touch(&dir.join(spec.filename));

        let r = resolver(&config_with(&dir, &[]), &dir);
        let resolved = r.resolve("llama-1b").unwrap();

        assert_eq!(resolved.id, "llama-3.2-1b");
        assert_eq!(resolved.path, dir.join(spec.filename));
        assert_eq!(resolved.template, Some(TemplateStyle::Llama3));
        assert_eq!(r.canonical_id("LLAMA-1B"), "llama-3.2-1b");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn catalog_model_that_is_not_downloaded_is_an_error() {
        let dir = scratch("undownloaded");
        let r = resolver(&config_with(&dir, &[]), &dir);
        let err = r.resolve("smollm2").unwrap_err().to_string();
        assert!(err.contains("vej models pull smollm2-1.7b"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn alias_keys_are_canonicalized_and_template_only_overrides_apply() {
        let dir = scratch("alias");
        let spec = find_model_spec("llama-3.2-1b").unwrap();
        touch(&dir.join(spec.filename));

        let config = config_with(&dir, &[("llama-1b", None, Some(TemplateStyle::ChatML))]);
        let resolved = resolver(&config, &dir).resolve("llama-3.2-1b").unwrap();

        assert_eq!(resolved.path, dir.join(spec.filename));
        assert_eq!(resolved.template, Some(TemplateStyle::ChatML));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn relative_override_paths_resolve_against_the_config_directory() {
        let dir = scratch("relative");
        let cache = scratch("relative-cache");
        touch(&dir.join("q8.gguf"));

        let config = config_with(&dir, &[("llama-3.2-1b", Some("q8.gguf"), None)]);
        let resolved = resolver(&config, &cache).resolve("llama-3.2-1b").unwrap();

        assert_eq!(resolved.path, dir.join("q8.gguf"));
        assert_eq!(resolved.template, Some(TemplateStyle::Llama3));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&cache).ok();
    }

    #[test]
    fn missing_override_file_is_an_error_naming_the_path() {
        let dir = scratch("missing");
        let config = config_with(&dir, &[("llama-3.2-1b", Some("gone.gguf"), None)]);
        let err = resolver(&config, &dir)
            .resolve("llama-3.2-1b")
            .unwrap_err()
            .to_string();
        assert!(err.contains("gone.gguf"), "{err}");
        assert!(err.contains("[models.llama-3.2-1b]"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn custom_names_need_a_path() {
        let dir = scratch("custom-nopath");
        let config = config_with(&dir, &[("mistral", None, Some(TemplateStyle::ChatML))]);
        let err = ModelResolver::new(&config, dir.clone())
            .unwrap_err()
            .to_string();
        assert!(err.contains("[models.mistral]"), "{err}");
        assert!(err.contains("path"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn custom_names_resolve_case_insensitively_with_an_optional_template() {
        let dir = scratch("custom");
        touch(&dir.join("mistral.gguf"));
        let config = config_with(
            &dir,
            &[
                ("mistral", Some("mistral.gguf"), Some(TemplateStyle::ChatML)),
                ("phi", Some("mistral.gguf"), None),
            ],
        );
        let r = resolver(&config, &dir);

        let resolved = r.resolve("Mistral").unwrap();
        assert_eq!(resolved.id, "mistral");
        assert_eq!(resolved.path, dir.join("mistral.gguf"));
        assert_eq!(resolved.template, Some(TemplateStyle::ChatML));
        assert_eq!(r.canonical_id("PHI"), "phi");
        assert_eq!(r.resolve("phi").unwrap().template, None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_existing_file_path_is_used_directly() {
        let dir = scratch("file");
        let file = dir.join("any.gguf");
        touch(&file);
        let r = resolver(&config_with(&dir, &[]), &dir);
        let query = file.display().to_string();
        let resolved = r.resolve(&query).unwrap();
        assert_eq!(resolved.path, file);
        assert_eq!(resolved.template, None);
        assert_eq!(r.canonical_id(&query), query);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unknown_names_list_the_catalog_and_custom_names() {
        let dir = scratch("unknown");
        touch(&dir.join("m.gguf"));
        let config = config_with(&dir, &[("mine", Some("m.gguf"), None)]);
        let err = resolver(&config, &dir)
            .resolve("nope")
            .unwrap_err()
            .to_string();
        assert!(err.contains("llama-3.2-1b"), "{err}");
        assert!(err.contains("mine"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
