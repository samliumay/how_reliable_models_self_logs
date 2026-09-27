//! Database connection (migrations embedded at compile time and applied on connect) and the
//! queries several modules share: scenario versions, system logs, storing an episode.

use std::collections::HashMap;

use anyhow::{Context, Result};
use sqlx::postgres::PgPoolOptions;
use sqlx::types::Json;
use sqlx::{PgPool, Row};

use crate::agent::Episode;
use crate::scenario::{Files, Scenario};
use crate::tools::SystemLogEntry;
use crate::types::Arm;

/// Connect to `DATABASE_URL` and apply pending migrations.
pub async fn connect() -> Result<PgPool> {
    let url =
        std::env::var("DATABASE_URL").context("DATABASE_URL is not set (see ../.env.example)")?;
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .context("connecting to PostgreSQL (is it running? `make db`)")?;
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("applying migrations")?;
    Ok(pool)
}

/// Parse the scenario version stored in row `id` of `scenarios`.
pub async fn scenario(pool: &PgPool, id: i64) -> Result<Scenario> {
    let files: Json<Files> = sqlx::query_scalar("SELECT files FROM scenarios WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .with_context(|| format!("scenario row {id}"))?;
    Scenario::from_files(files.0).with_context(|| format!("scenario row {id}"))
}

/// The system logs of the given episodes, each in `seq` order.
pub async fn system_logs(
    pool: &PgPool,
    episodes: &[i64],
) -> Result<HashMap<i64, Vec<SystemLogEntry>>> {
    let rows = sqlx::query(
        "SELECT episode_id, turn, seq, call_id, tool, args, raw_args, result, is_error, side_effect
         FROM tool_calls WHERE episode_id = ANY($1) ORDER BY episode_id, seq",
    )
    .bind(episodes)
    .fetch_all(pool)
    .await?;
    let mut out: HashMap<i64, Vec<SystemLogEntry>> = HashMap::new();
    for r in rows {
        out.entry(r.get("episode_id"))
            .or_default()
            .push(SystemLogEntry {
                turn: r.get::<i32, _>("turn") as u32,
                seq: r.get::<i32, _>("seq") as u32,
                call_id: r.get("call_id"),
                tool: r.get("tool"),
                args: r.get::<Json<serde_json::Value>, _>("args").0,
                raw_args: r.get("raw_args"),
                result: r.get("result"),
                is_error: r.get("is_error"),
                side_effect: r.get("side_effect"),
            });
    }
    Ok(out)
}

/// Where an episode belongs.
pub struct Cell<'a> {
    /// Run id.
    pub run_id: &'a str,
    /// Scenario row (version).
    pub scenario_row: i64,
    /// Arm.
    pub arm: Arm,
    /// Sample index.
    pub sample: i32,
    /// Code commit that ran it (`-dirty` suffix if uncommitted changes).
    pub code: &'a str,
}

/// Store an episode with its turns and system log in one transaction. Returns its id.
pub async fn store_episode(pool: &PgPool, cell: &Cell<'_>, ep: &Episode) -> Result<i64> {
    let mut tx = pool.begin().await?;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO episodes (run_id, scenario_row, arm, sample, status, turns, final_answer, transcript,
                               fired_events, prompt_tokens, completion_tokens, reasoning_tokens, cost_usd,
                               duration_ms, attempts, error, code_git_hash)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
         RETURNING id",
    )
    .bind(cell.run_id)
    .bind(cell.scenario_row)
    .bind(cell.arm.as_str())
    .bind(cell.sample)
    .bind(ep.status.as_str())
    .bind(ep.turns.len() as i32)
    .bind(&ep.final_answer)
    .bind(Json(&ep.transcript))
    .bind(Json(&ep.fired))
    .bind(ep.sum(|t| t.prompt_tokens))
    .bind(ep.sum(|t| t.completion_tokens))
    .bind(ep.sum(|t| t.reasoning_tokens))
    .bind(ep.sum(|t| t.cost_usd))
    .bind(ep.duration.as_millis() as i64)
    .bind(ep.attempts() as i32)
    .bind(&ep.error)
    .bind(cell.code)
    .fetch_one(&mut *tx)
    .await?;
    for t in &ep.turns {
        sqlx::query(
            "INSERT INTO turns (episode_id, turn, status, content, reasoning, tool_calls, finish_reason, served_by,
                                api_model, prompt_tokens, completion_tokens, reasoning_tokens, cost_usd, attempts,
                                duration_ms, error, raw_response)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)",
        )
        .bind(id)
        .bind(t.turn as i32)
        .bind(t.status.as_str())
        .bind(&t.content)
        .bind(&t.reasoning)
        .bind(Json(&t.tool_calls))
        .bind(&t.finish_reason)
        .bind(&t.served_by)
        .bind(&t.api_model)
        .bind(t.prompt_tokens)
        .bind(t.completion_tokens)
        .bind(t.reasoning_tokens)
        .bind(t.cost_usd)
        .bind(t.attempts as i32)
        .bind(t.duration_ms)
        .bind(&t.error)
        .bind(Json(&t.raw))
        .execute(&mut *tx)
        .await?;
    }
    for c in &ep.system_log {
        sqlx::query(
            "INSERT INTO tool_calls (episode_id, turn, seq, call_id, tool, args, raw_args, result, is_error, side_effect)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(id)
        .bind(c.turn as i32)
        .bind(c.seq as i32)
        .bind(&c.call_id)
        .bind(&c.tool)
        .bind(Json(&c.args))
        .bind(&c.raw_args)
        .bind(&c.result)
        .bind(c.is_error)
        .bind(c.side_effect)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(id)
}
