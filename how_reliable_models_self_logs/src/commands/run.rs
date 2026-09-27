//! `run`: one model over the chosen scenarios × both arms × samples; every episode is stored
//! with its turns and system log.

use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail};
use futures::StreamExt;
use sqlx::{PgPool, Row};

use crate::agent::{Settings, run_episode};
use crate::config::{Config, ModelConfig, ModelSource, SCRIPTED_PREFIX};
use crate::db::{Cell, scenario, store_episode};
use crate::models::Backend;
use crate::scenario::Scenario;
use crate::types::{Api, Arm, EpisodeStatus, Phase};
use crate::util::git::git_state;
use crate::util::text::truncate;

/// Options of the `run` command.
pub struct RunArgs {
    /// Model id from `config.toml`, or `scripted:<path>` (new runs).
    pub model: Option<String>,
    /// Why the run is made (new runs).
    pub phase: Option<Phase>,
    /// Scenario ids; none means every loaded scenario (new runs).
    pub scenarios: Vec<String>,
    /// Samples per scenario × arm; default from `config.toml`.
    pub samples: Option<u32>,
    /// Run id to continue.
    pub resume: Option<String>,
    /// Run despite uncommitted changes.
    pub allow_dirty: bool,
}

/// A started or resumed run.
struct Started {
    /// Run id.
    run_id: String,
    /// Model id.
    model: String,
    /// Pin recorded with the run.
    pin: String,
    /// Scenario ids the run covers.
    scenario_ids: Vec<String>,
    /// Samples per scenario × arm.
    samples: u32,
    /// Model client.
    backend: Backend,
}

/// One scenario × arm × sample to run.
struct Job<'a> {
    /// Scenario row (version).
    row: i64,
    /// The parsed scenario.
    scenario: &'a Scenario,
    /// Arm.
    arm: Arm,
    /// Sample index, from 0.
    sample: i32,
}

/// Start or resume a run and store every episode. Returns the run id.
pub async fn run(pool: &PgPool, cfg: &Config, args: RunArgs) -> Result<String> {
    let (code_hash, code_dirty) = git_state(&cfg.crate_dir);
    let code = format!("{code_hash}{}", if code_dirty { "-dirty" } else { "" });
    let st = match &args.resume {
        Some(id) => resume_run(pool, cfg, id, args.allow_dirty).await?,
        None => new_run(pool, cfg, &args).await?,
    };

    // Latest version of each of the run's scenarios.
    let rows = sqlx::query(
        "SELECT DISTINCT ON (scenario_id) id, scenario_id FROM scenarios WHERE scenario_id = ANY($1)
         ORDER BY scenario_id, loaded_at DESC, id DESC",
    )
    .bind(&st.scenario_ids)
    .fetch_all(pool)
    .await?;
    let current: HashMap<String, i64> = rows
        .iter()
        .map(|r| (r.get("scenario_id"), r.get("id")))
        .collect();
    for id in &st.scenario_ids {
        if !current.contains_key(id) {
            bail!("scenario {id} is not loaded: run `load` first");
        }
    }
    // A resumed run must keep the scenario versions it started with.
    let used: Vec<i64> =
        sqlx::query_scalar("SELECT DISTINCT scenario_row FROM episodes WHERE run_id = $1")
            .bind(&st.run_id)
            .fetch_all(pool)
            .await?;
    let current_rows: HashSet<i64> = current.values().copied().collect();
    if used.iter().any(|r| !current_rows.contains(r)) {
        bail!(
            "scenarios changed since run {} started; start a new run instead of resuming",
            st.run_id
        );
    }
    let done: HashSet<(i64, String, i32)> = sqlx::query(
        "SELECT scenario_row, arm, sample FROM episodes WHERE run_id = $1 AND status <> 'failed'",
    )
    .bind(&st.run_id)
    .fetch_all(pool)
    .await?
    .iter()
    .map(|r| (r.get("scenario_row"), r.get("arm"), r.get("sample")))
    .collect();

    let mut parsed: Vec<(i64, Scenario)> = Vec::new();
    for id in &st.scenario_ids {
        let row = current[id];
        parsed.push((row, scenario(pool, row).await?));
    }
    let mut jobs = Vec::new();
    for (row, s) in &parsed {
        for arm in Arm::ALL {
            for sample in 0..st.samples as i32 {
                if !done.contains(&(*row, arm.as_str().to_string(), sample)) {
                    jobs.push(Job {
                        row: *row,
                        scenario: s,
                        arm: *arm,
                        sample,
                    });
                }
            }
        }
    }
    println!(
        "{}: {} via {} {}, {} episodes to run ({} already done)",
        st.run_id,
        st.model,
        st.backend.api(),
        truncate(&st.pin, 24),
        jobs.len(),
        done.len()
    );

    let total = jobs.len();
    let mut stream = futures::stream::iter(jobs.into_iter().map(|job| {
        let st = &st;
        async move {
            let settings = Settings {
                model: &st.model,
                pin: &st.pin,
                temperature: cfg.run.temperature,
                max_tokens: cfg.run.max_tokens,
                // Different samples get different, reproducible seeds.
                seed: cfg.run.seed.map(|s| s + job.sample as u64),
                max_retries: cfg.run.max_retries,
            };
            let ep = run_episode(&st.backend, job.scenario, job.arm, &settings).await;
            (job, ep)
        }
    }))
    .buffer_unordered(cfg.run.concurrency);

    let (mut n, mut failed) = (0usize, 0usize);
    while let Some((job, ep)) = stream.next().await {
        n += 1;
        let cell = Cell {
            run_id: &st.run_id,
            scenario_row: job.row,
            arm: job.arm,
            sample: job.sample,
            code: &code,
        };
        let id = store_episode(pool, &cell, &ep).await?;
        let name = format!("{} {} s{}", job.scenario.id, job.arm, job.sample);
        match ep.status {
            EpisodeStatus::Failed => {
                failed += 1;
                println!(
                    "  [{n}/{total}] FAILED {name} (episode {id}): {}",
                    truncate(ep.error.as_deref().unwrap_or("?"), 160)
                );
            }
            EpisodeStatus::Finished | EpisodeStatus::MaxTurns | EpisodeStatus::Truncated => {
                let side = ep
                    .system_log
                    .iter()
                    .filter(|c| c.side_effect && !c.is_error)
                    .count();
                let logs = ep
                    .system_log
                    .iter()
                    .filter(|c| c.tool == job.scenario.self_log_tool && !c.is_error)
                    .count();
                println!(
                    "  [{n}/{total}] {:9} {name} (episode {id}): {} turns, {} calls, {side} side-effect, {logs} self-log, {:.0}s",
                    ep.status,
                    ep.turns.len(),
                    ep.system_log.len(),
                    ep.duration.as_secs_f64()
                );
            }
        }
    }
    println!(
        "{}: {} stored, {failed} failed{}",
        st.run_id,
        n - failed,
        if failed > 0 {
            format!(" (resume with --resume {})", st.run_id)
        } else {
            String::new()
        }
    );
    drop(stream);
    Ok(st.run_id)
}

/// Check the pin and the git trees, then insert the run row.
async fn new_run(pool: &PgPool, cfg: &Config, args: &RunArgs) -> Result<Started> {
    let (Some(model_id), Some(phase)) = (&args.model, args.phase) else {
        bail!("a new run needs --model and --phase");
    };
    let m = cfg.model(model_id)?;
    if m.api() == Api::Scripted && phase != Phase::Smoke {
        bail!("scripted models run only in the smoke phase");
    }
    let backend = Backend::new(cfg, &m)?;
    // Fail before creating the run if the pinned endpoint or digest cannot serve it.
    let pin = backend.check(&m, cfg.run.max_tokens).await?;
    let (code_hash, code_dirty, bench_hash, bench_dirty) = ensure_clean(cfg, args.allow_dirty)?;
    let samples = args.samples.unwrap_or(cfg.run.samples);
    if samples == 0 {
        bail!("--samples must be at least 1");
    }
    let scenario_ids: Vec<String> = if args.scenarios.is_empty() {
        sqlx::query_scalar("SELECT DISTINCT scenario_id FROM scenarios ORDER BY scenario_id")
            .fetch_all(pool)
            .await?
    } else {
        args.scenarios.clone()
    };
    if scenario_ids.is_empty() {
        bail!("no scenarios loaded: run `load` first");
    }
    let slug = match m.api() {
        Api::Scripted => "scripted".to_string(),
        Api::OpenRouter | Api::Ollama => m.id.replace(['/', ':'], "_"),
    };
    let run_id = format!("{}_{slug}", chrono::Local::now().format("%Y%m%d-%H%M%S"));
    sqlx::query(
        "INSERT INTO runs (run_id, phase, model, api, provider, samples, scenario_ids, config_toml, config_sha256,
                           code_git_hash, code_dirty, benchmark_git_hash, benchmark_dirty)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
    )
    .bind(&run_id)
    .bind(phase.as_str())
    .bind(&m.id)
    .bind(m.api().as_str())
    .bind(&pin)
    .bind(samples as i32)
    .bind(&scenario_ids)
    .bind(&cfg.text)
    .bind(&cfg.sha256)
    .bind(&code_hash)
    .bind(code_dirty)
    .bind(&bench_hash)
    .bind(bench_dirty)
    .execute(pool)
    .await?;
    Ok(Started {
        run_id,
        model: m.id.clone(),
        pin,
        scenario_ids,
        samples,
        backend,
    })
}

/// Reload a run. The settings that shape its episodes (`[run]` and its model entry, or the
/// script's hash) must be unchanged; other edits to `config.toml` are allowed.
async fn resume_run(
    pool: &PgPool,
    cfg: &Config,
    run_id: &str,
    allow_dirty: bool,
) -> Result<Started> {
    ensure_clean(cfg, allow_dirty)?;
    let Some(r) = sqlx::query(
        "SELECT model, api, provider, samples, scenario_ids, config_toml FROM runs WHERE run_id = $1",
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await?
    else {
        bail!("no run {run_id}");
    };
    let (model, pin): (String, String) = (r.get("model"), r.get("provider"));
    let api: Api = r.get::<String, _>("api").parse()?;
    let old = Config::from_text(r.get("config_toml"), cfg.crate_dir.clone())?;
    if old.run != cfg.run {
        bail!("[run] changed since run {run_id} started; restore it or start a new run");
    }
    let m = match api {
        Api::Scripted => ModelConfig {
            id: model.clone(),
            source: ModelSource::Scripted {
                script: model.strip_prefix(SCRIPTED_PREFIX).unwrap_or(&model).into(),
            },
        },
        Api::OpenRouter | Api::Ollama => {
            if old.model(&model)? != cfg.model(&model)? {
                bail!(
                    "the entry for {model} changed since run {run_id} started; restore it or start a new run"
                );
            }
            cfg.model(&model)?
        }
    };
    let backend = Backend::new(cfg, &m)?;
    let checked = backend.check(&m, cfg.run.max_tokens).await?;
    if checked != pin {
        bail!("run {run_id} is pinned to {pin}, now {checked}; start a new run");
    }
    Ok(Started {
        run_id: run_id.to_string(),
        model,
        pin,
        scenario_ids: r.get("scenario_ids"),
        samples: r.get::<i32, _>("samples") as u32,
        backend,
    })
}

/// Git state of code and benchmark; refuses uncommitted changes unless allowed.
fn ensure_clean(cfg: &Config, allow_dirty: bool) -> Result<(String, bool, String, bool)> {
    let (code_hash, code_dirty) = git_state(&cfg.crate_dir);
    let (bench_hash, bench_dirty) = git_state(&cfg.benchmark_dir);
    if (code_dirty || bench_dirty) && !allow_dirty {
        let dirty: Vec<&str> = [(code_dirty, "code"), (bench_dirty, "benchmark")]
            .iter()
            .filter(|(d, _)| *d)
            .map(|(_, n)| *n)
            .collect();
        bail!(
            "refusing to run: uncommitted changes in {}. Commit first, or pass --allow-dirty (recorded with the run).",
            dirty.join(" and ")
        );
    }
    Ok((code_hash, code_dirty, bench_hash, bench_dirty))
}
