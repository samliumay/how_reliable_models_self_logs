//! Command-line entry point: parses the subcommand and calls the library.

#![warn(missing_docs, clippy::missing_docs_in_private_items)]

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use how_reliable_models_self_logs::commands::{
    judge_aware, judge_log_accuracy, label, load, report, run, validate,
};
use how_reliable_models_self_logs::config::Config;
use how_reliable_models_self_logs::db;
use how_reliable_models_self_logs::types::Phase;

/// Command-line arguments.
#[derive(Parser)]
#[command(about = "How reliable are models' self-logs?")]
struct Cli {
    /// The experiment config.
    #[arg(long, default_value = "config.toml")]
    config: PathBuf,
    /// What to do.
    #[command(subcommand)]
    command: Command,
}

/// The pipeline steps.
#[derive(Subcommand)]
enum Command {
    /// Apply database migrations.
    Migrate,
    /// Parse and check the scenarios on disk (no database).
    Validate {
        /// Scenario ids (repeatable); none means all.
        #[arg(long = "scenario")]
        scenarios: Vec<String>,
    },
    /// Validate scenarios and load new versions into the database.
    Load {
        /// Scenario ids (repeatable); none means all.
        #[arg(long = "scenario")]
        scenarios: Vec<String>,
    },
    /// Run one model over the scenarios × both arms × samples.
    Run {
        /// Model id from config.toml, or `scripted:<script.json>` (smoke phase only).
        #[arg(long)]
        model: Option<String>,
        /// smoke | pilot | study
        #[arg(long)]
        phase: Option<String>,
        /// Scenario ids (repeatable); none means every loaded scenario.
        #[arg(long = "scenario")]
        scenarios: Vec<String>,
        /// Samples per scenario × arm (default: config.toml).
        #[arg(long)]
        samples: Option<u32>,
        /// Continue an existing run; only missing or failed cells are run.
        #[arg(long)]
        resume: Option<String>,
        /// Run despite uncommitted changes; recorded with the run.
        #[arg(long)]
        allow_dirty: bool,
    },
    /// Deterministic labels (harm) for every unlabelled episode.
    Label {
        /// Run ids (repeatable); none means all runs.
        #[arg(long = "run")]
        runs: Vec<String>,
    },
    /// Label test awareness of every unjudged episode.
    JudgeAware {
        /// Run ids (repeatable); none means all runs.
        #[arg(long = "run")]
        runs: Vec<String>,
    },
    /// Label log accuracy of every unjudged episode (`no_log` needs no judge).
    JudgeLogAccuracy {
        /// Run ids (repeatable); none means all runs.
        #[arg(long = "run")]
        runs: Vec<String>,
    },
    /// Print the tables and write results/<run_id>/.
    Report {
        /// Run ids (repeatable); none means all runs.
        #[arg(long = "run")]
        runs: Vec<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = Config::load(&cli.config)?;
    match dotenvy::from_path(&cfg.env_file) {
        Ok(()) => {}
        Err(e) if e.not_found() => {}
        Err(e) => return Err(e).context("reading .env"),
    }
    if let Command::Validate { scenarios } = &cli.command {
        return validate::validate(&cfg, scenarios);
    }
    let pool = db::connect().await?;
    match cli.command {
        Command::Migrate => println!("migrations applied"),
        Command::Validate { .. } => {}
        Command::Load { scenarios } => load::load(&pool, &cfg, &scenarios).await?,
        Command::Run {
            model,
            phase,
            scenarios,
            samples,
            resume,
            allow_dirty,
        } => {
            let args = run::RunArgs {
                model,
                phase: phase.map(|p| p.parse::<Phase>()).transpose()?,
                scenarios,
                samples,
                resume,
                allow_dirty,
            };
            run::run(&pool, &cfg, args).await?;
        }
        Command::Label { runs } => label::label(&pool, &runs).await?,
        Command::JudgeAware { runs } => judge_aware::judge_aware(&pool, &cfg, &runs).await?,
        Command::JudgeLogAccuracy { runs } => {
            judge_log_accuracy::judge_log_accuracy(&pool, &cfg, &runs).await?
        }
        Command::Report { runs } => report::report(&pool, &cfg, &runs).await?,
    }
    Ok(())
}
