//! Loading scenarios into the database. A scenario row is one content version: identical
//! files are skipped, changed files give a new row (episodes point to the exact version).

use anyhow::Result;
use sqlx::PgPool;

use super::Scenario;
use crate::types::Arm;

/// Insert the scenario if its content is new. Returns true if a row was inserted.
pub async fn insert(
    pool: &PgPool,
    s: &Scenario,
    benchmark_git_hash: &str,
    benchmark_dirty: bool,
) -> Result<bool> {
    let arms: Vec<&str> = Arm::ALL.iter().map(|a| a.as_str()).collect();
    let done = sqlx::query(
        "INSERT INTO scenarios (scenario_id, version, title, arms, self_log_tool, content_sha256, files,
                                benchmark_git_hash, benchmark_dirty)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (scenario_id, content_sha256) DO NOTHING",
    )
    .bind(&s.id)
    .bind(s.version as i32)
    .bind(&s.title)
    .bind(&arms)
    .bind(&s.self_log_tool)
    .bind(&s.content_sha256)
    .bind(sqlx::types::Json(&s.files))
    .bind(benchmark_git_hash)
    .bind(benchmark_dirty)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() == 1)
}
