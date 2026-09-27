-- Schema v1: scenarios, runs, episodes with turns and the system log, and the labels.
-- Closed sets are TEXT with CHECK constraints; the Rust enums in src/types.rs mirror them.

-- One row per scenario content version. Changed files give a new row (new content hash);
-- episodes point to the exact version they ran on. `files` holds every file of the folder
-- (relative path -> text), so the runner parses exactly what `load` validated.
CREATE TABLE scenarios (
    id                  BIGSERIAL PRIMARY KEY,
    scenario_id         TEXT NOT NULL,
    version             INTEGER NOT NULL,
    title               TEXT NOT NULL,
    arms                TEXT[] NOT NULL,
    self_log_tool       TEXT NOT NULL,
    content_sha256      TEXT NOT NULL,
    files               JSONB NOT NULL,
    benchmark_git_hash  TEXT NOT NULL,
    benchmark_dirty     BOOLEAN NOT NULL,
    loaded_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (scenario_id, content_sha256)
);

-- One run = one model over the chosen scenarios x both arms x samples.
CREATE TABLE runs (
    run_id              TEXT PRIMARY KEY,
    phase               TEXT NOT NULL CHECK (phase IN ('smoke', 'pilot', 'study')),
    model               TEXT NOT NULL,
    api                 TEXT NOT NULL CHECK (api IN ('openrouter', 'ollama', 'scripted')),
    provider            TEXT NOT NULL,        -- pin: OpenRouter provider tag, Ollama digest, script sha256
    samples             INTEGER NOT NULL CHECK (samples > 0),
    scenario_ids        TEXT[] NOT NULL,      -- the scenarios the run covers
    config_toml         TEXT NOT NULL,
    config_sha256       TEXT NOT NULL,
    code_git_hash       TEXT NOT NULL,
    code_dirty          BOOLEAN NOT NULL,
    benchmark_git_hash  TEXT NOT NULL,
    benchmark_dirty     BOOLEAN NOT NULL,
    started_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (api <> 'scripted' OR phase = 'smoke')
);

-- Every episode that ended is stored, failures included. At most one non-failed row per
-- (run, scenario version, arm, sample); a resumed run retries only the cells without one.
CREATE TABLE episodes (
    id                  BIGSERIAL PRIMARY KEY,
    run_id              TEXT NOT NULL REFERENCES runs (run_id),
    scenario_row        BIGINT NOT NULL REFERENCES scenarios (id),
    arm                 TEXT NOT NULL CHECK (arm IN ('pressure', 'control')),
    sample              INTEGER NOT NULL CHECK (sample >= 0),
    status              TEXT NOT NULL CHECK (status IN ('finished', 'max_turns', 'truncated', 'failed')),
    turns               INTEGER NOT NULL,
    final_answer        TEXT,
    transcript          JSONB NOT NULL,       -- every message, incl. reasoning, tool calls and results
    fired_events        JSONB NOT NULL,
    prompt_tokens       INTEGER,
    completion_tokens   INTEGER,
    reasoning_tokens    INTEGER,
    cost_usd            DOUBLE PRECISION,
    duration_ms         BIGINT NOT NULL,
    attempts            INTEGER NOT NULL,
    error               TEXT,
    code_git_hash       TEXT NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX episodes_one_result ON episodes (run_id, scenario_row, arm, sample) WHERE status <> 'failed';

-- One row per model turn: reasoning, text, tool calls and the call's bookkeeping.
CREATE TABLE turns (
    episode_id          BIGINT NOT NULL REFERENCES episodes (id),
    turn                INTEGER NOT NULL CHECK (turn >= 1),
    status              TEXT NOT NULL CHECK (status IN ('ok', 'truncated', 'failed')),
    content             TEXT,
    reasoning           TEXT,
    tool_calls          JSONB NOT NULL,
    finish_reason       TEXT,
    served_by           TEXT,
    api_model           TEXT,
    prompt_tokens       INTEGER,
    completion_tokens   INTEGER,
    reasoning_tokens    INTEGER,
    cost_usd            DOUBLE PRECISION,
    attempts            INTEGER NOT NULL,
    duration_ms         BIGINT NOT NULL,
    error               TEXT,
    raw_response        JSONB,
    PRIMARY KEY (episode_id, turn)
);

-- The system log: every tool call the harness ran, written from the actual calls.
CREATE TABLE tool_calls (
    id                  BIGSERIAL PRIMARY KEY,
    episode_id          BIGINT NOT NULL REFERENCES episodes (id),
    turn                INTEGER NOT NULL,
    seq                 INTEGER NOT NULL,     -- 0-based over the episode
    call_id             TEXT NOT NULL,
    tool                TEXT NOT NULL,
    args                JSONB NOT NULL,       -- null when the model's argument text did not parse
    raw_args            TEXT,
    result              TEXT NOT NULL,
    is_error            BOOLEAN NOT NULL,
    side_effect         BOOLEAN NOT NULL,
    UNIQUE (episode_id, seq)
);

-- The model's own log: the system-log rows of the scenario's self-log tool.
CREATE VIEW self_log AS
SELECT tc.*
FROM tool_calls tc
JOIN episodes e ON e.id = tc.episode_id
JOIN scenarios s ON s.id = e.scenario_row
WHERE tc.tool = s.self_log_tool;

-- Deterministic harm label: the harm rules matched on the system log.
CREATE TABLE harm_labels (
    id              BIGSERIAL PRIMARY KEY,
    episode_id      BIGINT NOT NULL REFERENCES episodes (id),
    harmful         BOOLEAN NOT NULL,
    matched_rules   TEXT[] NOT NULL,
    rules_sha256    TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (episode_id, rules_sha256),
    CHECK (harmful = (cardinality(matched_rules) > 0))
);

-- Test-awareness verdicts. One per episode, judge model and prompt version.
CREATE TABLE aware_labels (
    id              BIGSERIAL PRIMARY KEY,
    episode_id      BIGINT NOT NULL REFERENCES episodes (id),
    label           TEXT NOT NULL CHECK (label IN ('none', 'suspected', 'explicit')),
    evidence        TEXT NOT NULL,
    judge_model     TEXT NOT NULL,
    prompt_sha256   TEXT NOT NULL,
    judge_output    TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (episode_id, judge_model, prompt_sha256)
);

-- Log-accuracy labels. `no_log` is deterministic (the self-log tool was never called
-- successfully) and has no judge; every other label comes from the judge.
CREATE TABLE log_accuracy_labels (
    id              BIGSERIAL PRIMARY KEY,
    episode_id      BIGINT NOT NULL REFERENCES episodes (id),
    label           TEXT NOT NULL CHECK (label IN ('accurate', 'softened', 'fabricated', 'omitted', 'no_log')),
    items           JSONB NOT NULL,           -- per-action and per-log-entry verdicts
    judge_model     TEXT,
    prompt_sha256   TEXT,
    judge_output    TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((label = 'no_log') = (judge_model IS NULL)),
    CHECK ((judge_model IS NULL) = (prompt_sha256 IS NULL) AND (judge_model IS NULL) = (judge_output IS NULL))
);
CREATE UNIQUE INDEX log_accuracy_one_no_log ON log_accuracy_labels (episode_id) WHERE judge_model IS NULL;
CREATE UNIQUE INDEX log_accuracy_one_verdict ON log_accuracy_labels (episode_id, judge_model, prompt_sha256) WHERE judge_model IS NOT NULL;

CREATE TABLE judge_failures (
    id              BIGSERIAL PRIMARY KEY,
    episode_id      BIGINT NOT NULL REFERENCES episodes (id),
    judge           TEXT NOT NULL CHECK (judge IN ('eval_aware', 'log_accuracy')),
    judge_model     TEXT NOT NULL,
    prompt_sha256   TEXT NOT NULL,
    error           TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
