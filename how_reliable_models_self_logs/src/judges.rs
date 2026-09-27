//! LLM judges: the pieces every judge shares (prompt loading, template fill, tag parsing,
//! the retry loop, failure records). One file per judge in `judges/`; `gemini.rs` is the API
//! client. Judges run at temperature 0; every verdict stores the judge model, the prompt
//! hash and the judge's full output (its reasoning).

pub mod eval_aware;
pub mod gemini;
pub mod log_accuracy;

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use sqlx::PgPool;

use crate::config::Config;
use crate::types::Judge;
use crate::util::hash::sha256_parts;
use crate::util::text::truncate;

/// A judge prompt, read from `benchmark/labels/<label>/`.
pub struct Prompt {
    /// `system.md`: the judge's instructions.
    pub system: String,
    /// `user.md`: the message template with `{placeholders}`.
    pub user_template: String,
    /// Hash of both files; stored with every verdict as the prompt version.
    pub sha256: String,
}

impl Prompt {
    /// Read both files of `benchmark/labels/<label>/` and hash them.
    pub fn load(benchmark_dir: &Path, label: &str) -> Result<Prompt> {
        let dir = benchmark_dir.join("labels").join(label);
        let read = |f: &str| {
            std::fs::read_to_string(dir.join(f))
                .with_context(|| format!("reading {}", dir.join(f).display()))
        };
        let system = read("system.md")?;
        let user_template = read("user.md")?;
        let sha256 = sha256_parts(&[&system, &user_template]);
        Ok(Prompt {
            system,
            user_template,
            sha256,
        })
    }
}

/// Replace `{name}` placeholders in one left-to-right pass, so text inserted for one
/// placeholder (a model answer containing "{answer}", say) is never substituted again.
pub fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    'outer: while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        for (name, value) in values {
            let key = format!("{{{name}}}");
            if after.starts_with(&key) {
                out.push_str(value);
                rest = &after[key.len()..];
                continue 'outer;
            }
        }
        out.push('{');
        rest = &after[1..];
    }
    out.push_str(rest);
    out
}

/// Text between the last `open` and the next `close`, trimmed.
pub fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = text.rfind(open)? + open.len();
    let end = text[start..].find(close)? + start;
    Some(text[start..end].trim())
}

/// One judge call; retries HTTP errors and unparseable answers. Returns the parsed verdict
/// and the judge's full output.
pub async fn ask<T>(
    client: &gemini::Client,
    cfg: &Config,
    prompt: &Prompt,
    message: &str,
    parse: impl Fn(&str) -> Result<T>,
) -> Result<(T, String)> {
    let mut last = anyhow!("no attempt made");
    for attempt in 0..=cfg.judge.max_retries {
        match client
            .generate(&cfg.judge.model, &prompt.system, message)
            .await
        {
            Ok(text) => match parse(&text) {
                Ok(verdict) => return Ok((verdict, text)),
                Err(e) => last = e.context(format!("judge output: {}", truncate(&text, 200))),
            },
            Err(e) => last = e,
        }
        if attempt < cfg.judge.max_retries {
            tokio::time::sleep(Duration::from_secs((3 * 2u64.pow(attempt)).min(90))).await;
        }
    }
    Err(last)
}

/// Store a judge call that failed after all retries.
pub async fn record_failure(
    pool: &PgPool,
    judge: Judge,
    episode_id: i64,
    cfg: &Config,
    prompt: &Prompt,
    error: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO judge_failures (episode_id, judge, judge_model, prompt_sha256, error) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(episode_id)
    .bind(judge.as_str())
    .bind(&cfg.judge.model)
    .bind(&prompt.sha256)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

/// `GEMINI_API_KEY`, read only when there is something to judge.
pub fn api_key() -> Result<String> {
    std::env::var("GEMINI_API_KEY").context("GEMINI_API_KEY is not set (add it to ../.env)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_does_not_resubstitute_inserted_text() {
        let out = fill("A={a} B={b} C={c}", &[("a", "{b}"), ("b", "x")]);
        assert_eq!(out, "A={b} B=x C={c}");
    }

    #[test]
    fn between_takes_the_last_open_tag() {
        assert_eq!(between("<a>1</a> <a> 2 </a>", "<a>", "</a>"), Some("2"));
        assert_eq!(between("<a>1", "<a>", "</a>"), None);
    }
}
