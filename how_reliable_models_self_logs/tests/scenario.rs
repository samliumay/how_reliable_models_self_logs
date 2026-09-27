//! Loading and validating a scenario from disk, with the tiny test fixture under
//! `tests/fixtures/scenarios/` (not a benchmark scenario). Also a scripted episode on it,
//! end to end through the harm rules and the log-accuracy split (no database, no API).

use std::path::PathBuf;

use how_reliable_models_self_logs::agent::{Settings, run_episode};
use how_reliable_models_self_logs::judges::log_accuracy::split;
use how_reliable_models_self_logs::labels::harm;
use how_reliable_models_self_logs::models::Backend;
use how_reliable_models_self_logs::models::scripted::Script;
use how_reliable_models_self_logs::scenario::{self, Files, Scenario, content_sha256, read_files};
use how_reliable_models_self_logs::types::{Arm, EpisodeStatus};
use how_reliable_models_self_logs::world::World;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scenarios/fixture_refund")
}

fn files() -> Files {
    read_files(&fixture_dir()).unwrap()
}

/// Parse the fixture with one file replaced.
fn with(key: &str, text: &str) -> anyhow::Result<Scenario> {
    let mut f = files();
    f.insert(key.into(), text.into());
    Scenario::from_files(f)
}

/// The fixture's scenario.toml with `from` replaced by `to`.
fn toml_with(from: &str, to: &str) -> String {
    let t = files()["scenario.toml"].clone();
    assert!(t.contains(from), "{from}");
    t.replace(from, to)
}

#[test]
fn fixture_loads_and_validates() {
    let s = scenario::load_dir(&fixture_dir()).unwrap();
    assert_eq!(s.id, "fixture_refund");
    assert_eq!(s.self_log_tool, "write_worklog");
    assert_eq!(s.harm_rules.len(), 1);
    let names: Vec<&str> = s.tool_specs().iter().map(|t| t.name).collect();
    assert_eq!(names[0], "list_inbox");
    assert_eq!(s.world.emails.as_ref().unwrap().len(), 3);
    assert_eq!(s.world.events.len(), 1);
    assert!(s.system_prompt.contains("support assistant"));
}

#[test]
fn each_arm_sees_its_own_items() {
    let s = scenario::load_dir(&fixture_dir()).unwrap();
    let p = World::new(&s, Arm::Pressure);
    let c = World::new(&s, Arm::Control);
    let body = |w: &World| w.inbox.iter().find(|e| e.id == "1").unwrap().body.clone();
    assert!(body(&p).contains("Keep them down"));
    assert!(body(&c).contains("as usual"));
    assert_eq!(p.inbox.len(), 2);
    assert_eq!(p.pending.len(), 1);
}

#[test]
fn content_hash_is_stable_and_covers_every_file() {
    let f = files();
    assert!(!f.keys().any(|k| k.starts_with('.')));
    let h = content_sha256(&f);
    assert_eq!(h, content_sha256(&files()));
    let mut g = f.clone();
    g.insert("world/emails/002_request.md".into(), "changed".into());
    assert_ne!(h, content_sha256(&g));
}

#[test]
fn tools_inside_the_last_person_are_rejected() {
    let t = files()["scenario.toml"].clone();
    let line = t
        .lines()
        .find(|l| l.starts_with("tools = "))
        .unwrap()
        .to_string();
    let moved = t.replace(&format!("{line}\n"), "").replace(
        "company = \"Customer Co\"\n",
        &format!("company = \"Customer Co\"\n{line}\n"),
    );
    let e = with("scenario.toml", &moved).unwrap_err();
    assert!(format!("{e:#}").contains("inside [people]"), "{e:#}");
}

#[test]
fn inconsistent_scenarios_are_rejected() {
    let cases = [
        // Tool not in the registry.
        toml_with(
            "\"write_worklog\"]",
            "\"write_worklog\", \"launch_rocket\"]",
        ),
        // Self-log tool not offered.
        toml_with("tool = \"write_worklog\"", "tool = \"write_file\""),
        // Rule on a tool the scenario does not offer.
        toml_with(
            "{ tool = \"send_email\", args",
            "{ tool = \"decide_refund\", args",
        ),
        // Rule on an argument the tool does not have.
        toml_with("arg = \"to\"", "arg = \"recipients\""),
        // A third arm.
        toml_with(
            "[arms.control]",
            "[arms.extra]\ndescription = \"x\"\n\n[arms.control]",
        ),
        // Unknown key.
        toml_with("max_turns = 6", "max_turns = 6\nseed = 3"),
        // A tool whose world file is missing.
        toml_with("\"write_worklog\"]", "\"write_worklog\", \"list_tickets\"]"),
    ];
    for (i, text) in cases.iter().enumerate() {
        assert!(
            with("scenario.toml", text).is_err(),
            "case {i} was accepted"
        );
    }
    // A body file that does not exist.
    let emails = files()["world/emails.toml"].replace("002_request.md", "999_missing.md");
    assert!(with("world/emails.toml", &emails).is_err());
    // An unknown world file.
    assert!(with("world/payroll.toml", "x = 1").is_err());
}

#[test]
fn tools_tables_may_replace_descriptions_but_not_signatures() {
    let t = files()["scenario.toml"].clone();
    let line = t
        .lines()
        .find(|l| l.starts_with("tools = "))
        .unwrap()
        .to_string();
    let tables = r#"[[tools]]
name = "list_inbox"
description = "Show the support inbox."

[[tools]]
name = "send_email"
params = { to = "string[]", cc = "string[]?", subject = "string", body = "string" }

[[tools]]
name = "write_worklog"
"#;
    let text = format!("{}\n{tables}", t.replace(&format!("{line}\n"), ""));
    let s = with("scenario.toml", &text).unwrap();
    assert_eq!(s.tool_specs()[0].description, "Show the support inbox.");
    let wrong = text.replace("cc = \"string[]?\"", "cc = \"string[]\"");
    assert!(with("scenario.toml", &wrong).is_err());
}

#[tokio::test]
async fn scripted_episode_through_harm_rules_and_log_split() {
    let s = scenario::load_dir(&fixture_dir()).unwrap();
    let script = Script::load(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/script_fixture.json"),
    )
    .unwrap();
    let backend = Backend::Scripted(script);
    let settings = Settings {
        model: "scripted",
        pin: "x",
        temperature: 1.0,
        max_tokens: 100,
        seed: None,
        max_retries: 0,
    };
    let ep = run_episode(&backend, &s, Arm::Pressure, &settings).await;
    assert_eq!(ep.status, EpisodeStatus::Finished);
    assert_eq!(
        harm::evaluate(&s, &ep.system_log),
        vec!["deny_valid_refund"]
    );
    let sp = split(&ep.system_log, &s.self_log_tool);
    assert_eq!(sp.actions.len(), 1);
    assert_eq!(sp.entries.len(), 1);
    // The control arm with the same script is labelled the same way: labels depend on the
    // calls, not on the arm.
    let ep = run_episode(&backend, &s, Arm::Control, &settings).await;
    assert_eq!(
        harm::evaluate(&s, &ep.system_log),
        vec!["deny_valid_refund"]
    );
}
