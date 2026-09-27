//! `load`: validate scenarios from the benchmark and insert their new content versions.
//! Valid scenarios are loaded even if others fail; the command then fails.

use anyhow::{Result, bail};
use sqlx::PgPool;

use super::validate::check_all;
use crate::config::Config;
use crate::scenario::load::insert;
use crate::util::git::git_state;

/// Load every scenario (or only the given ids) and print what was inserted.
pub async fn load(pool: &PgPool, cfg: &Config, only: &[String]) -> Result<()> {
    let (hash, dirty) = git_state(&cfg.benchmark_dir);
    let results = check_all(cfg, only)?;
    let (mut new, mut same, mut bad) = (0, 0, 0);
    for (name, r) in results {
        match r {
            Ok(s) => {
                if insert(pool, &s, &hash, dirty).await? {
                    new += 1;
                    println!("new       {name} (sha {})", &s.content_sha256[..12]);
                } else {
                    same += 1;
                    println!("unchanged {name}");
                }
            }
            Err(e) => {
                bad += 1;
                println!("INVALID   {name}: {e:#}");
            }
        }
    }
    println!(
        "load: {new} new, {same} unchanged, {bad} invalid (benchmark {}{})",
        &hash[..hash.len().min(12)],
        if dirty { ", dirty" } else { "" }
    );
    if bad > 0 {
        bail!("{bad} scenarios are invalid and were not loaded");
    }
    Ok(())
}
