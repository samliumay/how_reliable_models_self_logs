//! `world/drive.toml`: the shared drive. `[[file]]` path, content (markdown path),
//! confidential (default false), arms. Paths are unique per arm.

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use super::{body, parse_opt, unique_per_arm};
use crate::scenario::Files;
use crate::types::Arm;

/// `[[file]]` as written.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    /// Path on the drive, as the model sees it.
    path: String,
    /// Markdown file with the content, relative to `world/`.
    content: String,
    /// Marked confidential.
    #[serde(default)]
    confidential: bool,
    /// Arms it appears in.
    arms: Option<Vec<Arm>>,
}

/// The file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DriveFile {
    /// The files.
    #[serde(default)]
    file: Vec<RawFile>,
}

/// A drive file with its content.
#[derive(Debug, Clone, PartialEq)]
pub struct FileDef {
    /// Path on the drive.
    pub path: String,
    /// Content text.
    pub content: String,
    /// Marked confidential (metadata for rules and authors; not shown to the model).
    pub confidential: bool,
    /// Arms it appears in (`None`: both).
    pub arms: Option<Vec<Arm>>,
}

/// Parse `world/drive.toml`, if present.
pub fn parse(files: &Files) -> Result<Option<Vec<FileDef>>> {
    let Some(f) = parse_opt::<DriveFile>(files, "drive")? else {
        return Ok(None);
    };
    let out = f
        .file
        .into_iter()
        .map(|f| {
            if f.path.is_empty() || f.path.starts_with('/') || f.path.ends_with('/') {
                bail!("drive path {:?} must be relative and name a file", f.path);
            }
            Ok(FileDef {
                content: body(files, &f.content)
                    .with_context(|| format!("drive file {}", f.path))
                    .context("in world/drive.toml")?,
                path: f.path,
                confidential: f.confidential,
                arms: f.arms,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    unique_per_arm("drive path", out.iter().map(|f| (f.path.as_str(), &f.arms)))?;
    Ok(Some(out))
}
