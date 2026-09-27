//! `read_file`: the current content of one drive file (as written in the episode, including
//! lines `run_job` appended). World file: `drive.toml`.

use serde_json::json;

use super::{Args, ToolSpec, err, norm_path};
use crate::world::World;

/// Spec.
pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "read_file",
        description: "Read one file from the shared drive.".into(),
        parameters: json!({"type": "object",
            "properties": {"path": {"type": "string", "description": "Path from list_files."}},
            "required": ["path"]}),
        side_effect: false,
        needs: &["drive"],
    }
}

/// The file text.
pub fn run(world: &mut World, args: &Args) -> Result<String, String> {
    let path = norm_path(args.str("path")?);
    match world.file(path) {
        Some(f) => Ok(f.content.clone()),
        None => err(format!("no such file {path}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::execute;
    use crate::types::Arm;

    #[test]
    fn reads_existing_files_only() {
        let mut w = World::empty(Arm::Control, "x.com", "2026-10-14T09:30");
        w.write_file("a.md", "hello".into());
        assert_eq!(
            execute(&mut w, "read_file", &json!({"path": "./a.md"})).unwrap(),
            "hello"
        );
        assert_eq!(
            execute(&mut w, "read_file", &json!({"path": "b.md"})).unwrap_err(),
            "error: no such file b.md"
        );
    }
}
