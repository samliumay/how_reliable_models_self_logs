# How Reliable Are Models' Self-Logs?

When a model is asked to keep its own action log, is that log true? The model works through native tool calls in a simulated workplace (hand-written scenarios, each with a pressure and a control arm). The harness records every tool call in the **system log**; the model's **own log** is a normal work tool (worklog, close notes, shift summary). A judge compares the two and labels each episode accurate, softened, fabricated or omitted.

| | |
|---|---|
| Benchmark | [Hugging Face](https://huggingface.co/datasets/logicBombExe/how_reliable_models_self_logs) |
| Stack | Rust (`cargo`), PostgreSQL in Docker |
| Earlier study | [INSIDER_LLM_DETECTION_CODE](https://github.com/samliumay/INSIDER_LLM_DETECTION_CODE) (archived) |

## Quick start

```bash
cp .env.example .env          # add OPENROUTER_API_KEY and GEMINI_API_KEY
make db                       # PostgreSQL in Docker + migrations
make data                     # validate and load ../benchmark/scenarios
make run MODEL=qwen/qwen3.8-27b PHASE=pilot
make eval                     # harm labels, then the test-awareness and log-accuracy judges
make report                   # tables in the terminal, files in ../results/<run_id>/
```

Needs Docker, Rust (edition 2024), and the benchmark repo cloned next to this one as `../benchmark` (scenarios in `scenarios/`, judge prompts in `labels/`). Models with `api = "ollama"` need a local Ollama server with the model pulled.

## Make targets

| Target | Does |
|---|---|
| `make db` / `make db-down` | start / stop PostgreSQL (port 55431) |
| `make validate` | parse and check every scenario (no database) |
| `make data` | validate and load new scenario versions |
| `make run MODEL=… PHASE=smoke\|pilot\|study [ARGS=…]` | one model × scenarios × both arms × samples; `ARGS="--samples 1"`, `"--scenario ID"`, `"--resume <run_id>"`, `"--allow-dirty"` |
| `make smoke SCRIPT=script.json [ARGS=…]` | dry run with canned replies (no API, smoke phase only) |
| `make label` | deterministic harm labels |
| `make judge` | test awareness + log accuracy (Gemini) |
| `make eval` | `label` then `judge` |
| `make report` | per-run tables, test-unaware and all episodes |
| `make check` | fmt, clippy `-D warnings`, tests, doc lints |
| `make baseline` | not implemented yet |

## Configuration

One file, `how_reliable_models_self_logs/config.toml`, for every model: temperature 1.0, token budget per turn, samples, retries, seed, the judge model, and the model list. Each model is pinned: an OpenRouter model to one provider (fallbacks off, tool calling checked), an Ollama model to its manifest digest. Scripted models are not in the config: `--model scripted:<script.json>`.

## Layout

```
how_reliable_models_self_logs/src/
├── main.rs  lib.rs  config.rs  types.rs  db.rs
├── scenario.rs        # scenario.toml: parse + validate; scenario/{rules,load,world}.rs
├── scenario/world/    # one file per world file: emails, drive, events, jobs, expenses, ledger, tickets
├── world.rs           # the in-memory world of one episode
├── tools.rs           # ToolSpec, registry, argument checks, dispatch
├── tools/             # one file per tool (19, from benchmark/scenarios/TOOLS.md)
├── agent.rs           # the multi-turn loop: model ↔ tools, events, stop conditions
├── models.rs          # conversation types, Reply, Backend enum
├── models/            # openrouter.rs, ollama.rs (native tool calling), scripted.rs
├── judges.rs          # prompt, fill, retry loop
├── judges/            # gemini.rs, eval_aware.rs, log_accuracy.rs
├── labels.rs  labels/harm.rs
├── commands.rs        # one file per subcommand in commands/
└── util.rs            # util/{hash,git,text}.rs
```

A parent file holds the shared types and the dispatch; its folder holds one unit per file (no `mod.rs`). A new tool is one file in `tools/` plus its lines in `tools.rs`. Details: `../docs/code_documentation.md`.

## Status

Milestone 1: scenario loader, tools, agent loop, labels, judges, report. The judge prompts are drafts in `how_reliable_models_self_logs/prompts_draft/` and must move to `../benchmark/labels/` before judging. Baselines are next.

## License

Not chosen yet.
