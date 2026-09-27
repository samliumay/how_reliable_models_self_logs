//! `world/jobs.toml`: the scripted job runner of `run_job`. Outcomes are fixed here, never
//! computed.
//!
//! ```toml
//! job_log = "logs/jobs/maintenance-2026-11.log"   # drive file each successful run appends to
//! [[job]]
//! name = "purge_invoice_drafts"
//! result = "..."            # text the model sees
//! log_line = "..."          # line appended to job_log
//! repeat_result = "..."     # optional: second and later runs (with repeat_log_line)
//! repeat_log_line = "..."
//!   [[job.when]]            # optional: used when job `ran` already ran successfully;
//!   ran = "other_job"       # first matching `when` wins
//!   result = "..."
//!   log_line = "..."
//! [[job]]
//! name = "restore_billing_snapshot"
//! restricted = true         # always refused with `error`
//! error = "..."
//! ```
//! Precedence at run time: restricted → repeat → when → result.

use anyhow::{Result, bail};
use serde::Deserialize;

use super::{parse_opt, unique_per_arm};
use crate::scenario::Files;
use crate::types::Arm;

/// `[[job.when]]` as written.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWhen {
    /// The other job that must have run successfully.
    ran: String,
    /// Result text.
    result: String,
    /// Line appended to the job log.
    log_line: String,
}

/// `[[job]]` as written.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawJob {
    /// Job name.
    name: String,
    /// Result text.
    result: Option<String>,
    /// Line appended to the job log.
    log_line: Option<String>,
    /// Result of second and later runs.
    repeat_result: Option<String>,
    /// Log line of second and later runs.
    repeat_log_line: Option<String>,
    /// Always refused.
    #[serde(default)]
    restricted: bool,
    /// Refusal text of a restricted job.
    error: Option<String>,
    /// Conditional outcomes.
    #[serde(default)]
    when: Vec<RawWhen>,
    /// Arms it exists in.
    arms: Option<Vec<Arm>>,
}

/// The file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct JobsFile {
    /// Drive path of the job log.
    job_log: String,
    /// The jobs.
    #[serde(default)]
    job: Vec<RawJob>,
}

/// A result text and the log line it appends.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// Text the model sees.
    pub result: String,
    /// Line appended to the job log.
    pub log_line: String,
}

/// A conditional outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct When {
    /// The other job that must have run successfully in the episode.
    pub ran: String,
    /// The outcome then.
    pub outcome: Outcome,
}

/// What running a job does.
#[derive(Debug, Clone, PartialEq)]
pub enum JobKind {
    /// Always refused with this text.
    Restricted {
        /// Refusal text.
        error: String,
    },
    /// Runs.
    Runs {
        /// Default outcome.
        first: Outcome,
        /// Outcome of second and later runs.
        repeat: Option<Outcome>,
        /// Conditional outcomes, first match wins.
        when: Vec<When>,
    },
}

/// A job.
#[derive(Debug, Clone, PartialEq)]
pub struct JobDef {
    /// Job name.
    pub name: String,
    /// What it does.
    pub kind: JobKind,
    /// Arms it exists in (`None`: both).
    pub arms: Option<Vec<Arm>>,
}

/// `world/jobs.toml`.
#[derive(Debug, Clone, PartialEq)]
pub struct JobsDef {
    /// Drive path each successful run appends its log line to.
    pub job_log: String,
    /// The jobs.
    pub jobs: Vec<JobDef>,
}

/// Check one job's fields and build it.
fn job(j: RawJob) -> Result<JobDef> {
    let name = j.name;
    let kind = if j.restricted {
        match (
            j.error,
            &j.result,
            &j.log_line,
            &j.repeat_result,
            j.when.is_empty(),
        ) {
            (Some(error), None, None, None, true) => JobKind::Restricted { error },
            (None, ..) => bail!("job {name}: restricted = true needs `error`"),
            _ => bail!("job {name}: a restricted job has only `error`"),
        }
    } else {
        if j.error.is_some() {
            bail!("job {name}: `error` is only for restricted jobs");
        }
        let (Some(result), Some(log_line)) = (j.result, j.log_line) else {
            bail!("job {name}: needs `result` and `log_line`");
        };
        let repeat = match (j.repeat_result, j.repeat_log_line) {
            (Some(result), Some(log_line)) => Some(Outcome { result, log_line }),
            (None, None) => None,
            _ => bail!("job {name}: `repeat_result` and `repeat_log_line` go together"),
        };
        JobKind::Runs {
            first: Outcome { result, log_line },
            repeat,
            when: j
                .when
                .into_iter()
                .map(|w| When {
                    ran: w.ran,
                    outcome: Outcome {
                        result: w.result,
                        log_line: w.log_line,
                    },
                })
                .collect(),
        }
    };
    Ok(JobDef {
        name,
        kind,
        arms: j.arms,
    })
}

/// Parse `world/jobs.toml`, if present.
pub fn parse(files: &Files) -> Result<Option<JobsDef>> {
    let Some(f) = parse_opt::<JobsFile>(files, "jobs")? else {
        return Ok(None);
    };
    if f.job_log.is_empty() || f.job_log.starts_with('/') || f.job_log.ends_with('/') {
        bail!(
            "jobs.toml: job_log {:?} must be a relative file path",
            f.job_log
        );
    }
    let jobs = f.job.into_iter().map(job).collect::<Result<Vec<_>>>()?;
    unique_per_arm("job", jobs.iter().map(|j| (j.name.as_str(), &j.arms)))?;
    for j in &jobs {
        if let JobKind::Runs { when, .. } = &j.kind {
            for w in when {
                if !jobs.iter().any(|o| o.name == w.ran) || w.ran == j.name {
                    bail!(
                        "job {}: [[job.when]] ran = {:?} is not another job",
                        j.name,
                        w.ran
                    );
                }
            }
        }
    }
    Ok(Some(JobsDef {
        job_log: f.job_log,
        jobs,
    }))
}
