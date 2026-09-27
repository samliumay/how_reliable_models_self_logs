//! `validate`: parse and check every scenario on disk (no database). Prints one line per
//! scenario and fails if any scenario is invalid.

use anyhow::{Result, bail};

use crate::config::Config;
use crate::scenario::{self, Scenario};

/// Every scenario folder under the benchmark (or only `only`), each loaded and validated.
pub fn check_all(cfg: &Config, only: &[String]) -> Result<Vec<(String, Result<Scenario>)>> {
    let dirs = scenario::scenario_dirs(&cfg.scenarios_dir())?;
    let mut out = Vec::new();
    for d in dirs {
        let name = d
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !only.is_empty() && !only.contains(&name) {
            continue;
        }
        out.push((name, scenario::load_dir(&d)));
    }
    for o in only {
        if !out.iter().any(|(n, _)| n == o) {
            bail!(
                "no scenario folder {o} under {}",
                cfg.scenarios_dir().display()
            );
        }
    }
    Ok(out)
}

/// Print the result for each scenario; an error if any is invalid.
pub fn validate(cfg: &Config, only: &[String]) -> Result<()> {
    let results = check_all(cfg, only)?;
    if results.is_empty() {
        println!("no scenarios under {}", cfg.scenarios_dir().display());
    }
    let mut bad = 0;
    for (name, r) in &results {
        match r {
            Ok(s) => println!(
                "ok      {name} v{} ({} tools, self-log {}, {} harm rules; {}; sha {})",
                s.version,
                s.tools.len(),
                s.self_log_tool,
                s.harm_rules.len(),
                s.world.summary(),
                &s.content_sha256[..12]
            ),
            Err(e) => {
                bad += 1;
                println!("INVALID {name}: {e:#}");
            }
        }
    }
    if bad > 0 {
        bail!("{bad} of {} scenarios are invalid", results.len());
    }
    Ok(())
}
