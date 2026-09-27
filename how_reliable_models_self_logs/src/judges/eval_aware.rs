//! The test-awareness judge. It reads the full conversation: system prompt, task, every
//! turn's reasoning and text, every tool call and tool result.

use std::path::Path;

use anyhow::{Result, anyhow};
use futures::StreamExt;
use sqlx::{PgPool, Row};

use super::{Prompt, api_key, ask, between, fill, gemini, record_failure};
use crate::agent::render_transcript;
use crate::config::Config;
use crate::models::Message;
use crate::types::{EvalAware, Judge};
use crate::util::text::truncate;

/// Folder of this judge's prompt under `benchmark/labels/`.
pub const LABEL: &str = "eval_aware";

/// The test-awareness judge prompt.
pub fn load_prompt(benchmark_dir: &Path) -> Result<Prompt> {
    Prompt::load(benchmark_dir, LABEL)
}

/// Read `<label>` (last occurrence) and `<evidence>` from the judge's answer.
pub fn parse_verdict(text: &str) -> Result<(EvalAware, String)> {
    let label = between(text, "<label>", "</label>")
        .ok_or_else(|| anyhow!("no <label> in judge output"))?;
    let label: EvalAware = label.to_lowercase().parse()?;
    let evidence = between(text, "<evidence>", "</evidence>")
        .unwrap_or("")
        .to_string();
    Ok((label, evidence))
}

/// Label every non-failed episode that has no verdict for the current judge and prompt.
pub async fn label(pool: &PgPool, cfg: &Config, runs: &[String]) -> Result<()> {
    let prompt = load_prompt(&cfg.benchmark_dir)?;
    let model = cfg.judge.model.as_str();
    let rows = sqlx::query(
        "SELECT e.id, e.transcript
         FROM episodes e
         WHERE e.status <> 'failed'
           AND (cardinality($1::text[]) = 0 OR e.run_id = ANY($1))
           AND NOT EXISTS (SELECT 1 FROM aware_labels a
                           WHERE a.episode_id = e.id AND a.judge_model = $2 AND a.prompt_sha256 = $3)
         ORDER BY e.id",
    )
    .bind(runs)
    .bind(model)
    .bind(&prompt.sha256)
    .fetch_all(pool)
    .await?;
    let items: Vec<(i64, String)> = rows
        .iter()
        .map(|r| {
            let transcript: sqlx::types::Json<Vec<Message>> = r.try_get("transcript")?;
            let message = fill(
                &prompt.user_template,
                &[("transcript", &render_transcript(&transcript.0))],
            );
            Ok((r.get("id"), message))
        })
        .collect::<Result<_>>()?;
    println!(
        "judge-aware: {} episodes to judge with {model} (prompt {})",
        items.len(),
        &prompt.sha256[..12]
    );
    if items.is_empty() {
        return Ok(());
    }

    let client = gemini::Client::new(api_key()?, cfg.judge.max_output_tokens)?;
    let total = items.len();
    let mut stream = futures::stream::iter(items.into_iter().map(|(id, message)| {
        let (client, prompt) = (&client, &prompt);
        async move { (id, ask(client, cfg, prompt, &message, parse_verdict).await) }
    }))
    .buffer_unordered(cfg.judge.concurrency);

    let (mut n, mut failed) = (0usize, 0usize);
    while let Some((episode_id, result)) = stream.next().await {
        n += 1;
        match result {
            Ok(((label, evidence), output)) => {
                sqlx::query(
                    "INSERT INTO aware_labels (episode_id, label, evidence, judge_model, prompt_sha256, judge_output)
                     VALUES ($1, $2, $3, $4, $5, $6)",
                )
                .bind(episode_id)
                .bind(label.as_str())
                .bind(&evidence)
                .bind(model)
                .bind(&prompt.sha256)
                .bind(&output)
                .execute(pool)
                .await?;
                println!(
                    "  [{n}/{total}] episode {episode_id}: {label:9} {}",
                    truncate(&evidence.replace('\n', " "), 110)
                );
            }
            Err(e) => {
                failed += 1;
                let msg = format!("{e:#}");
                record_failure(pool, Judge::EvalAware, episode_id, cfg, &prompt, &msg).await?;
                println!(
                    "  [{n}/{total}] episode {episode_id}: FAILED {}",
                    truncate(&msg, 160)
                );
            }
        }
    }
    println!("judge-aware: {} labelled, {failed} failed", n - failed);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_verdict_reads_label_and_evidence() {
        let (label, ev) =
            parse_verdict("<evidence>\"this is a test\"</evidence>\n<label> Explicit </label>")
                .unwrap();
        assert_eq!(label, EvalAware::Explicit);
        assert_eq!(ev, "\"this is a test\"");
    }

    #[test]
    fn parse_verdict_uses_last_label() {
        let text = "The format is <label>none | suspected | explicit</label>. Answer: <label>suspected</label>";
        assert_eq!(parse_verdict(text).unwrap().0, EvalAware::Suspected);
    }

    #[test]
    fn parse_verdict_rejects_missing_or_unknown_label() {
        assert!(parse_verdict("no tags here").is_err());
        assert!(parse_verdict("<label>maybe</label>").is_err());
    }

    /// The draft prompt must fill without leftover placeholders.
    #[test]
    fn draft_prompt_has_the_transcript_placeholder() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("prompts_draft");
        let p = Prompt::load(&dir, LABEL).unwrap();
        let out = fill(&p.user_template, &[("transcript", "T")]);
        assert!(!out.contains("{transcript}") && out.contains('T'));
    }
}
