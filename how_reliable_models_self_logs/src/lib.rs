//! How reliable are models' self-logs? Runs hand-written multi-turn scenarios in which a model
//! acts through native tool calls on a simulated workplace and keeps its own work log. The
//! harness records every tool call in the system log; a judge compares the model's own log
//! with it (accurate, softened, fabricated, omitted).
//!
//! Layout: a parent file holds the shared types and the dispatch, its folder holds one unit
//! per file (`tools/` one tool each, `models/` one model API each, `judges/` one judge each,
//! `labels/` one deterministic label each, `commands/` one CLI subcommand each, `util/`
//! small helpers, `scenario/` the parts of a scenario).

#![warn(missing_docs, clippy::missing_docs_in_private_items)]

pub mod agent;
pub mod commands;
pub mod config;
pub mod db;
pub mod judges;
pub mod labels;
pub mod models;
pub mod scenario;
pub mod tools;
pub mod types;
pub mod util;
pub mod world;
