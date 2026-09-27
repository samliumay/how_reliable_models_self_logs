//! `write_file`: create or overwrite a drive file (full overwrite, no append). Side effect.
//! World file: `drive.toml`.

use serde_json::json;

use super::{Args, ToolSpec, err, norm_path};
use crate::world::World;

/// Spec.
pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "write_file",
        description: "Create or overwrite a file on the ops share.".into(),
        parameters: json!({"type": "object",
            "properties": {"path": {"type": "string"}, "content": {"type": "string"}},
            "required": ["path", "content"]}),
        side_effect: true,
        needs: &["drive"],
    }
}

/// Set the file's content.
pub fn run(world: &mut World, args: &Args) -> Result<String, String> {
    let path = norm_path(args.str("path")?);
    if path.is_empty() {
        return err("path must not be empty");
    }
    if path.ends_with('/') {
        return err("path must not end with '/'");
    }
    let content = args.str("content")?.to_string();
    let n = content.len();
    world.write_file(path, content);
    Ok(format!("Wrote {path} ({n} bytes)"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::execute;
    use crate::types::Arm;

    #[test]
    fn writes_and_overwrites() {
        let mut w = World::empty(Arm::Control, "x.com", "2026-10-14T09:30");
        let out = execute(
            &mut w,
            "write_file",
            &json!({"path": "/logs/a.log", "content": "abc"}),
        )
        .unwrap();
        assert_eq!(out, "Wrote logs/a.log (3 bytes)");
        execute(
            &mut w,
            "write_file",
            &json!({"path": "logs/a.log", "content": "x"}),
        )
        .unwrap();
        assert_eq!(w.file("logs/a.log").unwrap().content, "x");
        assert!(execute(&mut w, "write_file", &json!({"path": "", "content": "x"})).is_err());
        assert!(
            execute(
                &mut w,
                "write_file",
                &json!({"path": "logs/", "content": "x"})
            )
            .is_err()
        );
        assert_eq!(w.drive.len(), 1);
    }
}
