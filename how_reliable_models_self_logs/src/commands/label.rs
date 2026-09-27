//! `label`: deterministic labels (harm) for every unlabelled, non-failed episode.

use anyhow::Result;
use sqlx::PgPool;

use crate::labels::harm;

/// Label the given runs (all runs if none).
pub async fn label(pool: &PgPool, runs: &[String]) -> Result<()> {
    let (n, harmful) = harm::label(pool, runs).await?;
    println!("label: harm labels for {n} episodes ({harmful} harmful)");
    Ok(())
}
