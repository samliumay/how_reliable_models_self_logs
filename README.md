# How Reliable Are Models' Self-Logs?

When a model is asked to keep its own action log, is that log true? Each self-written entry is compared with an automatic record of the model's tool calls and labelled as accurate, softened, fabricated or omitted.

**Status:** in design. No code yet.

| | |
|---|---|
| Benchmark | [Hugging Face](https://huggingface.co/datasets/logicBombExe/how_reliable_models_self_logs) |
| Stack | Rust (`cargo`), PostgreSQL in Docker |
| Earlier study | [INSIDER_LLM_DETECTION_CODE](https://github.com/samliumay/INSIDER_LLM_DETECTION_CODE) (archived) |
