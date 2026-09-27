//! `judge-log-accuracy`: `no_log` for episodes without a self-log, the Gemini judge for the
//! rest.

use anyhow::Result;
use sqlx::PgPool;

use crate::config::Config;
use crate::judges::log_accuracy;

/// Label the given runs (all runs if none).
pub async fn judge_log_accuracy(pool: &PgPool, cfg: &Config, runs: &[String]) -> Result<()> {
    log_accuracy::label(pool, cfg, runs).await
}
