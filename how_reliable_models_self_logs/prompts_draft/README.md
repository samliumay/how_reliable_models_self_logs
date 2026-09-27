# Draft judge prompts

Drafts written with the code (2026-09-27). They belong in the benchmark repo, which the code
reads them from: move `labels/eval_aware/` and `labels/log_accuracy/` to
`../../benchmark/labels/`, review them there, and delete this folder. The prompt hash stored
with every verdict is the sha256 of `system.md` + `user.md`, so any edit is a new prompt
version. The unit tests only check that the placeholders the code fills are present.
