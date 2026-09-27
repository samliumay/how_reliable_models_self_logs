//! `list_files`: the drive's current paths (initial files minus deleted plus written),
//! optionally under a prefix, sorted. World file: `drive.toml`.

use serde_json::json;

use super::{Args, ToolSpec, norm_path};
use crate::world::World;

/// Spec.
pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "list_files",
        description: "List files on the shared drive. Optional path prefix, e.g. 'logs/'.".into(),
        parameters: json!({"type": "object",
            "properties": {"path": {"type": "string", "description": "Path prefix."}}}),
        side_effect: false,
        needs: &["drive"],
    }
}

/// One path per line, or `(no files under <path>)`.
pub fn run(world: &mut World, args: &Args) -> Result<String, String> {
    let prefix = args.opt_str("path").map(norm_path).unwrap_or("");
    let mut paths: Vec<&str> = world
        .drive
        .iter()
        .map(|f| f.path.as_str())
        .filter(|p| p.starts_with(prefix))
        .collect();
    paths.sort_unstable();
    if paths.is_empty() {
        return Ok(format!("(no files under {prefix})"));
    }
    Ok(paths.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::execute;
    use crate::types::Arm;

    #[test]
    fn lists_sorted_paths_under_a_prefix() {
        let mut w = World::empty(Arm::Control, "x.com", "2026-10-14T09:30");
        for p in ["logs/b.log", "a.md", "logs/a.log"] {
            w.write_file(p, String::new());
        }
        assert_eq!(
            execute(&mut w, "list_files", &json!({})).unwrap(),
            "a.md\nlogs/a.log\nlogs/b.log"
        );
        assert_eq!(
            execute(&mut w, "list_files", &json!({"path": "/logs/"})).unwrap(),
            "logs/a.log\nlogs/b.log"
        );
        assert_eq!(
            execute(&mut w, "list_files", &json!({"path": "tmp/"})).unwrap(),
            "(no files under tmp/)"
        );
    }
}
