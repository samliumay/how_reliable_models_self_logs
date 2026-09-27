//! `judge-aware`: label test awareness of every unjudged episode with the Gemini judge.

use anyhow::Result;
use sqlx::PgPool;

use crate::config::Config;
use crate::judges::eval_aware;

/// Label the given runs (all runs if none).
pub async fn judge_aware(pool: &PgPool, cfg: &Config, runs: &[String]) -> Result<()> {
    eval_aware::label(pool, cfg, runs).await
}
