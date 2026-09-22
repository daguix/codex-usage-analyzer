# Codex Usage Analyzer

A small, database-free Rust CLI that reads Codex `rollout-*.jsonl` files and
reports token usage and estimated costs.

## Build

```bash
cargo build --release
```

The binary is written to `target/release/codex-usage-analyzer`.

## Usage

```bash
# Today's usage
codex-usage-analyzer --today

# Last seven days, broken down by model
codex-usage-analyzer --last 7d --by model

# JSON for all available rollouts
codex-usage-analyzer report --last total --format json

# Latest captured usage snapshot
codex-usage-analyzer status

# Estimated composition of input and cached-input context over seven days
codex-usage-analyzer breakdown --since 7d

# All available rollouts
codex-usage-analyzer breakdown
```

The default rollout directory is `~/.codex/sessions`. Override it with
`--rollouts PATH` or `CODEX_USAGE_ROLLOUTS`.

Supported report options include:

- `--today`, `--last`, `--from`, and `--to`
- `--group day|week|month`
- `--by model|directory|session`
- `--format table|json|csv`
- `--timezone IANA_NAME`
- `--output PATH`

`breakdown` reads context items but does not store them. It allocates the exact
reported input, cached-input, output, and reasoning-output totals across
categories using the recorded context order. Tokenization, encrypted
compaction summaries, model-injected tool schemas, and protocol overhead make
the category split an estimate.
`reasoning_output_tokens` is shown separately; depending on the rollout schema,
it may be a subset of `output_tokens` rather than an additional token count.
Without `--since`, every available rollout is analyzed.
Use `--format table|json|csv` and `--output PATH` as with reports. Code-looking
output from file-reading/search commands is classified as repository source.
Other tool output is split into build/test/lint, search/listings, version
control, patches/edits, web/external data, UI/media, process control,
data/analysis, system/environment, diagnostics, generic shell, and
uncategorized output.
