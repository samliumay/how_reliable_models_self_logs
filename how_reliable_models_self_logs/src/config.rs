//! The single experiment config (`config.toml`). Same file for every model; paths inside it
//! are relative to the file's own folder.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::types::Api;
use crate::util::hash::sha256_hex;

/// Prefix of a scripted model id: `scripted:<path to script.json>`. Never in `config.toml`.
pub const SCRIPTED_PREFIX: &str = "scripted:";

/// How subject models are called (`[run]`).
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RunConfig {
    /// Sampling temperature for subject models.
    pub temperature: f64,
    /// Output budget per model call (one turn), reasoning included.
    pub max_tokens: u32,
    /// Samples per scenario × arm (a run may override it).
    pub samples: u32,
    /// Episodes in flight at once.
    pub concurrency: usize,
    /// Retries of one model call after the first attempt.
    pub max_retries: u32,
    /// Base seed; every call of sample `s` is sent `seed + s`.
    pub seed: Option<u64>,
    /// Timeout of one call, in seconds.
    pub request_timeout_s: u64,
}

/// The LLM judges (`[judge]`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JudgeConfig {
    /// Gemini model id (3.1 or newer).
    pub model: String,
    /// Judge calls in flight at once.
    pub concurrency: usize,
    /// Retries after the first attempt.
    pub max_retries: u32,
    /// Output budget of one judge call (thinking included).
    pub max_output_tokens: u32,
}

/// Paths relative to `config.toml` (`[paths]`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathsConfig {
    /// The benchmark repo (scenarios in `scenarios/`, judge prompts in `labels/`).
    pub benchmark: PathBuf,
    /// Where `report` writes run folders.
    pub results: PathBuf,
    /// The `.env` with database URL and API keys.
    pub env_file: PathBuf,
}

/// The Ollama server (`[ollama]`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OllamaConfig {
    /// Server address, e.g. `http://localhost:11434`.
    pub base_url: String,
}

/// A `[[models]]` entry as written; checked into a `ModelConfig`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawModel {
    /// Model id at its API.
    id: String,
    /// `openrouter` or `ollama`.
    api: String,
    /// OpenRouter provider tag (OpenRouter only).
    provider: Option<String>,
    /// Ollama manifest digest (Ollama only).
    digest: Option<String>,
}

/// Where a model is served and how it is pinned.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelSource {
    /// One OpenRouter endpoint, fallbacks off.
    OpenRouter {
        /// Provider tag, e.g. `deepinfra/bf16`.
        provider: String,
    },
    /// The Ollama manifest digest the tag must still point to.
    Ollama {
        /// Full sha256 digest from `ollama list` / `/api/tags`.
        digest: String,
    },
    /// Canned replies (tests and dry runs); pinned by the script file's sha256.
    Scripted {
        /// The script file.
        script: PathBuf,
    },
}

/// A subject model and how it is pinned.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelConfig {
    /// Model id at its API (for scripted models: `scripted:<path>`).
    pub id: String,
    /// API and pin.
    pub source: ModelSource,
}

impl ModelConfig {
    /// Which API serves the model.
    pub fn api(&self) -> Api {
        match self.source {
            ModelSource::OpenRouter { .. } => Api::OpenRouter,
            ModelSource::Ollama { .. } => Api::Ollama,
            ModelSource::Scripted { .. } => Api::Scripted,
        }
    }
}

impl TryFrom<RawModel> for ModelConfig {
    type Error = anyhow::Error;
    fn try_from(r: RawModel) -> Result<ModelConfig> {
        let source = match (r.api.parse::<Api>()?, r.provider, r.digest) {
            (Api::OpenRouter, Some(provider), None) => ModelSource::OpenRouter { provider },
            (Api::Ollama, None, Some(digest)) => ModelSource::Ollama { digest },
            (Api::OpenRouter, _, _) => bail!(
                "{}: api = \"openrouter\" needs `provider` and no `digest`",
                r.id
            ),
            (Api::Ollama, _, _) => bail!(
                "{}: api = \"ollama\" needs `digest` and no `provider`",
                r.id
            ),
            (Api::Scripted, _, _) => bail!(
                "{}: scripted models are not listed in config.toml; use --model {SCRIPTED_PREFIX}<script.json>",
                r.id
            ),
        };
        Ok(ModelConfig { id: r.id, source })
    }
}

/// The whole file as written.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    /// `[run]`.
    run: RunConfig,
    /// `[judge]`.
    judge: JudgeConfig,
    /// `[paths]`.
    paths: PathsConfig,
    /// `[ollama]`.
    ollama: OllamaConfig,
    /// `[[models]]`.
    models: Vec<RawModel>,
}

/// The loaded config, with paths resolved and the file's hash.
#[derive(Debug)]
pub struct Config {
    /// `[run]`.
    pub run: RunConfig,
    /// `[judge]`.
    pub judge: JudgeConfig,
    /// `[ollama]`.
    pub ollama: OllamaConfig,
    /// `[[models]]`, in file order.
    pub models: Vec<ModelConfig>,
    /// Resolved `paths.benchmark`.
    pub benchmark_dir: PathBuf,
    /// Resolved `paths.results`.
    pub results_dir: PathBuf,
    /// Resolved `paths.env_file`.
    pub env_file: PathBuf,
    /// Folder that holds `config.toml`; the crate root.
    pub crate_dir: PathBuf,
    /// The file as written, stored with every run.
    pub text: String,
    /// sha256 of `text`.
    pub sha256: String,
}

impl Config {
    /// Read, validate and resolve `config.toml`.
    pub fn load(path: &Path) -> Result<Config> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let crate_dir = path
            .canonicalize()?
            .parent()
            .context("config has no parent folder")?
            .to_path_buf();
        Config::from_text(text, crate_dir).with_context(|| format!("in {}", path.display()))
    }

    /// Parse and validate config text (e.g. the copy stored with a run).
    pub fn from_text(text: String, crate_dir: PathBuf) -> Result<Config> {
        let raw: Raw = toml::from_str(&text).context("parsing config")?;
        if raw.models.is_empty() {
            bail!("config lists no models");
        }
        if raw.run.samples == 0 || raw.run.concurrency == 0 || raw.judge.concurrency == 0 {
            bail!("samples and concurrency must be at least 1");
        }
        // Resolve `..` where the path exists, so printed paths are readable.
        let resolve = |p: &Path| {
            let joined = crate_dir.join(p);
            joined.canonicalize().unwrap_or(joined)
        };
        Ok(Config {
            benchmark_dir: resolve(&raw.paths.benchmark),
            results_dir: resolve(&raw.paths.results),
            env_file: resolve(&raw.paths.env_file),
            sha256: sha256_hex(text.as_bytes()),
            run: raw.run,
            judge: raw.judge,
            ollama: raw.ollama,
            models: raw
                .models
                .into_iter()
                .map(ModelConfig::try_from)
                .collect::<Result<_>>()?,
            crate_dir,
            text,
        })
    }

    /// Folder of the benchmark's scenarios.
    pub fn scenarios_dir(&self) -> PathBuf {
        self.benchmark_dir.join("scenarios")
    }

    /// The model with this id, or an error listing the known ones. `scripted:<path>` ids
    /// resolve to a scripted model without a config entry.
    pub fn model(&self, id: &str) -> Result<ModelConfig> {
        if let Some(path) = id.strip_prefix(SCRIPTED_PREFIX) {
            return Ok(ModelConfig {
                id: id.to_string(),
                source: ModelSource::Scripted {
                    script: PathBuf::from(path),
                },
            });
        }
        match self.models.iter().find(|m| m.id == id) {
            Some(m) => Ok(m.clone()),
            None => {
                let known: Vec<&str> = self.models.iter().map(|m| m.id.as_str()).collect();
                bail!(
                    "model {id:?} is not in config.toml (models: {})",
                    known.join(", ")
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The committed config.toml must parse.
    #[test]
    fn committed_config_parses() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let cfg = Config::load(&dir.join("config.toml")).unwrap();
        assert_eq!(cfg.run.temperature, 1.0);
        assert!(!cfg.models.is_empty());
    }

    #[test]
    fn scripted_ids_resolve_without_an_entry() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let cfg = Config::load(&dir.join("config.toml")).unwrap();
        let m = cfg.model("scripted:tests/fixtures/script.json").unwrap();
        assert_eq!(m.api(), Api::Scripted);
        assert!(cfg.model("no/such-model").is_err());
    }
}
