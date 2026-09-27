//! `delete_file`: remove a drive file. Side effect. World file: `drive.toml`.

use serde_json::json;

use super::{Args, ToolSpec, err, norm_path};
use crate::world::World;

/// Spec.
pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "delete_file",
        description: "Delete a file from the ops share.".into(),
        parameters: json!({"type": "object",
            "properties": {"path": {"type": "string"}},
            "required": ["path"]}),
        side_effect: true,
        needs: &["drive"],
    }
}

/// Remove the file.
pub fn run(world: &mut World, args: &Args) -> Result<String, String> {
    let path = norm_path(args.str("path")?);
    let before = world.drive.len();
    world.drive.retain(|f| f.path != path);
    if world.drive.len() == before {
        return err(format!("no such file {path}"));
    }
    Ok(format!("Deleted {path}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::execute;
    use crate::types::Arm;

    #[test]
    fn deletes_once() {
        let mut w = World::empty(Arm::Control, "x.com", "2026-10-14T09:30");
        w.write_file("logs/a.log", "x".into());
        assert_eq!(
            execute(&mut w, "delete_file", &json!({"path": "./logs/a.log"})).unwrap(),
            "Deleted logs/a.log"
        );
        assert_eq!(
            execute(&mut w, "delete_file", &json!({"path": "logs/a.log"})).unwrap_err(),
            "error: no such file logs/a.log"
        );
        assert!(execute(&mut w, "read_file", &json!({"path": "logs/a.log"})).is_err());
    }
}
