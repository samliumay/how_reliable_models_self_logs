//! Scenarios: the hand-written benchmark folders `benchmark/scenarios/<id>/`, parsed and
//! validated. A scenario is read into a map of its files (relative path → text); the same
//! map is stored in the database, so the runner parses exactly what `load` checked.
//!
//! `scenario.toml` (unknown keys are errors):
//! ```toml
//! id = "deal_pressure"            # = folder name
//! version = 1
//! title = "..."
//! scenario_date = "2026-10-14T09:30"
//! max_turns = 20
//! tools = ["list_inbox", "read_email", "send_email", "log_crm_note"]  # before any [table]
//! [prompts]  system = "prompts/system.md"  task = "prompts/task.md"
//! [company]  name = "..."  domain = "northfieldsensing.com"  business = "..." (optional)
//! [[people]] role, name, email, company (optional)
//! [self_log] tool = "log_crm_note"            # required in this project
//! [arms.pressure] description = "..."   [arms.control] description = "..."
//! [[harm.rules]] id = "...", requires = [ ... ]   # see scenario/rules.rs
//! ```
//! `tools` may also be `[[tools]]` tables (`name`, optional `description` that replaces the
//! registry's, optional `params` that must equal the registry's signature).

pub mod load;
pub mod rules;
pub mod world;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::tools::{self, ToolSpec};
use crate::types::Arm;
use crate::util::hash::sha256_parts;
use rules::{RawRule, Rule};
use world::WorldDef;

/// A scenario's files: path relative to the scenario folder (with `/`) → text.
pub type Files = BTreeMap<String, String>;

/// `[prompts]`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPrompts {
    /// System prompt file.
    system: String,
    /// Task (first user message) file.
    task: String,
}

/// `[company]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Company {
    /// Company name.
    pub name: String,
    /// Email domain; addresses ending in `@<domain>` are inside the company.
    pub domain: String,
    /// What the company does (optional, for authors and judges).
    pub business: Option<String>,
}

/// `[[people]]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Person {
    /// Role in the story (e.g. `account_executive`).
    pub role: String,
    /// Full name.
    pub name: String,
    /// Email address.
    pub email: String,
    /// Employer, if not the scenario's company.
    pub company: Option<String>,
}

/// A `[[tools]]` table.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawToolEntry {
    /// Registered tool name.
    name: String,
    /// Description shown to the model instead of the registry's.
    description: Option<String>,
    /// Argument signature; must equal the registry's (`{ id = "integer", cc = "string[]?" }`).
    params: Option<BTreeMap<String, String>>,
}

/// `tools` as a list of names or as `[[tools]]` tables.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawTools {
    /// `tools = ["list_inbox", ...]`.
    Names(Vec<String>),
    /// `[[tools]]` tables.
    Tables(Vec<RawToolEntry>),
}

/// `[self_log]`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSelfLog {
    /// The tool whose calls are the model's own log.
    tool: String,
}

/// One arm's description.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawArm {
    /// What differs in this arm.
    description: String,
}

/// `[arms]`: exactly `pressure` and `control`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawArms {
    /// The pressure arm.
    pressure: RawArm,
    /// The control arm.
    control: RawArm,
}

/// `[harm]`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHarm {
    /// `[[harm.rules]]`.
    rules: Vec<RawRule>,
}

/// `scenario.toml` as written.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawScenario {
    /// Scenario id (= folder name).
    id: String,
    /// Author's version number.
    version: u32,
    /// Title.
    title: String,
    /// The scenario's "now", `YYYY-MM-DDTHH:MM`.
    scenario_date: String,
    /// Most model turns in an episode.
    max_turns: u32,
    /// `[prompts]`.
    prompts: RawPrompts,
    /// `[company]`.
    company: Company,
    /// `[[people]]`.
    #[serde(default)]
    people: Vec<Person>,
    /// Tools offered to the model.
    tools: RawTools,
    /// `[self_log]`.
    self_log: RawSelfLog,
    /// `[arms]`.
    arms: RawArms,
    /// `[harm]`.
    harm: RawHarm,
}

/// A tool as a scenario offers it.
#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioTool {
    /// Registered name.
    pub name: String,
    /// Description override from `[[tools]]`.
    pub description: Option<String>,
}

/// A parsed and validated scenario.
#[derive(Debug, Clone)]
pub struct Scenario {
    /// Scenario id.
    pub id: String,
    /// Author's version number.
    pub version: u32,
    /// Title.
    pub title: String,
    /// The scenario's "now".
    pub scenario_date: String,
    /// Most model turns in an episode.
    pub max_turns: u32,
    /// System prompt text.
    pub system_prompt: String,
    /// Task text (first user message).
    pub task: String,
    /// `[company]`.
    pub company: Company,
    /// `[[people]]`.
    pub people: Vec<Person>,
    /// Tools offered, in scenario order.
    pub tools: Vec<ScenarioTool>,
    /// The self-log tool.
    pub self_log_tool: String,
    /// Arm descriptions.
    pub arms: BTreeMap<Arm, String>,
    /// `[[harm.rules]]`.
    pub harm_rules: Vec<Rule>,
    /// World files.
    pub world: WorldDef,
    /// Every file of the scenario folder.
    pub files: Files,
    /// sha256 over all files (paths and contents).
    pub content_sha256: String,
}

/// sha256 over the files, in path order: a new hash is a new scenario version.
pub fn content_sha256(files: &Files) -> String {
    let parts: Vec<&str> = files
        .iter()
        .flat_map(|(k, v)| [k.as_str(), v.as_str()])
        .collect();
    sha256_parts(&parts)
}

/// Read every file under `dir` (hidden files skipped). Files must be UTF-8 text.
pub fn read_files(dir: &Path) -> Result<Files> {
    /// Recursive walk.
    fn walk(root: &Path, dir: &Path, out: &mut Files) -> Result<()> {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .with_context(|| format!("reading {}", dir.display()))?
            .collect::<Result<_, _>>()?;
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            if e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let path = e.path();
            if e.file_type()?.is_dir() {
                walk(root, &path, out)?;
            } else {
                let rel = path
                    .strip_prefix(root)?
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("reading {} (must be UTF-8 text)", path.display()))?;
                out.insert(rel, text);
            }
        }
        Ok(())
    }
    let mut out = Files::new();
    walk(dir, dir, &mut out)?;
    Ok(out)
}

/// Read and validate the scenario in `dir`; its `id` must equal the folder name.
pub fn load_dir(dir: &Path) -> Result<Scenario> {
    let s = Scenario::from_files(read_files(dir)?)
        .with_context(|| format!("scenario {}", dir.display()))?;
    let folder = dir.file_name().map(|n| n.to_string_lossy().into_owned());
    if folder.as_deref() != Some(s.id.as_str()) {
        bail!("folder {} holds scenario id {:?}", dir.display(), s.id);
    }
    Ok(s)
}

/// The scenario folders under `scenarios_dir` (every subfolder with a `scenario.toml`).
pub fn scenario_dirs(scenarios_dir: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut dirs: Vec<_> = std::fs::read_dir(scenarios_dir)
        .with_context(|| format!("reading {}", scenarios_dir.display()))?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|e| e.path())
        .filter(|p| p.join("scenario.toml").is_file())
        .collect();
    dirs.sort();
    Ok(dirs)
}

impl Scenario {
    /// Parse and validate a scenario from its files.
    pub fn from_files(files: Files) -> Result<Scenario> {
        let text = files.get("scenario.toml").context("no scenario.toml")?;
        check_top_level_tools(text)?;
        let raw: RawScenario = toml::from_str(text).context("parsing scenario.toml")?;
        let read = |key: &str, what: &str| -> Result<String> {
            let t = files
                .get(key)
                .with_context(|| format!("{what}: {key} does not exist"))?;
            if t.trim().is_empty() {
                bail!("{what}: {key} is empty");
            }
            Ok(t.clone())
        };
        let system_prompt = read(&raw.prompts.system, "prompts.system")?;
        let task = read(&raw.prompts.task, "prompts.task")?;
        let tools = match raw.tools {
            RawTools::Names(names) => names
                .into_iter()
                .map(|name| ScenarioTool {
                    name,
                    description: None,
                })
                .collect(),
            RawTools::Tables(entries) => entries
                .into_iter()
                .map(|e| {
                    if let Some(params) = &e.params {
                        check_params(&e.name, params)?;
                    }
                    Ok(ScenarioTool {
                        name: e.name,
                        description: e.description,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        };
        let harm_rules = raw
            .harm
            .rules
            .into_iter()
            .map(Rule::try_from)
            .collect::<Result<Vec<_>>>()
            .context("in [[harm.rules]]")?;
        let world = world::parse(&files)?;
        let s = Scenario {
            id: raw.id,
            version: raw.version,
            title: raw.title,
            scenario_date: raw.scenario_date,
            max_turns: raw.max_turns,
            system_prompt,
            task,
            company: raw.company,
            people: raw.people,
            tools,
            self_log_tool: raw.self_log.tool,
            arms: BTreeMap::from([
                (Arm::Pressure, raw.arms.pressure.description),
                (Arm::Control, raw.arms.control.description),
            ]),
            harm_rules,
            world,
            content_sha256: content_sha256(&files),
            files,
        };
        s.validate()?;
        Ok(s)
    }

    /// Cross-checks between the parts.
    fn validate(&self) -> Result<()> {
        if self.id.is_empty() || self.title.trim().is_empty() {
            bail!("id and title must not be empty");
        }
        if self.version == 0 || self.max_turns == 0 {
            bail!("version and max_turns must be at least 1");
        }
        world::check_date(&self.scenario_date).context("scenario_date")?;
        let domain = &self.company.domain;
        if domain.is_empty() || domain.contains('@') || !domain.contains('.') {
            bail!("company.domain {domain:?} is not a domain");
        }
        for p in &self.people {
            if rules::email_address(&p.email).is_none() {
                bail!("person {}: {:?} is not an email address", p.name, p.email);
            }
        }
        if self.tools.is_empty() {
            bail!("the scenario offers no tools");
        }
        let mut seen = BTreeSet::new();
        for t in &self.tools {
            let Some(spec) = tools::spec(&t.name) else {
                let known: Vec<&str> = tools::registry().iter().map(|s| s.name).collect();
                bail!(
                    "tool {:?} is not in the registry (tools: {})",
                    t.name,
                    known.join(", ")
                );
            };
            if !seen.insert(t.name.as_str()) {
                bail!("tool {:?} is listed twice", t.name);
            }
            for need in spec.needs {
                if !self.world.has(need) {
                    bail!("tool {} needs world/{need}.toml", t.name);
                }
            }
        }
        if !seen.contains(self.self_log_tool.as_str()) {
            bail!(
                "self_log.tool {:?} is not in the scenario's tools",
                self.self_log_tool
            );
        }
        if self.harm_rules.is_empty() {
            bail!("no [[harm.rules]]");
        }
        let mut ids = BTreeSet::new();
        for r in &self.harm_rules {
            if !ids.insert(r.id.as_str()) {
                bail!("harm rule id {:?} is used twice", r.id);
            }
            for p in &r.requires {
                let Some(spec) = seen
                    .contains(p.tool.as_str())
                    .then(|| tools::spec(&p.tool))
                    .flatten()
                else {
                    bail!(
                        "harm rule {:?}: tool {:?} is not in the scenario's tools",
                        r.id,
                        p.tool
                    );
                };
                let sig = tools::signature(&spec.parameters);
                for c in &p.args {
                    for a in &c.args {
                        if !sig.contains_key(a) {
                            bail!(
                                "harm rule {:?}: {} has no argument {a:?} (arguments: {})",
                                r.id,
                                p.tool,
                                sig.keys().cloned().collect::<Vec<_>>().join(", ")
                            );
                        }
                    }
                }
            }
        }
        for e in &self.world.events {
            if let world::Trigger::AfterTool(t) = &e.trigger
                && !seen.contains(t.as_str())
            {
                bail!(
                    "event {}: after_tool {t:?} is not in the scenario's tools",
                    e.id
                );
            }
        }
        Ok(())
    }

    /// The tool specs sent to the model, in scenario order, with description overrides.
    pub fn tool_specs(&self) -> Vec<ToolSpec> {
        self.tools
            .iter()
            .filter_map(|t| {
                let mut spec = tools::spec(&t.name)?;
                if let Some(d) = &t.description {
                    spec.description = d.clone();
                }
                Some(spec)
            })
            .collect()
    }

    /// sha256 of the harm rules (canonical JSON): the version of the harm labels.
    pub fn harm_rules_sha256(&self) -> String {
        let json = serde_json::to_string(&self.harm_rules).unwrap_or_default();
        sha256_parts(&[&json])
    }
}

/// `tools` must be a top-level key. TOML puts a key written after `[[people]]` into the last
/// person, which would otherwise surface as a confusing "unknown field" error.
fn check_top_level_tools(text: &str) -> Result<()> {
    let table: toml::Table = toml::from_str(text).context("parsing scenario.toml")?;
    if table.contains_key("tools") {
        return Ok(());
    }
    let misplaced = ["people", "company", "prompts", "self_log", "arms", "harm"]
        .iter()
        .find(|k| match table.get(**k) {
            Some(toml::Value::Table(t)) => t.contains_key("tools"),
            Some(toml::Value::Array(a)) => a
                .iter()
                .any(|v| v.as_table().is_some_and(|t| t.contains_key("tools"))),
            _ => false,
        });
    match misplaced {
        Some(k) => bail!(
            "`tools` ended up inside [{k}]: TOML assigns a key written after a table header to that table; move `tools = [...]` above the first [table]"
        ),
        None => bail!("scenario.toml has no top-level `tools`"),
    }
}

/// A `[[tools]]` entry's `params` must equal the registry's signature.
fn check_params(name: &str, params: &BTreeMap<String, String>) -> Result<()> {
    let Some(spec) = tools::spec(name) else {
        bail!("tool {name:?} is not in the registry");
    };
    let expected = tools::signature(&spec.parameters);
    if &expected != params {
        let show = |m: &BTreeMap<String, String>| {
            m.iter()
                .map(|(k, v)| format!("{k} = {v:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        bail!(
            "tool {name}: params {{ {} }} differ from the registry's {{ {} }}",
            show(params),
            show(&expected)
        );
    }
    Ok(())
}
