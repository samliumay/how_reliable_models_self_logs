# Every step of the pipeline; see README.md.
CRATE  := how_reliable_models_self_logs
HRSL   := cd $(CRATE) && cargo run --release --quiet --
MODEL  ?= qwen/qwen3.8-27b
PHASE  ?= pilot

.PHONY: db db-down validate data run label judge eval report baseline smoke check

db:        ## start PostgreSQL and apply migrations
	docker compose up -d --wait
	$(HRSL) migrate

db-down:   ## stop PostgreSQL (data is kept in the volume)
	docker compose down

validate:  ## parse and check every scenario in ../benchmark (no database)
	$(HRSL) validate

data:      ## validate and load the scenarios from ../benchmark into the database
	$(HRSL) load

run:       ## run one model: make run MODEL=... PHASE=smoke|pilot|study [ARGS="--samples 1 --scenario ID --allow-dirty"]
	$(HRSL) run --model $(MODEL) --phase $(PHASE) $(ARGS)

smoke:     ## dry run with canned replies (no API): make smoke SCRIPT=path/to/script.json [ARGS=...]
	$(HRSL) run --model scripted:$(abspath $(SCRIPT)) --phase smoke --samples 1 $(ARGS)

label:     ## deterministic labels (harm) for every unlabelled episode
	$(HRSL) label

judge:     ## test awareness and log accuracy of every unjudged episode (Gemini)
	$(HRSL) judge-aware
	$(HRSL) judge-log-accuracy

eval: label judge

report:    ## print tables and write ../results/<run_id>/
	$(HRSL) report

baseline:  ## trivial log-accuracy baselines (not implemented yet)
	@echo "baseline: not implemented yet (planned: e.g. 'log mentions every side-effect tool name' vs. the judge)"; exit 1

check:     ## format, lint, test, doc lints
	cd $(CRATE) && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --quiet
